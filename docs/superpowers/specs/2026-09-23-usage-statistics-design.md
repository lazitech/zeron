# Usage Statistics Dashboard Design

## Goal

Add a local Insights page to Zeron with the same core usage totals, activity
views, time distributions, and rankings as Wake Insights. The page combines
supported native coding-agent histories with Zeron's own local chats so the
result is not limited to runs launched through Zeron.

## Scope and data ownership

- The snapshot covers this device only. It reads local native histories through
  Wake's adapters and unarchived Zeron chats hosted by this device; it does not
  synchronize statistics across devices.
- Native transcripts are opened read-only. Zeron stores only compact session
  metadata, prompt timestamps, usage totals, and source fingerprints in a
  device-local SQLite index. Transcript bodies are never copied into that
  index.
- The index is refreshed on demand and reparses a native file only when its
  path, modification time, or size changes. A scan or parse failure preserves
  the previous good summary where possible and reports partial coverage.
- Zeron chats merge with a native session by the harness slug and native
  session id, using the nearest valid creation timestamp when an id has been
  reused. If the native creation time is unavailable, the rows stay separate
  rather than risking the collapse of distinct sessions. Native metadata is
  preferred; Zeron metadata fills values a native adapter does not provide.
- Archived native sessions and archived Zeron chats are excluded, matching
  Wake Insights.

## Dashboard content

The page presents:

1. Overview totals for sessions, tokens, prompts, agents, projects, and active
   days.
2. Sessions, prompts, and active days for the latest seven local dates compared
   with the preceding seven dates.
3. A prompt-only, Monday-aligned heatmap covering 26 calendar weeks, with
   current streak, longest streak, and busiest day.
4. A 26-week prompt trend stacked by agent.
5. Prompt distributions by local hour, weekday, and month, with peak labels.
6. Agent, project, and model rankings. Each board can be sorted by sessions,
  tokens, or prompts.

When no sessions are available, the page shows an empty state. Source
coverage details remain in the engine snapshot for
diagnostics and are not displayed on the dashboard.

Wake-specific “Agents asking Wake” telemetry is excluded because Zeron has no
equivalent local event source.

## Metric semantics

- Sessions count distinct, unarchived native sessions and local Zeron chats
  after merging duplicates. Session activity is assigned to its creation date.
- Prompts count main-line user messages, excluding sidechains. Prompt totals
  and rankings include messages without a usable timestamp.
- Tokens are the lifetime totals reported by each native session or Zeron's
  journal. An unknown token total remains unknown; it is not presented as zero.
  Tokens are not attributed to recent dates because a session's lifetime total
  cannot be reliably split across days.
- Active days, streaks, the heatmap, agent trend, and time distributions use
  timestamped main-line prompts only. An untimestamped prompt contributes to
  totals and rankings but not date-based views.
- Project rankings are keyed by the non-empty project path. Model rankings use
  the session model when available. Agents are grouped by agent family, not by
  model.
- All date and hour grouping uses the device's local timezone. Future-dated
  records do not contribute to date-based views.

## Architecture

- A Statistics route is opened from a chart button in the chat sidebar and
  retains the normal sidebar/navigation behavior.
- A typed engine RPC scans native history off the async executor, reads local
  chat metadata, prompt timestamps, and journals, merges matching sessions,
  then returns only the aggregate snapshot to the UI.
- The native index lives below the device data root. Failure to open or refresh
  the index does not stop the engine; the UI can still show Zeron data, while
  source coverage details remain in the engine snapshot for diagnostics.
- The UI uses the existing GPUI theme and layout primitives; no charting
  dependency or provider network request is introduced.

## Acceptance criteria

- The page includes all Dashboard content listed above and identifies its scope
  as all supported local agents on this device.
- Native sessions across supported Wake adapters appear even when they were
  never launched by Zeron.
- Archived sessions are excluded, sidechain messages do not count as prompts,
  and matching Zeron/native sessions count once.
- Totals, rankings, date views, and time distributions follow the metric
  semantics above. In particular, token totals are not assigned to dates.
- A partial native or Zeron history is recorded in RPC coverage metadata while
  valid records continue contributing to the snapshot; the dashboard does not
  display a coverage panel.
- Rendering does not scan transcripts in the UI or call model providers.

## Compatibility and verification

The statistics RPC is additive. The native SQLite index is application-owned
and migrates older index schemas when opened. Existing run journal rows remain
readable; malformed JSONL lines retain the journal reader's skip-and-log
behavior and increment the coverage count.

Implementation should follow the repository's Rust formatting conventions.
Automated tests or builds are run only when requested by the user.
