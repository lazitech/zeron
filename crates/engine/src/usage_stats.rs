//! Machine-local coding-agent usage statistics.
//!
//! The engine reduces native agent histories and Zeron journals to compact
//! session summaries before building the dashboard snapshot. Transcript text
//! is never persisted by this module.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use zeron_proto::{
    AgentEvent, HarnessId, UsageAgentPrompt, UsageBucket, UsageCoverage, UsageDay,
    UsageDistribution, UsageRank, UsageStatistics, UsageTotals, UsageTrendWeek,
};

use crate::native_token_usage::TokenUsageEvent;
use crate::run_journal::JournalRecord;

const UNKNOWN_AGENT_ID: &str = "unknown";
const UNKNOWN_AGENT_LABEL: &str = "未知";
const TREND_WEEKS: usize = 26;

/// One session reduced to the fields needed by Insights. Native ids allow a
/// Zeron-managed run and its underlying CLI transcript to collapse to one row.
#[derive(Debug, Clone, Default)]
pub(crate) struct UsageSessionInput {
    /// Source-unique key; native rows include agent, id, and creation time.
    pub key: String,
    pub agent_id: String,
    pub agent_label: String,
    pub native_session_id: Option<String>,
    pub project_path: Option<String>,
    pub project_label: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: Option<i64>,
    /// None means the source does not report token usage.
    pub tokens: Option<u64>,
    /// Per-event usage with an authoritative source timestamp.
    pub token_usage_events: Vec<TokenUsageEvent>,
    /// Main-line user messages, including prompts with no usable timestamp.
    pub prompts: u64,
    pub prompt_timestamps_ms: Vec<i64>,
}

pub(crate) struct ChatUsageInput {
    pub chat: zeron_proto::Chat,
    /// None means the local chat document could not be read completely.
    pub prompt_timestamps: Option<Vec<i64>>,
    /// None means the local journal could not be read.
    pub records: Option<(Vec<JournalRecord>, u64)>,
}

#[derive(Default)]
pub(crate) struct UsageInputCoverage {
    pub unreadable_files: u64,
    pub malformed_lines: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentIdentity {
    id: String,
    label: String,
    model: Option<String>,
}

impl AgentIdentity {
    fn unknown() -> Self {
        Self {
            id: UNKNOWN_AGENT_ID.into(),
            label: UNKNOWN_AGENT_LABEL.into(),
            model: None,
        }
    }

