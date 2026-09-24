# Context usage

Every selected conversation has a context ring beneath its composer, including
project-less and remote conversations. The ring turns amber at 75% and red at 90%; its drawing clamps
at a full circle while the label preserves over-capacity measurements. A dash
means the harness has not reported enough data to calculate a percentage. Measured
zero is displayed as 0%.

Hovering displays exactly three session metrics: **Input tokens**, **Output
tokens**, and **Cache hit rate**. Input includes cached input, and the rate is
cumulative cached input divided by cumulative input. Unknown data or a zero
denominator is shown as `—`; a reported zero token count is shown as `0`.
Supported cumulative sources:

- Codex reads `tokenUsage.total`, not the latest call in `last`.
- OpenCode v1 restores session message history and aggregates billing snapshots
  by message ID; repeated updates replace that message's counters. OpenCode v2
  restores cumulative tokens from session info and replaces them on native
  usage events. Input includes cache reads and writes; output uses native
  `tokens.output` (OpenCode reports reasoning separately). Missing fields
  remain unknown, rather than presenting a partial sum as the total.
- Pi reads the exact native session file referenced by pi-acp's session map.
  The host checks the header's session ID, reads existing usage on attachment,
  then follows appended billing metadata. Counts include assistant calls,
  billed tool results, standalone usage entries, and compaction/branch-summary
  usage, matching Pi's native session-statistics categories. Input includes
  `input + cacheRead + cacheWrite`; cached input is `cacheRead`. Text and tool
  bodies are not retained by the usage reader. No native files are modified.

Other harnesses currently show unavailable values. Existing Codex sessions
populate the card on their next provider update; Pi and OpenCode also restore
their existing billing data when attached.

Session metrics are stored as one `meta.sessionUsage` snapshot. Notifications
replace that snapshot rather than accumulating it, so repeated updates cannot
inflate the counts. The snapshot survives restart, replication and thin rebuilds,
and is included in both opening and incremental transcript updates. Missing
fields replace old fields rather than retaining a stale cache numerator. Starting
a fresh native session clears the snapshot; resuming keeps it. No local history
scan or provider request is performed on hover. If session usage is available
without a context capacity, the control is visible with a `—` context label.

The host normalizes context occupancy separately from billing `Usage` events:

| Harness | Measurement |
| --- | --- |
| Claude Code | Latest parent assistant message's input plus cache-read/cache-creation tokens; capacity from that model's result metadata. Aggregate result billing and child agents are excluded. |
| Codex | Latest model call (`tokenUsage.last`), with `modelContextWindow`; never the cumulative thread total. |
| OpenCode | Latest parent assistant message's total, or input/output/cache counts; capacity from the advertised provider/model catalog. Empty in-progress placeholders do not clear a measurement. |
| ACP (Devin, Grok, Hermes) | `usage_update.used` and advertised capacity when the agent reports them. |
| Pi | Native `getContextUsage()` via a Zeron-enabled extension, including the active model capacity and Pi’s branch/compaction-aware estimate. An unknown reading after compaction clears stale occupancy. |
| Cursor | The pinned SDK exposes billed per-turn counts, not context occupancy. The shared control shows unavailable. |
| Mock / older hosts | Unavailable until a context snapshot is supplied. |

`AgentEvent::ContextUsage` updates one atomic `meta.contextUsage` value in the
session document. Partial measurements preserve known fields; a zero capacity is
ignored. New non-resumed runs clear old usage. Post-turn updates can refresh the
snapshot without reopening a completed turn, and subagent events cannot change
the parent meter.

The existing document sync carries the value through the relay and local storage.
`WatchDocMessages` includes a typed `TranscriptUpdate` envelope with the snapshot,
including context-only commits and the opening reset. Old readers ignore the
additive field; old hosts decode as unavailable. Thin-document rebuilding also
preserves the snapshot. UI rendering reads the in-memory state, with no polling,
provider calls, filesystem access, or new remote endpoint.

Validation covers adapter normalization, compaction to zero, cache accounting,
subagent isolation, snapshot and incremental import, thin rebuilds, context-only
watch updates, reconnects, and old/new watch compatibility. Screenshots use the
native desktop attached to a viewing engine; a separate host sends deterministic
Claude protocol fixtures through a local relay. The measurements are test data.

Pi context occupancy is independent of cumulative billing. At conversation
startup Zeron installs its owned `zeron-context.js` into Pi's extensions directory
(respecting `PI_CODING_AGENT_DIR`). It activates only when Zeron passes a private
per-run metadata directory; ordinary Pi processes register no handlers. The
extension publishes numeric context snapshots on session, message, model and
compaction events. The host polls that directory alongside billing metadata and
removes it when the native session task ends. `ContextUsageSnapshot` replaces
unknown fields too, so compaction never leaves the previous percentage visible.
Pi session histories, provider settings and credentials are not modified.
