//! Machine-local coding-agent usage statistics returned by the engine RPC.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageStatistics {
    /// The snapshot includes locally indexed agent histories and Zeron journals.
    pub scope: String,
    /// Snapshot creation time in epoch milliseconds.
    pub generated_at_ms: i64,
    pub totals: UsageTotals,
    /// Latest seven local calendar dates, oldest first.
    pub recent_days: Vec<UsageDay>,
    /// Seven local calendar dates immediately before `recent_days`.
    pub previous_days: Vec<UsageDay>,
    /// 26 Monday-aligned calendar weeks (182 dates), including empty future
    /// dates in the current week. Heatmap color is prompt-only, matching
    /// Wake's Insights semantics; dated token usage is included for tooltips.
    pub heatmap: Vec<UsageDay>,
    /// Per-agent prompt counts for the same 26 calendar weeks as the heatmap.
    pub agent_trend: Vec<UsageTrendWeek>,
    /// All-time prompt distribution by local hour, weekday, and month.
    pub distribution: UsageDistribution,
    /// Rankings are complete; the UI sorts and limits each board by its metric.
    pub agents: Vec<UsageRank>,
    pub projects: Vec<UsageRank>,
    pub models: Vec<UsageRank>,
    pub coverage: UsageCoverage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageTotals {
    pub sessions: u64,
    pub tokens: u64,
    /// False means none of the indexed sessions reported token data.
    pub has_token_data: bool,
    /// Exact token usage attributed to the current local calendar date.
    /// None means the available sources have no timestamped usage data.
    pub today_tokens: Option<u64>,
    /// Exact token usage attributed to the current Monday-start local week.
    pub week_tokens: Option<u64>,
    pub prompts: u64,
    pub agents: u64,
    pub projects: u64,
    pub active_days: u64,
    pub current_streak_days: u64,
    pub longest_streak_days: u64,
    pub busiest_day: Option<String>,
    pub busiest_day_prompts: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageDay {
    /// Local date formatted as `YYYY-MM-DD`.
    pub day: String,
    /// Sessions are grouped by their creation date.
    pub sessions: u64,
    /// Main-line user messages grouped by their own timestamp.
    pub prompts: u64,
    /// Token usage attributable to this local date. None means the available
    /// sources have no date-specific token data for this day.
    pub tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageTrendWeek {
    /// Monday date formatted as `YYYY-MM-DD`.
    pub week_start: String,
    pub agents: Vec<UsageAgentPrompt>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageAgentPrompt {
    pub agent_id: String,
    pub agent_label: String,
    pub prompts: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageDistribution {
    /// Labels are local hour numbers `00` through `23`.
    pub hours: Vec<UsageBucket>,
    /// Monday-first localized labels.
    pub weekdays: Vec<UsageBucket>,
    /// January-first month labels.
    pub months: Vec<UsageBucket>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageBucket {
    pub label: String,
    pub value: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageRank {
    pub id: String,
    pub label: String,
    pub sessions: u64,
    pub tokens: u64,
    pub has_token_data: bool,
    pub prompts: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageCoverage {
    /// At least one configured source could not be read completely.
    pub partial: bool,
    /// Number of local agent families with at least one discovered session.
    pub indexed_sources: u64,
    pub indexed_sessions: u64,
    pub unreadable_files: u64,
    /// Native adapters that could not be enumerated, plus the local index if
    /// it could not be opened or refreshed.
    pub unavailable_sources: u64,
    /// Unsupported or malformed lines retained as partial session coverage.
    pub malformed_lines: u64,
    /// Some supported agents do not record token counts in their local history.
    pub agents_without_token_data: u64,
    /// Most recent successful or partial local index refresh.
    pub last_scanned_at_ms: i64,
}