    fn new(harness: HarnessId, model: &str) -> Self {
        let id = serde_json::to_value(harness)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{harness:?}").to_lowercase());
        let label = match harness {
            HarnessId::ClaudeCode => "Claude Code",
            HarnessId::Codex => "Codex",
            HarnessId::Cursor => "Cursor",
            HarnessId::Devin => "Devin",
            HarnessId::Grok => "Grok",
            HarnessId::Hermes => "Hermes",
            HarnessId::Pi => "Pi",
            HarnessId::Opencode => "OpenCode",
            HarnessId::Antigravity => "Antigravity",
            HarnessId::Mock => "Mock",
        };
        Self {
            id,
            label: label.into(),
            model: nonempty(model),
        }
    }
}

#[derive(Default)]
struct LaneUsage {
    tokens: u64,
    timestamp_ms: Option<i64>,
}

#[derive(Default)]
struct MutableTurn {
    usage_by_lane: HashMap<String, LaneUsage>,
    closed_lane_usages: u64,
}

struct Turn {
    usage_by_lane: HashMap<String, LaneUsage>,
}

#[derive(Default)]
struct DayAccumulator {
    sessions: u64,
    prompts: u64,
}

#[derive(Default)]
struct RankAccumulator {
    sessions: u64,
    prompts: u64,
    tokens: u64,
    has_token_data: bool,
    label: String,
}

/// Turn one Zeron chat and its journal into the same compact form used for
/// native sessions. The caller supplies device/profile filtering.
pub(crate) fn from_zeron_chat(input: ChatUsageInput) -> (UsageSessionInput, UsageInputCoverage) {
    let (records, skipped_lines, journal_unreadable) = match input.records {
        Some((records, skipped_lines)) => (records, skipped_lines, false),
        None => (Vec::new(), 0, true),
    };
    let (prompt_timestamps_ms, chat_unreadable) = match input.prompt_timestamps {
        Some(mut timestamps) => {
            timestamps.sort_unstable();
            (timestamps, false)
        }
        None => (Vec::new(), true),
    };

    let mut identity =
        input.chat.config.as_ref().map(|config| {
            AgentIdentity::new(config.harness, config.model.as_deref().unwrap_or(""))
        });
    let mut native_session_id = input.chat.harness_session_id.as_deref().and_then(nonempty);
    // Root SessionStarted rows carry the native resume id and the effective
    // model. Subagent SessionStarted rows are nested and do not identify the
    // chat's primary session.
    for record in &records {
        if let AgentEvent::SessionStarted {
            harness,
            model,
            session_id,
            ..
        } = &record.event
        {
            identity = Some(AgentIdentity::new(*harness, model));
            native_session_id = nonempty(session_id);
        }
    }
    let identity = identity.unwrap_or_else(AgentIdentity::unknown);
    let turns = turns_from_records(&records);
    let mut token_total = 0u64;
    let mut token_usage_events = Vec::new();
    let mut has_token_data = false;
    for turn in turns {
        for lane in turn.usage_by_lane.values() {
            has_token_data = true;
            token_total = token_total.saturating_add(lane.tokens);
            if let Some(timestamp_ms) = lane.timestamp_ms {
                token_usage_events.push(TokenUsageEvent {
                    timestamp_ms,
                    tokens: lane.tokens,
                });
            }
        }
    }

    let model = identity.model.or_else(|| {
        input
            .chat
            .config
            .as_ref()
            .and_then(|config| config.model.as_deref().and_then(nonempty))
    });
    let session = UsageSessionInput {
        // Keep the local chat row unique until merge_sessions has enough
        // native identity and creation-time information to match it safely.
        key: format!("zeron:{}", input.chat.id),
        agent_id: identity.id,
        agent_label: identity.label,
        native_session_id,
        project_path: input.chat.cwd.as_deref().and_then(nonempty),
        project_label: input.chat.cwd.as_deref().and_then(project_label),
        model,
        created_at_ms: Some(input.chat.created_at.timestamp_millis()),
        tokens: has_token_data.then_some(token_total),
        token_usage_events,
        prompts: prompt_timestamps_ms.len() as u64,
        prompt_timestamps_ms,
    };
    let coverage = UsageInputCoverage {
        unreadable_files: u64::from(chat_unreadable) + u64::from(journal_unreadable),
        malformed_lines: skipped_lines,
    };
    (session, coverage)
}

/// Merge Zeron chats into native history. The native row is authoritative when
/// present; journal/doc values fill fields that the native adapter cannot read.
pub(crate) fn merge_sessions(
    native: Vec<UsageSessionInput>,
    zeron: Vec<UsageSessionInput>,
) -> Vec<UsageSessionInput> {
    let mut sessions: BTreeMap<String, UsageSessionInput> = BTreeMap::new();
    let mut native_keys = HashMap::<(String, String), Vec<String>>::new();
    for session in native {
        if let Some(native_id) = session.native_session_id.as_deref() {
            native_keys
                .entry((session.agent_id.clone(), native_id.to_owned()))
                .or_default()
                .push(session.key.clone());
        }
        sessions.insert(session.key.clone(), session);
    }
    for session in zeron {
        let matching_key = session
            .native_session_id
            .as_deref()
            .and_then(|native_id| {
                native_keys.get(&(session.agent_id.clone(), native_id.to_owned()))
            })
            .and_then(|keys| nearest_session_key(keys, &sessions, session.created_at_ms));
        if let Some(existing) = matching_key.and_then(|key| sessions.get_mut(key)) {
            merge_fallback(existing, session);
        } else {
            sessions.insert(session.key.clone(), session);
        }
    }
    sessions.into_values().collect()
}

fn nearest_session_key<'a>(
    keys: &'a [String],
    sessions: &BTreeMap<String, UsageSessionInput>,
    created_at_ms: Option<i64>,
) -> Option<&'a String> {
    let created_at_ms = created_at_ms.filter(|timestamp| *timestamp > 0)?;
    keys.iter()
        .filter_map(|key| {
            let timestamp = sessions
                .get(key)
                .and_then(|session| session.created_at_ms.filter(|timestamp| *timestamp > 0))?;
            Some((key, created_at_ms.abs_diff(timestamp)))
        })
        .min_by_key(|(_, distance)| *distance)
        .map(|(key, _)| key)
}

