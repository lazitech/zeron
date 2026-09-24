# Usage Statistics Dashboard Implementation Plan

> **For agentic workers:** Execute this plan inline in the current session, one task at a time. Do not run automated tests unless the user asks.

**Goal:** Add a local, profile-scoped statistics dashboard to Zeron v0.2.79 matching the sections in the supplied Wake reference.

**Architecture:** Persist an optional UTC timestamp alongside each new run-journal event, read local chat prompt timestamps and usage records in the engine, and expose one aggregated `UsageStatistics` RPC response. Add a statistics route and render a compact report row, native GPUI charts, and a date-aligned heatmap.

**Tech Stack:** Rust, Serde, Chrono, Loro chat documents, the existing JSON-RPC client/server, and GPUI.

**Spec:** `docs/superpowers/specs/2026-09-23-usage-statistics-design.md`

## Global Constraints

- Statistics cover the current device and active Zeron profile only.
- All day and hour buckets use the device's local timezone.
- Existing journal rows without timestamps contribute to lifetime tokens but never to fabricated day/hour buckets.
- New journal rows include a UTC event timestamp and remain backward compatible with existing JSONL.
- The UI calls one aggregate RPC and does not scan transcripts or query model providers.
- Use GPUI primitives and existing settings-page layout helpers; add no chart dependency.
- The dashboard includes a flat overview row, recent seven-day comparison, activity heatmap, monthly prompts by agent, prompts by local hour, and an agent ranking switchable between sessions and tokens.

---

### Task 1: Add timestamped journal records and shared statistics DTO

**Files:**
- Modify: `crates/engine/src/run_journal.rs`
- Create: `crates/proto/src/usage_stats.rs`
- Modify: `crates/proto/src/lib.rs`
- Modify: `crates/rpc/src/lib.rs`

**Interfaces:**
- `JournalRecord { seq: u64, timestamp_ms: Option<i64>, event: AgentEvent }` is the engine's read-only representation of one journal row.
- `RunJournal::records(chat_id: &str) -> Result<Vec<JournalRecord>, JournalError>` reads a chat's valid rows while preserving timestamps when available.
- `RunJournal::append` records `Utc::now().timestamp_millis()` in each newly serialized JSONL row; deserializing a legacy row yields `timestamp_ms: None`.
- `UsageStatistics` is the serde wire response for aggregate totals (including streaks), daily buckets, monthly agent prompt counts, hourly prompt counts, ranking rows, and coverage flags.
- `zeron_rpc::methods::USAGE_STATISTICS` is the unary method name `UsageStatistics`.

- [x] Add `#[serde(default)] timestamp_ms: Option<i64>` to the internal journal row and fill it on append.
- [x] Add a records reader that returns `seq`, timestamp, and event; keep `replay`'s existing `(seq, event)` contract by projecting from those records.
- [x] Define and export DTOs with these fields: totals (`sessions`, `tokens`, `prompts`, `agents`, `projects`, `active_days`, `current_streak_days`, `longest_streak_days`); daily rows (`day`, `sessions`, `prompts`, `tokens`); monthly prompt rows (`month`, `agent_id`, `agent_label`, `prompts`); hour rows (`hour`, `prompts`); agent ranking rows (`agent_id`, `agent_label`, `sessions`, `tokens`); coverage (`token_dates_complete`, `prompt_dates_complete`, `untimed_token_turns`, `unreadable_chats`, `unreadable_journals`); and response scope plus generated time.
- [x] Add the shared RPC method constant and keep all legacy journal JSON readable without migration.

### Task 2: Aggregate current-profile prompt and token history in the engine

**Files:**
- Create: `crates/engine/src/usage_stats.rs`
- Modify: `crates/engine/src/lib.rs`
- Modify: `crates/engine/src/run_journal.rs`
- Modify: `crates/engine/src/sessions.rs`
- Modify: `crates/engine/src/doc_host.rs`
- Modify: `crates/engine/src/rpc.rs`

**Interfaces:**
- `SessionsEngine::usage_records(chat_id: &str) -> Result<Vec<JournalRecord>, EngineError>` delegates to the active profile's journal.
- `DocHost::prompt_timestamps(chat_id: &str) -> Result<Option<Vec<i64>>, EngineError>` reads only `created_at` from user-role entries, using an already-open doc when present and otherwise importing the local SQLite snapshot without opening a chat room; `None` means a chat expected to have content has no readable local snapshot.
- `usage_stats::build(inputs: Vec<ChatUsageInput>, now_ms: i64)` returns a serializable `UsageStatistics` in local calendar time. Each `ChatUsageInput` contains one local `Chat`, optional prompt timestamps, and optional journal records.
- `EngineRpc` handles `USAGE_STATISTICS` with no parameters and includes only chats whose `device_id` equals `doc_host.device_id()`.

