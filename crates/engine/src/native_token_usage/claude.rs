use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};

use wake_core::models::SessionFileRef;

/// Extract one dated usage record for every Claude assistant API response.
///
/// Claude writes usage on the nested message object while the authoritative
/// event time is the outer JSONL row's timestamp. Sidechain rows belong to
/// the separate subagent transcript and are excluded here, matching Wake's
/// main-session summary semantics.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut events = Vec::new();

    for line in reader.lines() {
        let Ok(line) = line else {
            // A concurrently appended JSONL tail can be incomplete. Preserve
            // the valid prefix just as Wake's transcript parser does.
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if row.get("type").and_then(Value::as_str) != Some("assistant")
            || row.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }

        let Some(usage) = row.pointer("/message/usage") else {
            continue;
        };
        let relevant = [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ];
        if !relevant.iter().any(|key| usage.get(*key).is_some()) {
            continue;
        }

        let tokens = relevant
            .iter()
            .filter_map(|key| usage.get(*key).and_then(nonnegative_tokens))
            .fold(0u64, u64::saturating_add);
        let timestamp_ms = timestamp_ms(row.get("timestamp"));
        if timestamp_ms > 0 {
            events.push(super::TokenUsageEvent {
                timestamp_ms,
                tokens,
            });
        }
    }

    Ok(events)
}

fn nonnegative_tokens(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
}

fn timestamp_ms(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::String(value)) => chrono::DateTime::parse_from_rfc3339(value)
            .map(|date| date.timestamp_millis())
            .unwrap_or(0),
        Some(Value::Number(value)) => value.as_i64().map(normalize_unix_timestamp).unwrap_or(0),
        _ => 0,
    }
}

fn normalize_unix_timestamp(value: i64) -> i64 {
    if value <= 0 {
        0
    } else if value > 1_000_000_000_000 {
        value
    } else {
        value.saturating_mul(1_000)
    }
}
