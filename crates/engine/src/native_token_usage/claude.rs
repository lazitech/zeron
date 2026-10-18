use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

use wake_core::models::SessionFileRef;

/// Extract one dated usage record for every Claude assistant API response.
///
/// Claude writes usage on the nested message object while the authoritative
/// event time is the outer JSONL row's timestamp. Sidechain rows belong to
/// the separate subagent transcript and are excluded here, matching Wake's
/// main-session summary semantics.
///
/// Claude Code splits one API response into several assistant rows (thinking,
/// text, and each tool use), and every row repeats the response's `usage`.
/// Rows are therefore collapsed by `(message.id, requestId)` so a response is
/// counted once; rows without a message id remain separate.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut events = Vec::<super::TokenUsageEvent>::new();
    let mut positions = HashMap::<(String, String), usize>::new();

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
        if timestamp_ms <= 0 {
            continue;
        }
        let event = super::TokenUsageEvent {
            timestamp_ms,
            tokens,
        };

        let Some(message_id) = row
            .pointer("/message/id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            events.push(event);
            continue;
        };
        let request_id = row
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let key = (message_id.to_owned(), request_id.to_owned());
        if let Some(&position) = positions.get(&key) {
            // Split rows repeat the same usage; a streamed row may carry a
            // partial count. Keep the largest report at the response's first
            // timestamp.
            let previous = &mut events[position];
            previous.tokens = previous.tokens.max(event.tokens);
        } else {
            positions.insert(key, events.len());
            events.push(event);
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

#[cfg(test)]
mod tests {
    use super::*;
    use wake_core::models::AgentId;

    fn file_ref(path: &std::path::Path) -> SessionFileRef {
        SessionFileRef {
            agent: AgentId::ClaudeCode,
            native_id: "session".into(),
            file_path: path.to_string_lossy().into_owned(),
            mtime_ms: 0,
            size: 0,
        }
    }

    fn assistant(message_id: Option<&str>, request_id: &str, at: &str, output: u64) -> String {
        let mut message = serde_json::json!({
            "usage": {
                "input_tokens": 10,
                "output_tokens": output,
                "cache_creation_input_tokens": 100,
                "cache_read_input_tokens": 1000
            }
        });
        if let Some(id) = message_id {
            message["id"] = id.into();
        }
        serde_json::json!({
            "type": "assistant",
            "timestamp": at,
            "requestId": request_id,
            "message": message
        })
        .to_string()
    }

    #[test]
    fn split_rows_of_one_response_count_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let lines = [
            // One response written as thinking, text, and tool-use rows.
            assistant(Some("msg_1"), "req_1", "2026-10-01T10:00:00Z", 5),
            assistant(Some("msg_1"), "req_1", "2026-10-01T10:00:01Z", 5),
            assistant(Some("msg_1"), "req_1", "2026-10-01T10:00:02Z", 5),
            // A second response in the same turn.
            assistant(Some("msg_2"), "req_2", "2026-10-01T10:01:00Z", 7),
            // Sidechain rows belong to the subagent transcript.
            serde_json::json!({
                "type": "assistant",
                "isSidechain": true,
                "timestamp": "2026-10-01T10:02:00Z",
                "message": {"id": "msg_3", "usage": {"output_tokens": 99}}
            })
            .to_string(),
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();

        let events = extract(&file_ref(&path)).unwrap();
        assert_eq!(
            events,
            vec![
                super::super::TokenUsageEvent {
                    timestamp_ms: 1_790_848_800_000,
                    tokens: 1115,
                },
                super::super::TokenUsageEvent {
                    timestamp_ms: 1_790_848_860_000,
                    tokens: 1117,
                },
            ]
        );
    }

    #[test]
    fn partial_stream_rows_keep_the_largest_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let lines = [
            assistant(Some("msg_1"), "req_1", "2026-10-01T10:00:00Z", 1),
            assistant(Some("msg_1"), "req_1", "2026-10-01T10:00:03Z", 40),
            // Rows without a message id cannot be matched and stay separate.
            assistant(None, "req_2", "2026-10-01T10:01:00Z", 2),
            assistant(None, "req_2", "2026-10-01T10:01:00Z", 2),
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();

        let events = extract(&file_ref(&path)).unwrap();
        let tokens: Vec<_> = events.iter().map(|event| event.tokens).collect();
        assert_eq!(tokens, vec![1150, 1112, 1112]);
        assert_eq!(events[0].timestamp_ms, 1_790_848_800_000);
    }
}
