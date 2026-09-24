use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};

use wake_core::models::SessionFileRef;

/// Extract Pi and Oh My Pi per-call usage records.
///
/// usage.totalTokens is the cost of one assistant API call, not a session
/// lifetime counter. Empty or thinking-only assistant calls still count, so
/// usage is inspected before looking at message content.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut events = Vec::new();

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
        if row.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = row.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(total_tokens) = message
            .pointer("/usage/totalTokens")
            .and_then(nonnegative_tokens)
        else {
            continue;
        };
        let timestamp_ms = timestamp_ms(row.get("timestamp"));
        if timestamp_ms > 0 {
            events.push(super::TokenUsageEvent {
                timestamp_ms,
                tokens: total_tokens,
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