/// Build the Wake-style Insights snapshot from compact, already de-duplicated
/// session summaries. `now_ms` is converted to the machine's local calendar.
pub(crate) fn build(
    sessions: Vec<UsageSessionInput>,
    mut coverage: UsageCoverage,
    now_ms: i64,
) -> UsageStatistics {
    let now = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Local::now);
    let today = now.date_naive();
    let current_week_start = week_start(today);
    let trend_start = week_start(today) - Duration::weeks((TREND_WEEKS - 1) as i64);
    let mut totals = UsageTotals::default();
    let mut today_tokens = 0u64;
    let mut week_tokens = 0u64;
    let mut has_today_token_data = false;
    let mut has_week_token_data = false;
    let mut days: BTreeMap<NaiveDate, DayAccumulator> = BTreeMap::new();
    let mut prompt_days = BTreeSet::new();
    let mut seen_agents = BTreeSet::new();
    let mut token_agents = BTreeSet::new();
    let mut seen_projects = BTreeSet::new();
    let mut agent_ranks: BTreeMap<String, RankAccumulator> = BTreeMap::new();
    let mut project_ranks: BTreeMap<String, RankAccumulator> = BTreeMap::new();
    let mut model_ranks: BTreeMap<String, RankAccumulator> = BTreeMap::new();
    let mut trend: BTreeMap<(usize, String), (String, u64)> = BTreeMap::new();
    let mut hourly = [0u64; 24];
    let mut weekdays = [0u64; 7];
    let mut months = [0u64; 12];

    totals.sessions = sessions.len() as u64;
    for session in &sessions {
        let agent_id = if session.agent_id.trim().is_empty() {
            UNKNOWN_AGENT_ID
        } else {
            session.agent_id.as_str()
        };
        let agent_label = if session.agent_label.trim().is_empty() {
            UNKNOWN_AGENT_LABEL
        } else {
            session.agent_label.as_str()
        };
        let known_agent = agent_id != UNKNOWN_AGENT_ID;
        let mut agent = if known_agent {
            seen_agents.insert(agent_id.to_owned());
            let agent = agent_ranks.entry(agent_id.to_owned()).or_default();
            agent.label = agent_label.to_owned();
            agent.sessions = agent.sessions.saturating_add(1);
            agent.prompts = agent.prompts.saturating_add(session.prompts);
            Some(agent)
        } else {
            None
        };

        totals.prompts = totals.prompts.saturating_add(session.prompts);
        if let Some(tokens) = session.tokens {
            totals.has_token_data = true;
            totals.tokens = totals.tokens.saturating_add(tokens);
            if known_agent {
                token_agents.insert(agent_id.to_owned());
                if let Some(agent) = agent.as_deref_mut() {
                    add_tokens(agent, tokens);
                }
            }

            if session.token_usage_events.is_empty()
                && let Some(created_date) = session
                    .created_at_ms
                    .filter(|timestamp| *timestamp > 0)
                    .and_then(local_datetime)
                    .map(|datetime| datetime.date_naive())
                    .filter(|date| *date <= today)
            {
                // A session created inside the window cannot contain older
                // usage, so its lifetime count is safely attributable to it.
                if created_date == today {
                    today_tokens = today_tokens.saturating_add(tokens);
                    has_today_token_data = true;
                }
                if created_date >= current_week_start {
                    week_tokens = week_tokens.saturating_add(tokens);
                    has_week_token_data = true;
                }
            }
        }

        for event in &session.token_usage_events {
            let Some(date) = local_datetime(event.timestamp_ms)
                .map(|datetime| datetime.date_naive())
                .filter(|date| *date <= today)
            else {
                continue;
            };
            if date == today {
                today_tokens = today_tokens.saturating_add(event.tokens);
                has_today_token_data = true;
            }
            if date >= current_week_start {
                week_tokens = week_tokens.saturating_add(event.tokens);
                has_week_token_data = true;
            }
        }

        if let Some(project) = session.project_path.as_deref().and_then(nonempty) {
            seen_projects.insert(project.clone());
            let rank = project_ranks.entry(project.clone()).or_default();
            rank.label = session
                .project_label
                .as_deref()
                .and_then(nonempty)
                .unwrap_or_else(|| project.clone());
            rank.sessions = rank.sessions.saturating_add(1);
            rank.prompts = rank.prompts.saturating_add(session.prompts);
            if let Some(tokens) = session.tokens {
                add_tokens(rank, tokens);
            }
        }

        if let Some(model) = session.model.as_deref().and_then(nonempty) {
            let rank = model_ranks.entry(model.clone()).or_default();
            rank.label = model;
            rank.sessions = rank.sessions.saturating_add(1);
            rank.prompts = rank.prompts.saturating_add(session.prompts);
            if let Some(tokens) = session.tokens {
                add_tokens(rank, tokens);
            }
        }

        if let Some(date) = session
            .created_at_ms
            .filter(|timestamp| *timestamp > 0)
            .and_then(local_datetime)
            .map(|datetime| datetime.date_naive())
            .filter(|date| *date <= today)
        {
            let day = days.entry(date).or_default();
            day.sessions = day.sessions.saturating_add(1);
        }

        let expected_prompts = session
            .prompts
            .max(session.prompt_timestamps_ms.len() as u64);
        // Native prompt rows are authoritative for totals and ranking. This
        // safeguards against an adapter that supplies timestamps but no count.
        totals.prompts = totals
            .prompts
            .saturating_add(expected_prompts.saturating_sub(session.prompts));
        if let Some(agent) = agent.as_deref_mut() {
            agent.prompts = agent
                .prompts
                .saturating_add(expected_prompts.saturating_sub(session.prompts));
        }
        if let Some(project) = session.project_path.as_deref().and_then(nonempty)
            && let Some(rank) = project_ranks.get_mut(&project)
        {
            rank.prompts = rank
                .prompts
                .saturating_add(expected_prompts.saturating_sub(session.prompts));
        }
        if let Some(model) = session.model.as_deref().and_then(nonempty)
            && let Some(rank) = model_ranks.get_mut(&model)
        {
            rank.prompts = rank
                .prompts
                .saturating_add(expected_prompts.saturating_sub(session.prompts));
        }

        for timestamp_ms in session.prompt_timestamps_ms.iter().copied() {
            let Some(local) = (timestamp_ms > 0)
                .then(|| local_datetime(timestamp_ms))
                .flatten()
                .filter(|local| local.date_naive() <= today)
            else {
                continue;
            };
            let date = local.date_naive();
            let day = days.entry(date).or_default();
            day.prompts = day.prompts.saturating_add(1);
            prompt_days.insert(date);
            hourly[local.hour() as usize] = hourly[local.hour() as usize].saturating_add(1);
            weekdays[date.weekday().num_days_from_monday() as usize] =
                weekdays[date.weekday().num_days_from_monday() as usize].saturating_add(1);
            months[(date.month() - 1) as usize] =
                months[(date.month() - 1) as usize].saturating_add(1);

            if known_agent && let Some(index) = trend_week_index(trend_start, date) {
                let slot = trend
                    .entry((index, agent_id.to_owned()))
                    .or_insert_with(|| (agent_label.to_owned(), 0));
                slot.1 = slot.1.saturating_add(1);
            }
        }
    }

    totals.today_tokens = has_today_token_data.then_some(today_tokens);
    totals.week_tokens = has_week_token_data.then_some(week_tokens);
    totals.agents = seen_agents.len() as u64;
    totals.projects = seen_projects.len() as u64;
    totals.active_days = prompt_days.len() as u64;
    let (current_streak_days, longest_streak_days) = streaks(&prompt_days, today);
    totals.current_streak_days = current_streak_days;
    totals.longest_streak_days = longest_streak_days;
    if let Some((date, day)) = days
        .iter()
        .filter(|(date, _)| prompt_days.contains(date))
        .max_by_key(|(_, counts)| counts.prompts)
    {
        totals.busiest_day = Some(date.format("%Y-%m-%d").to_string());
        totals.busiest_day_prompts = day.prompts;
    }

    coverage.indexed_sources = seen_agents.len() as u64;
    coverage.indexed_sessions = sessions.len() as u64;
    coverage.agents_without_token_data = seen_agents.difference(&token_agents).count() as u64;

    let recent_days = day_window(today, 0, 7, &days);
    let previous_days = day_window(today, 7, 7, &days);
    let heatmap = date_window(trend_start, TREND_WEEKS * 7, today, &days);
    let agent_trend = (0..TREND_WEEKS)
        .map(|index| {
            let week = trend_start + Duration::weeks(index as i64);
            let agents = trend
                .range((index, String::new())..=(index, String::from(char::MAX)))
                .map(|((_, agent_id), (agent_label, prompts))| UsageAgentPrompt {
                    agent_id: agent_id.clone(),
                    agent_label: agent_label.clone(),
                    prompts: *prompts,
                })
                .collect();
            UsageTrendWeek {
                week_start: week.format("%Y-%m-%d").to_string(),
                agents,
            }
        })
        .collect();

    let agents = into_ranks(agent_ranks);
    let projects = into_ranks(project_ranks);
    let models = into_ranks(model_ranks);
    UsageStatistics {
        scope: "device-all-agents".into(),
        generated_at_ms: now_ms,
        totals,
        recent_days,
        previous_days,
        heatmap,
        agent_trend,
        distribution: UsageDistribution {
            hours: hourly
                .into_iter()
                .enumerate()
                .map(|(hour, value)| UsageBucket {
                    label: format!("{hour:02}"),
                    value,
                })
                .collect(),
            weekdays: weekdays
                .into_iter()
                .zip(["周一", "周二", "周三", "周四", "周五", "周六", "周日"])
                .map(|(value, label)| UsageBucket {
                    label: label.into(),
                    value,
                })
                .collect(),
            months: months
                .into_iter()
                .enumerate()
                .map(|(month, value)| UsageBucket {
                    label: format!("{}月", month + 1),
                    value,
                })
                .collect(),
        },
        agents,
        projects,
        models,
        coverage,
    }
}

