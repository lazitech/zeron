use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::Result;
use serde_json::Value;
use wake_core::models::SessionFileRef;

/// Extract usage from the active Qoder message branch.
///
/// Qoder stores edits as a message tree and may write several assistant
/// fragments with one logical `message.id`.  Follow the same active-leaf and
/// fragment recovery rules as Wake, then count each logical message once.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let file = File::open(&file_ref.file_path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut nodes = HashMap::<String, MessageNode>::new();
    let mut order = Vec::new();
    let mut active_leaf = ActiveLeaf::Missing;

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
        match typ {
            "active-leaf" => match row.get("leafUuid") {
                Some(Value::Null) => active_leaf = ActiveLeaf::Empty,
                Some(Value::String(id)) => active_leaf = ActiveLeaf::Uuid(id.clone()),
                _ => {}
            },
            "user" | "assistant" | "system" | "attachment" => {
                if row.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let Some(uuid) = row.get("uuid").and_then(nonempty_string) else {
                    continue;
                };
                let parent = row
                    .get("logicalParentUuid")
                    .or_else(|| row.get("parentUuid"))
                    .and_then(nonempty_string);
                if !nodes.contains_key(&uuid) {
                    order.push(uuid.clone());
                }
                nodes.insert(uuid, MessageNode { row, parent });
            }
            _ => {}
        }
    }

    let chain = active_chain(&nodes, &order, &active_leaf);
    let mut seen_messages = HashSet::<String>::new();
    let mut events = Vec::new();
    for row in chain {
        if row.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = row.get("message") else {
            continue;
        };
        let Some(tokens) = message.get("usage").map(usage_tokens).filter(|n| *n > 0) else {
            continue;
        };
        let Some(timestamp_ms) = row.get("timestamp").map(to_epoch_ms).filter(|n| *n > 0) else {
            continue;
        };
        let key = message
            .get("id")
            .and_then(nonempty_string)
            .or_else(|| row.get("uuid").and_then(nonempty_string))
            .unwrap_or_else(|| serde_json::to_string(&row).unwrap_or_default());
        if seen_messages.insert(key) {
            events.push(super::TokenUsageEvent {
                timestamp_ms,
                tokens,
            });
        }
    }
    Ok(events)
}

#[derive(Clone)]
struct MessageNode {
    row: Value,
    parent: Option<String>,
}

enum ActiveLeaf {
    Missing,
    Empty,
    Uuid(String),
}

fn active_chain(
    nodes: &HashMap<String, MessageNode>,
    order: &[String],
    active_leaf: &ActiveLeaf,
) -> Vec<Value> {
    let mut cursor = match active_leaf {
        ActiveLeaf::Empty => return Vec::new(),
        ActiveLeaf::Uuid(id) if nodes.contains_key(id) => id.clone(),
        ActiveLeaf::Missing | ActiveLeaf::Uuid(_) => match order.last() {
            Some(id) => id.clone(),
            None => return Vec::new(),
        },
    };

    let mut seen = HashSet::new();
    let mut chain_ids = Vec::new();
    loop {
        if !seen.insert(cursor.clone()) {
            break;
        }
        let Some(node) = nodes.get(&cursor) else {
            break;
        };
        chain_ids.push(cursor.clone());
        let Some(parent) = node.parent.as_ref() else {
            break;
        };
        cursor = parent.clone();
    }
    chain_ids.reverse();

    // An API response can be split into assistant siblings that share one
    // message id.  Recover all such fragments and their tool results when an
    // active fragment is on the selected branch, matching Wake's transcript
    // parser's branch semantics.
    let mut fragments_by_message = HashMap::<String, Vec<String>>::new();
    let mut message_by_fragment = HashMap::<String, String>::new();
    for id in order {
        let Some(node) = nodes.get(id) else {
            continue;
        };
        if node.row.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message_id) = node.row.pointer("/message/id").and_then(nonempty_string) else {
            continue;
        };
        fragments_by_message
            .entry(message_id.clone())
            .or_default()
            .push(id.clone());
        message_by_fragment.insert(id.clone(), message_id);
    }

    let mut results_by_message = HashMap::<String, Vec<String>>::new();
    for id in order {
        let Some(node) = nodes.get(id) else {
            continue;
        };
        let is_tool_result = node.row.get("type").and_then(Value::as_str) == Some("user")
            && node
                .row
                .pointer("/message/content")
                .and_then(Value::as_array)
                .is_some_and(|blocks| {
                    blocks.iter().any(|block| {
                        block.get("type").and_then(Value::as_str) == Some("tool_result")
                    })
                });
        if !is_tool_result {
            continue;
        }
        let Some(message_id) = node
            .row
            .get("parentUuid")
            .and_then(nonempty_string)
            .and_then(|parent| message_by_fragment.get(&parent).cloned())
        else {
            continue;
        };
        results_by_message
            .entry(message_id)
            .or_default()
            .push(id.clone());
    }

    let mut output = Vec::new();
    let mut emitted = HashSet::new();
    for id in chain_ids {
        if emitted.contains(&id) {
            continue;
        }
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        let Some(message_id) = message_by_fragment.get(&id) else {
            emitted.insert(id);
            output.push(node.row.clone());
            continue;
        };
        if let Some(fragment_ids) = fragments_by_message.get(message_id) {
            for fragment_id in fragment_ids {
                if emitted.insert(fragment_id.clone()) {
                    if let Some(fragment) = nodes.get(fragment_id) {
                        output.push(fragment.row.clone());
                    }
                }
            }
        }
        if let Some(result_ids) = results_by_message.get(message_id) {
            for result_id in result_ids {
                if emitted.insert(result_id.clone()) {
                    if let Some(result) = nodes.get(result_id) {
                        output.push(result.row.clone());
                    }
                }
            }
        }
    }
    output
}

fn nonempty_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
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
