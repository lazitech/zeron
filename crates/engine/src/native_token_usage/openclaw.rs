use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use wake_core::models::SessionFileRef;

/// Extract dated usage from an OpenClaw transcript.
///
/// OpenClaw has two storage generations. Legacy sessions are JSONL files, and
/// current sessions are represented by a virtual path (`db#session_id`) into
/// `transcript_events`. In both cases the usage belongs to an assistant
/// message and the message/entry timestamp is the event's date. The
/// `sessions.json`/`session_nodes.entry_json` `totalTokens` values are lifetime
/// snapshots, so they are deliberately not converted into dated events.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let path = Path::new(&file_ref.file_path);
    if path.is_file() {
        return extract_jsonl(path);
    }

    // SQLite-backed SessionFileRefs use Wake's `<db>#<session_id>` convention.
    // `session_source_path` leaves a real file unchanged and strips the virtual
    // suffix only when the path itself does not exist.
    let source = PathBuf::from(wake_core::adapters::session_source_path(
        &file_ref.file_path,
    ));
    if !source.is_file() {
        // A stale/deleted source has no dated records to contribute. In
        // particular, do not fall back to mtime or an index total here.
        return Ok(Vec::new());
    }

    extract_sqlite(&source, &file_ref.native_id)
}

#[derive(Debug)]
struct Entry {
    /// The SQLite sequence is needed to map active-branch rows back to their
    /// JSON event. Legacy JSONL entries have no separate sequence value.
    seq: Option<i64>,
    value: Value,
    /// SQLite's envelope timestamp is only a fallback. The transcript JSON's
    /// own timestamp remains authoritative whenever it is present.
    created_at_ms: i64,
}

fn extract_jsonl(path: &Path) -> Result<Vec<super::TokenUsageEvent>> {
    let file =
        File::open(path).with_context(|| format!("open OpenClaw transcript {}", path.display()))?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line.with_context(|| format!("read OpenClaw transcript {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        // Match Wake's transcript parser: a malformed/torn JSONL line is
        // ignored so a partial tail does not erase valid earlier usage.
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            entries.push(Entry {
                seq: None,
                value,
                created_at_ms: 0,
            });
        }
    }

    Ok(usage_events(&entries, None))
}

fn extract_sqlite(db: &Path, native_id: &str) -> Result<Vec<super::TokenUsageEvent>> {
    let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open OpenClaw database {}", db.display()))?;

    let mut statement = match connection.prepare(
        "SELECT seq, event_json, created_at
         FROM transcript_events
         WHERE session_id = ?1
         ORDER BY seq",
    ) {
        Ok(statement) => statement,
        // A database containing only session/index metadata has no dated
        // transcript records. Its cumulative total is intentionally unusable
        // for day/week attribution.
        Err(error) if is_missing_table(&error, "transcript_events") => return Ok(Vec::new()),
        Err(error) => return Err(error).context("prepare OpenClaw transcript query"),
    };

    let rows = statement.query_map([native_id], |row| {
        let seq = row.get::<_, i64>(0)?;
        let event_json: String = row.get(1)?;
        let created_at = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
        Ok((seq, event_json, created_at))
    })?;

    let mut entries = Vec::new();
    for row in rows {
        let (seq, event_json, created_at) = row.context("read OpenClaw transcript row")?;
        let Ok(value) = serde_json::from_str::<Value>(&event_json) else {
            // Keep the same partial-log behavior as JSONL: malformed event
            // rows do not make valid earlier usage disappear.
            continue;
        };
        entries.push(Entry {
            seq: Some(seq),
            value,
            created_at_ms: epoch_ms(created_at),
        });
    }

    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // The active-events table is the canonical branch selection for current
    // OpenClaw stores. If it is absent or empty, use the adapter's fallback:
    // follow parentId from the last id-bearing entry.
    let active = active_indices(&connection, native_id, &entries)?;
    Ok(usage_events(&entries, active.as_deref()))
}

