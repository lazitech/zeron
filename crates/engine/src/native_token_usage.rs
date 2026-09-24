//! Event-level token usage extracted from native agent histories.
//!
//! Wake's session metadata only retains lifetime token totals. This module
//! extracts dated usage records from source formats that actually persist
//! them, so daily and weekly totals never infer dates from session totals.

mod claude;
mod codebuddy;
mod codex;
mod dsh;
mod openclaw;
mod pi;
mod qoder;
mod zcode;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use wake_core::models::SessionFileRef;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TokenUsageEvent {
    pub timestamp_ms: i64,
    pub tokens: u64,
}

pub(crate) fn extract(file_ref: &SessionFileRef) -> Result<Vec<TokenUsageEvent>> {
    let events = match file_ref.agent.as_str() {
        "claude-code" => claude::extract(file_ref)?,
        "codex" => codex::extract(file_ref)?,
        "codebuddy" | "workbuddy" => codebuddy::extract(file_ref)?,
        "dsh" => dsh::extract(file_ref)?,
        "openclaw" => openclaw::extract(file_ref)?,
        "pi" | "omp" => pi::extract(file_ref)?,
        "qoder" => qoder::extract(file_ref)?,
        "zcode" => zcode::extract(file_ref)?,
        _ => Vec::new(),
    };
    Ok(events
        .into_iter()
        .filter(|event| event.timestamp_ms > 0)
        .collect())
}