fn add_tokens(rank: &mut RankAccumulator, tokens: u64) {
    rank.tokens = rank.tokens.saturating_add(tokens);
    rank.has_token_data = true;
}

fn into_ranks(ranks: BTreeMap<String, RankAccumulator>) -> Vec<UsageRank> {
    ranks
        .into_iter()
        .map(|(id, rank)| UsageRank {
            id,
            label: rank.label,
            sessions: rank.sessions,
            tokens: rank.tokens,
            has_token_data: rank.has_token_data,
            prompts: rank.prompts,
        })
        .collect()
}

fn merge_fallback(base: &mut UsageSessionInput, fallback: UsageSessionInput) {
    if base.tokens.is_none() {
        base.tokens = fallback.tokens;
    }
    if base.token_usage_events.is_empty() {
        base.token_usage_events = fallback.token_usage_events;
    }
    if base.prompts == 0 {
        base.prompts = fallback.prompts;
    }
    if base.prompt_timestamps_ms.is_empty() {
        base.prompt_timestamps_ms = fallback.prompt_timestamps_ms;
    }
    if base.project_path.as_deref().and_then(nonempty).is_none() {
        base.project_path = fallback.project_path;
    }
    if base.project_label.as_deref().and_then(nonempty).is_none() {
        base.project_label = fallback.project_label;
    }
    if base.model.as_deref().and_then(nonempty).is_none() {
        base.model = fallback.model;
    }
    if base.created_at_ms.is_none_or(|timestamp| timestamp <= 0) {
        base.created_at_ms = fallback.created_at_ms;
    }
    if base.agent_id.trim().is_empty() {
        base.agent_id = fallback.agent_id;
    }
    if base.agent_label.trim().is_empty() {
        base.agent_label = fallback.agent_label;
    }
}