fn active_indices(
    connection: &Connection,
    native_id: &str,
    entries: &[Entry],
) -> Result<Option<Vec<usize>>> {
    let mut statement = match connection.prepare(
        "SELECT event_seq
         FROM session_transcript_active_events
         WHERE session_id = ?1
         ORDER BY active_position",
    ) {
        Ok(statement) => statement,
        Err(error) if is_missing_table(&error, "session_transcript_active_events") => {
            return Ok(None);
        }
        Err(error) => return Err(error).context("prepare OpenClaw active transcript query"),
    };

    let sequences = statement
        .query_map([native_id], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if sequences.is_empty() {
        return Ok(None);
    }

    let by_sequence = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.seq.map(|seq| (seq, index)))
        .collect::<HashMap<_, _>>();
    Ok(Some(
        sequences
            .into_iter()
            .filter_map(|sequence| by_sequence.get(&sequence).copied())
            .collect(),
    ))
}

fn usage_events(entries: &[Entry], active: Option<&[usize]>) -> Vec<super::TokenUsageEvent> {
    let selected = selected_indices(entries, active);
    let mut events = Vec::new();

    for index in selected {
        let entry = &entries[index];
        if entry.value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let Some(tokens) = usage_tokens(usage) else {
            continue;
        };

        // Prefer the transcript entry's ISO timestamp, then the nested
        // pi-ai message timestamp, and finally SQLite's event timestamp.
        let timestamp_ms = timestamp_ms(entry.value.get("timestamp"))
            .or_else(|| timestamp_ms(message.get("timestamp")))
            .or_else(|| positive_timestamp(entry.created_at_ms));
        let Some(timestamp_ms) = timestamp_ms else {
            // A usage total without a timestamp cannot be assigned to a day or
            // week without inventing a date, so omit it from this event index.
            continue;
        };

        events.push(super::TokenUsageEvent {
            timestamp_ms,
            tokens,
        });
    }

    events
}

fn selected_indices(entries: &[Entry], active: Option<&[usize]>) -> Vec<usize> {
    if let Some(active) = active {
        return active.to_vec();
    }

    let by_id = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .value
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, index))
        })
        .collect::<HashMap<_, _>>();
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = entries
        .iter()
        .rposition(|entry| entry.value.get("id").and_then(Value::as_str).is_some());

    while let Some(index) = cursor {
        if !seen.insert(index) {
            break;
        }
        chain.push(index);
        cursor = entries[index]
            .value
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|parent| by_id.get(parent).copied());
    }
    chain.reverse();
    chain
}

fn usage_tokens(usage: &Value) -> Option<u64> {
    if let Some(total) = usage.get("totalTokens") {
        // Wake treats totalTokens as the authoritative per-call total and
        // accepts zero, while rejecting malformed/negative values.
        return total
            .as_i64()
            .filter(|total| *total >= 0)
            .map(|total| total as u64);
    }

    // Some pi-ai compatible providers omit totalTokens but persist the
    // components. Keep the fallback conservative and only emit it when at
    // least one component is present.
    let mut present = false;
    let total = ["input", "output", "cacheRead", "cacheWrite"]
        .into_iter()
        .map(|key| {
            let value = usage.get(key).and_then(Value::as_u64);
            present |= value.is_some();
            value.unwrap_or(0)
        })
        .fold(0u64, u64::saturating_add);
    present.then_some(total)
}

fn timestamp_ms(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(value) => DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|timestamp| timestamp.timestamp_millis())
            .filter(|timestamp| *timestamp > 0),
        Value::Number(value) => value
            .as_f64()
            .map(|number| {
                if number > 1e12 {
                    number as i64
                } else if number > 0.0 {
                    (number * 1000.0) as i64
                } else {
                    0
                }
            })
            .filter(|timestamp| *timestamp > 0),
        _ => None,
    }
}

fn positive_timestamp(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

fn epoch_ms(value: i64) -> i64 {
    if value <= 0 {
        0
    } else if value > 1_000_000_000_000 {
        value
    } else {
        value.saturating_mul(1000)
    }
}

fn is_missing_table(error: &rusqlite::Error, table: &str) -> bool {
    error
        .to_string()
        .contains(&format!("no such table: {table}"))
}
