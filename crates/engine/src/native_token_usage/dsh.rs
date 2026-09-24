use anyhow::Result;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use wake_core::models::SessionFileRef;

/// Extract DeepSeek Harness model-call usage from plain or zstd JSONL.
///
/// DSH's surfaceOp replace rows replace an earlier visible surface range
/// during compaction. Wake ignores those rows, so their usage must not be
/// counted a second time. Usage-only assistant rows are retained.
pub(super) fn extract(file_ref: &SessionFileRef) -> Result<Vec<super::TokenUsageEvent>> {
    let reader = open_log(Path::new(&file_ref.file_path))?;
    let mut events = Vec::new();

    for line in reader.lines() {
        let Ok(line) = line else {
            // The writer appends independent zstd frames. A torn final frame
            // should not discard the valid prefix already read.
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if row.pointer("/surfaceOp/op").and_then(Value::as_str) == Some("replace")
            || row.get("type").and_then(Value::as_str) != Some("assistant/message")
        {
            continue;
        }

        let Some(usage) = row.get("data").and_then(|data| data.get("usage")) else {
            continue;
        };
        let relevant = [
            "inputTokens",
            "outputTokens",
            "cacheReadTokens",
            "cacheWriteTokens",
        ];
        if !relevant.iter().any(|key| usage.get(*key).is_some()) {
            continue;
        }
        let tokens = relevant
            .iter()
            .filter_map(|key| usage.get(*key).and_then(nonnegative_tokens))
            .fold(0u64, u64::saturating_add);
        let timestamp_ms = row.get("time").and_then(Value::as_i64).unwrap_or(0);
        if timestamp_ms > 0 {
            events.push(super::TokenUsageEvent {
                timestamp_ms,
                tokens,
            });
        }
    }

    Ok(events)
}

fn open_log(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    if path
        .extension()
        .is_some_and(|extension| extension == "zstd")
    {
        let decoder =
            zstd::stream::read::Decoder::with_buffer(BufReader::with_capacity(1 << 20, file))?;
        Ok(Box::new(BufReader::with_capacity(1 << 20, decoder)))
    } else {
        Ok(Box::new(BufReader::with_capacity(1 << 20, file)))
    }
}

fn nonnegative_tokens(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
}