fn turns_from_records(records: &[JournalRecord]) -> Vec<Turn> {
    let mut turns = Vec::new();
    let mut current: Option<MutableTurn> = None;

    for record in records {
        match &record.event {
            AgentEvent::SessionStarted { .. } => {
                current.get_or_insert_with(MutableTurn::default);
            }
            AgentEvent::Steered { .. } => {
                if let Some(previous) = current.take() {
                    turns.push(finish_turn(previous));
                }
                current = Some(MutableTurn::default());
            }
            AgentEvent::Done { .. } => {
                let previous = current.take().unwrap_or_default();
                turns.push(finish_turn(previous));
            }
            event => {
                let turn = current.get_or_insert_with(MutableTurn::default);
                visit_event(event, "", record.timestamp_ms, turn);
            }
        }
    }
    if let Some(last) = current {
        turns.push(finish_turn(last));
    }
    turns
}

fn visit_event(
    event: &AgentEvent,
    lane_id: &str,
    timestamp_ms: Option<i64>,
    turn: &mut MutableTurn,
) {
    match event {
        AgentEvent::Subagent {
            parent_tool_use_id,
            event,
        } => {
            let child_lane = if lane_id.is_empty() {
                parent_tool_use_id.clone()
            } else {
                format!("{lane_id}/{parent_tool_use_id}")
            };
            visit_event(event, &child_lane, timestamp_ms, turn);
        }
        AgentEvent::Steered { .. } if !lane_id.is_empty() => {
            if let Some(usage) = turn.usage_by_lane.remove(lane_id) {
                let closed_lane = format!("{lane_id}#{}", turn.closed_lane_usages);
                turn.closed_lane_usages = turn.closed_lane_usages.saturating_add(1);
                turn.usage_by_lane.insert(closed_lane, usage);
            }
        }
        AgentEvent::SessionStarted { .. } => {}
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
        } => {
            turn.usage_by_lane.insert(
                lane_id.to_owned(),
                LaneUsage {
                    tokens: (*input_tokens).saturating_add(*output_tokens),
                    timestamp_ms,
                },
            );
        }
        _ => {}
    }
}

