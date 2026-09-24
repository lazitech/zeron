use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::Result;
use serde_json::Value;
use wake_core::models::SessionFileRef;

/// Extract dated usage deltas from Codex's cumulative `token_count` events.
///
/// A rollout can contain a copied history when it is forked.  The child keeps
/// the parent's cumulative counter, so snapshots before the child was created
/// are only used to establish the baseline.  A counter drop is treated as a
/// reset and establishes a new baseline instead of creating a large fake
/// usage event.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut snapshots = Vec::new();
    let mut forked = false;
    let mut fork_started_at = 0i64;
    let mut saw_session_meta = false;

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let timestamp_ms = row.get("timestamp").map(to_epoch_ms).unwrap_or(0);
        if row.get("type").and_then(Value::as_str) == Some("session_meta") {
            if !saw_session_meta {
                saw_session_meta = true;
                fork_started_at = timestamp_ms;
                forked = row
                    .pointer("/payload/forked_from_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty());
            }
            continue;
        }
        if row.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        if row.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(total) = row
            .pointer("/payload/info/total_token_usage/total_tokens")
            .and_then(nonnegative_u64)
        else {
            continue;
        };
        snapshots.push(Snapshot {
            timestamp_ms,
            total,
        });
    }

    let mut events = Vec::new();
    let mut previous: Option<u64> = None;
    let mut started = !forked;
    for snapshot in snapshots {
        // For a fork, copied parent events normally retain their old
        // timestamps.  Keep their latest total as the child's baseline and
        // start emitting only after the child session's own start time.
        if forked && !started {
            if snapshot.timestamp_ms > fork_started_at {
                started = true;
                if previous.is_none() {
                    previous = Some(snapshot.total);
                    continue;
                }
            }
        }

        let Some(previous_total) = previous else {
            previous = Some(snapshot.total);
            if started && snapshot.total > 0 && snapshot.timestamp_ms > 0 {
                events.push(super::TokenUsageEvent {
                    timestamp_ms: snapshot.timestamp_ms,
                    tokens: snapshot.total,
                });
            }
            continue;
        };

        if snapshot.total < previous_total {
            // Compaction, a fresh process counter, or a malformed/forked
            // segment reset the cumulative counter.  Do not count the new
            // absolute value as if it were a delta from the old segment.
            previous = Some(snapshot.total);
            continue;
        }

        let delta = snapshot.total - previous_total;
        previous = Some(snapshot.total);
        if started && delta > 0 && snapshot.timestamp_ms > 0 {
            events.push(super::TokenUsageEvent {
                timestamp_ms: snapshot.timestamp_ms,
                tokens: delta,
            });
        }
    }
    Ok(events)
}

#[derive(Debug, Clone, Copy)]
struct Snapshot {
    timestamp_ms: i64,
    total: u64,
}

fn nonnegative_u64(value: &Value) -> Option<u64> {
    value.as_i64().and_then(|value| u64::try_from(value).ok())
}

fn to_epoch_ms(value: &Value) -> i64 {
    match value {
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or(0.0);
            if number > 1e12 {
                number as i64
            } else if number > 0.0 {
                (number * 1000.0) as i64
            } else {
                0
            }
        }
        Value::String(value) => chrono::DateTime::parse_from_rfc3339(value)
            .map(|date| date.timestamp_millis())
            .unwrap_or(0),
        _ => 0,
    }
}
