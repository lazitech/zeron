# Usage Statistics Dashboard Design

## Goal

Add a local usage statistics page to Zeron v0.2.79 that presents the usage overview shown in the user's Wake reference image, including token totals, activity history, prompt patterns, and per-agent rankings.

## Scope and data ownership

- Statistics cover the current device and active Zeron profile only. There is no cross-device synchronization in this version.
- The statistics page is opened from a chart button in the chat sidebar and keeps the normal sidebar visible.
- An engine RPC returns a pre-aggregated snapshot. The UI does not crawl transcript documents or call model providers for statistics.
- Existing per-chat run journals are the historical source for Token `Usage` events. New journal rows include a UTC event timestamp. Legacy rows remain usable for all-time totals, but their missing timestamps are not inferred from unrelated chat activity.
- A snapshot reports whether all-time token data and date-bucketed usage have complete timestamp coverage. A missing date bucket means unknown history, not zero usage.

## Dashboard content

The page includes:

1. Overview cards: sessions, tokens, prompts, agents, projects, and active days.
2. Recent seven days compared with the preceding seven days.
3. An activity heatmap with current and longest streak summaries.
4. Monthly prompt counts by agent.
5. Prompt counts by local hour of day.
6. An agent ranking with a selector for sessions or tokens.

Token use is the sum of the final input and output counts for each completed agent turn. When a journal run emits multiple cumulative usage updates, count only the latest update for each main-agent/subagent lane before the run's `Done` event. Prompts are user-role messages in the chat document, excluding system and assistant entries. Their agent attribution follows the chat journal's ordered turn boundaries (`SessionStarted` and `Steered`), matched to user entries in order; if a boundary or identity is unavailable, attribute the prompt to Unknown rather than guessing. Session counts are distinct local chats with at least one prompt. Agents are distinct harness/model identities associated with journaled usage records; unavailable identities are shown as unknown rather than discarded. Projects are distinct non-empty workspace roots attached to local chats. Active days are local calendar days with a timestamped prompt or usage record.

All date and hour grouping uses the device's local timezone. Prompt dates come from chat-document entry timestamps; usage dates come from journal timestamps. Recent comparisons use consecutive local calendar days, including the current day. Legacy token totals remain visible, but untimestamped usage cannot contribute to daily or hourly token buckets.

## Architecture

- Add a dedicated `Route::Statistics` and matching navigation history entry, with a chart button near the chat sidebar's existing bottom identity/settings controls.
- Add a focused engine-side usage statistics module. It reads local chat metadata and the active profile's journals, performs aggregation outside the UI, and exposes a typed RPC result through the existing RPC method registry.
- Extend journal rows with an optional timestamp that defaults to absent during deserialization. New appends write the current UTC time. Existing JSONL rows remain backward compatible.
- Render the page using existing settings page layout primitives and the current GPUI theme. Charts are native GPUI elements; no new charting dependency is introduced.
- Loading or journal read errors are represented as an unavailable/incomplete portion of the snapshot. The UI remains navigable and shows the scope and coverage state.

## Out of scope

- Cross-device synchronization, provider billing/cost estimation, cache-token cost accounting, exporting, arbitrary date-range filtering, and per-project detail drilldowns.
- Backfilling day/hour buckets by guessing event time from a chat's last activity.

## Acceptance criteria

- A user can open and leave Statistics from the chat sidebar without losing normal app navigation behavior.
- The page presents every section listed above and labels its scope as this device/profile.
- Token totals include journaled legacy Usage events, while legacy events without timestamps do not fabricate heatmap, daily, or hourly activity.
- Newly appended journal events have timestamps and appear in the correct local date/hour buckets.
- Repeated cumulative usage updates from one run are represented once in turn totals.
- Missing or malformed legacy data is surfaced as incomplete coverage rather than a misleading zero.
- No provider network requests or transcript-wide UI scans are required to render the page.

## Error and compatibility behavior

The timestamp field is optional on read, preserving compatibility with existing JSONL journals. Corrupt lines retain the journal reader's current skip-and-log behavior. If a profile cannot provide some chat or journal data, valid records still contribute to the snapshot and coverage flags indicate the partial result. An empty profile is shown as an empty state with zero totals.

## Verification

Implementation should be formatted and reviewed against the v0.2.79 source conventions. Automated tests are not part of this requested change unless the user asks for them.
