use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::Result;
use serde_json::Value;
use wake_core::models::SessionFileRef;

/// Extract per-response usage from CodeBuddy and WorkBuddy JSONL histories.
///
/// CodeBuddy can write the same stream record more than once while a response
/// is being persisted.  A record id is the stable identity when present; for
/// id-less records the serialized source row is used, so separate calls in
/// one user turn are not collapsed merely because they share a request id.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut candidates = Vec::<Candidate>::new();
    let mut positions = HashMap::<String, usize>::new();

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
        let typ = row.get("type").and_then(Value::as_str).unwrap_or_default();
        let is_assistant_record = match typ {
            "reasoning" | "function_call" => true,
            "message" => row.get("role").and_then(Value::as_str) == Some("assistant"),
            _ => false,
        };
        if !is_assistant_record {
            continue;
        }
        let tokens = row_usage(&row);
        if tokens == 0 {
            continue;
        }
        let timestamp_ms = row.get("timestamp").map(to_epoch_ms).unwrap_or(0);
        if timestamp_ms <= 0 {
            continue;
        }

        let key = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(|id| format!("id:{id}"))
            .unwrap_or_else(|| {
                // Keep request ids out of this fallback key.  The provider
                // reuses one conversationRequestId for every model call in a
                // tool-using user turn; collapsing by it loses later calls.
                format!(
                    "row:{}",
                    serde_json::to_string(&row).unwrap_or_else(|_| line.clone())
                )
            });
        let candidate = Candidate {
            timestamp_ms,
            tokens,
        };
        if let Some(&position) = positions.get(&key) {
            let previous = &mut candidates[position];
            // Stream updates may repeat an id with partial metadata.  Keep
            // the largest reported usage once and use the latest timestamp
            // for an equal/larger update.
            if candidate.tokens >= previous.tokens {
                previous.tokens = candidate.tokens;
                previous.timestamp_ms = previous.timestamp_ms.max(candidate.timestamp_ms);
            }
        } else {
            positions.insert(key, candidates.len());
            candidates.push(candidate);
        }
    }

    Ok(candidates
        .into_iter()
        .filter(|candidate| candidate.tokens > 0 && candidate.timestamp_ms > 0)
        .map(|candidate| super::TokenUsageEvent {
            timestamp_ms: candidate.timestamp_ms,
            tokens: candidate.tokens,
        })
        .collect())
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    timestamp_ms: i64,
    tokens: u64,
}

fn row_usage(row: &Value) -> u64 {
    row.pointer("/providerData/rawUsage")
        .or_else(|| row.pointer("/message/usage"))
        .map(usage_tokens)
        .unwrap_or(0)
}

fn usage_tokens(usage: &Value) -> u64 {
    if let Some(total) = usage.get("total_tokens").and_then(positive_u64) {
        return total;
    }
    let detailed = [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .into_iter()
    .map(|key| usage.get(key).and_then(positive_u64).unwrap_or(0))
    .sum::<u64>();
    if detailed > 0 {
        return detailed;
    }
    ["prompt_tokens", "completion_tokens"]
        .into_iter()
        .map(|key| usage.get(key).and_then(positive_u64).unwrap_or(0))
        .sum()
}

fn positive_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .filter(|value| *value > 0)
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