fn finish_turn(turn: MutableTurn) -> Turn {
    Turn {
        usage_by_lane: turn.usage_by_lane,
    }
}

fn local_datetime(timestamp_ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(timestamp_ms).single()
}

fn week_start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(day.weekday().num_days_from_monday() as i64)
}

fn trend_week_index(start: NaiveDate, day: NaiveDate) -> Option<usize> {
    let delta = (week_start(day) - start).num_days();
    if delta < 0 || delta % 7 != 0 {
        return None;
    }
    let index = (delta / 7) as usize;
    (index < TREND_WEEKS).then_some(index)
}

fn day_window(
    today: NaiveDate,
    days_ago: u64,
    len: usize,
    values: &BTreeMap<NaiveDate, DayAccumulator>,
) -> Vec<UsageDay> {
    let Some(end) = today.checked_sub_signed(Duration::days(days_ago as i64)) else {
        return Vec::new();
    };
    let start = end - Duration::days(len.saturating_sub(1) as i64);
    (0..len)
        .map(|offset| {
            let date = start + Duration::days(offset as i64);
            let counts = values.get(&date);
            UsageDay {
                day: date.format("%Y-%m-%d").to_string(),
                sessions: counts.map_or(0, |counts| counts.sessions),
                prompts: counts.map_or(0, |counts| counts.prompts),
            }
        })
        .collect()
}

fn date_window(
    start: NaiveDate,
    len: usize,
    today: NaiveDate,
    values: &BTreeMap<NaiveDate, DayAccumulator>,
) -> Vec<UsageDay> {
    (0..len)
        .map(|offset| {
            let date = start + Duration::days(offset as i64);
            let counts = (date <= today).then(|| values.get(&date)).flatten();
            UsageDay {
                day: date.format("%Y-%m-%d").to_string(),
                sessions: counts.map_or(0, |counts| counts.sessions),
                prompts: counts.map_or(0, |counts| counts.prompts),
            }
        })
        .collect()
}

fn streaks(active_dates: &BTreeSet<NaiveDate>, today: NaiveDate) -> (u64, u64) {
    let longest = active_dates.iter().copied().fold(
        (0u64, None::<NaiveDate>, 0u64),
        |(best, previous, current), date| {
            let next = if previous.is_some_and(|previous| date == previous + Duration::days(1)) {
                current.saturating_add(1)
            } else {
                1
            };
            (best.max(next), Some(date), next)
        },
    );
    let current_end = if active_dates.contains(&today) {
        Some(today)
    } else {
        today.checked_sub_signed(Duration::days(1))
    };
    let mut current_streak = 0u64;
    let mut date = current_end;
    while let Some(day) = date {
        if active_dates.contains(&day) {
            current_streak = current_streak.saturating_add(1);
            date = day.checked_sub_signed(Duration::days(1));
        } else {
            break;
        }
    }
    (current_streak, longest.0)
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn project_label(path: &str) -> Option<String> {
    let path = nonempty(path)?;
    Some(
        std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Unknown project".into()),
    )
}