- [x] Implement `DocHost::prompt_timestamps` so open docs are read in memory and cold docs are loaded one at a time from `DocsStore`; return `None` for a missing snapshot when the chat row has a `last_message_at`, and `Some(vec![])` for a known empty chat.
- [x] Add a `SessionsEngine` wrapper to read full journal records for one chat without exposing the journal directory to UI code.
- [x] Implement an aggregator that aligns sorted user entries with ordered journal turn boundaries (`SessionStarted` and `Steered`), folds repeated cumulative `Usage` updates to the latest value per lane before top-level `Done`, carries harness/model identity into the turn, and uses Unknown when identity cannot be established.
- [x] Count untimestamped token turns only in lifetime totals and mark prompt coverage incomplete when a local chat snapshot is missing or its latest message timestamp trails the workspace row's `last_message_at`.
- [x] Build dated prompt/activity buckets using each user entry's `created_at`; build usage buckets only from journal timestamps; convert both through `chrono::Local`.
- [x] Calculate distinct prompted chats, workspaces (`space_id`, falling back to `cwd`), agents, active days and streaks, the latest 7 local days and preceding 7 local days, a 364-day heatmap, the trailing 12 calendar months, and 24 hourly prompt bins.
- [x] Handle nested `Subagent` events as separate attribution lanes and preserve their usage totals without including event text in the RPC response.
- [x] Register the unary method in `EngineRpc`; unreadable local chat data increments coverage while valid chats still contribute.

### Task 3: Add statistics navigation while preserving the chat sidebar

**Files:**
- Modify: `crates/ui/src/shell.rs`
- Modify: `crates/ui/src/icons.rs`
- Create: `crates/ui/assets/icons/usage-stats.svg`

**Interfaces:**
- `Route::Statistics` and `NavEntry::Statistics` represent the statistics surface in route and back/forward history.
- `Shell::open_statistics` switches routes, records navigation, closes transient menus, and requests a fresh statistics page instance for the visit.
- The `SidebarPane` renders the usual chat sidebar while the route is `Statistics`.

- [x] Add the custom 16×16 chart icon to the existing embedded asset list.
- [x] Add `Statistics` to `Route` and `NavEntry`, route initialization, current-entry calculation, and `apply_nav`.
- [x] Add a main-outlet branch below the unified titlebar; the statistics page header supplies its visible title.
- [x] Add a “统计” chart button directly above the bottom user-menu row in the normal sidebar; selected styling follows the current theme, and clicking opens the new route.
- [x] Keep settings navigation behavior unchanged and keep the chat sidebar present while statistics is selected.

### Task 4: Render the dashboard and connect its RPC state

**Files:**
- Create: `crates/ui/src/statistics.rs`
- Modify: `crates/ui/src/lib.rs`
- Modify: `crates/ui/src/shell.rs`

**Interfaces:**
- `StatisticsPage::new(state: Entity<AppState>, cx: &mut Context<Self>)` starts one `USAGE_STATISTICS` call and stores its result in `Loadable<UsageStatistics>`.
- The page offers a retry action on RPC failure and displays `scope` and coverage status on both loaded and partial snapshots.
- A ranking toggle switches only the ranking metric between `sessions` and `tokens`.

- [x] Add a dedicated page component using `settings::widgets::{PageScroll, page_column, page_header, page_subtitle}` and the existing `Loadable` pattern.
- [x] Draw a flat summary row for sessions, tokens, prompts, agents, projects, and active days, with the Token total highlighted in the current theme accent.
- [x] Draw current-versus-previous seven-day summaries, including tokens, prompts, and active days, and show an incomplete-history note when untimestamped legacy usage exists.
- [x] Draw the 364-day activity heatmap with Monday-aligned columns, weekday/month labels, and current/longest streaks from timestamped prompt or token activity.
- [x] Draw trailing-12-month per-agent prompt bars, the 24-hour prompt histogram, and ranking rows with the sessions/tokens selector.
- [x] Add an empty-profile state, a loading state, a retryable error state, and explicit current-device/current-profile scope text.
- [x] Wire the page module into the shell and format the changed Rust files; do not run automated tests unless requested.

### Development environment

- [x] Add `environment.yml` with Conda build helpers, bootstrap the repository's stable Rust toolchain inside the environment, and document Ubuntu GPUI/WebKit dependencies and pkg-config search paths.

---

## Plan self-review

- Spec coverage: Task 1 covers timestamp compatibility and DTO/RPC shape; Task 2 covers local aggregation and coverage semantics; Tasks 3–4 cover navigation and every dashboard section.
- Placeholder scan: all implementation steps have concrete file ownership and observable outputs.
- Type consistency: the RPC method constant, `UsageStatistics` DTO, `SessionsEngine::usage_records`, `DocHost::prompt_timestamps`, and `StatisticsPage::new` signatures are named consistently across tasks.
- Scope check: all work belongs to one end-to-end dashboard feature; no independent subsystem needs to be split out.
