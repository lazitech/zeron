use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::PathBuf;

use wake_core::adapters::session_source_path;
use wake_core::models::SessionFileRef;

/// Extract ZCode's per-assistant-call token records from its SQLite message
/// table. The session table has no token total; each assistant message.data
/// row carries tokens.total and its own time.created.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let database = PathBuf::from(session_source_path(&file_ref.file_path));
    let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open ZCode database {}", database.display()))?;
    let mut statement = connection.prepare(
        "SELECT data, time_created
         FROM message
         WHERE session_id = ?1
         ORDER BY time_created, id",
    )?;
    let rows = statement.query_map([&file_ref.native_id], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<i64>>(1)?,
        ))
    })?;

    let mut events = Vec::new();
    for row in rows {
        let (Some(raw), fallback_timestamp) = row? else {
            continue;
        };
        let Ok(message) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(tokens) = message
            .pointer("/tokens/total")
            .and_then(nonnegative_tokens)
        else {
            continue;
        };
        let timestamp_ms = message
            .pointer("/time/created")
            .map(timestamp_value)
            .filter(|timestamp| *timestamp > 0)
            .or_else(|| fallback_timestamp.map(normalize_unix_timestamp))
            .unwrap_or(0);
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

fn timestamp_value(value: &Value) -> i64 {
    match value {
        Value::String(value) => chrono::DateTime::parse_from_rfc3339(value)
            .map(|date| date.timestamp_millis())
            .unwrap_or(0),
        Value::Number(value) => value.as_i64().map(normalize_unix_timestamp).unwrap_or(0),
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
