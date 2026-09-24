# Session usage tooltip

The user approved a hover card containing exactly three rows: `Input tokens`,
`Output tokens`, and `Cache hit rate`. The context ring and its percentage keep
their existing occupancy meaning. The card replaces the old context details;
there is no total, cached-token count, or extra heading.

Input includes cached input. Cache hit rate is cumulative cached input divided
by cumulative input, formatted to one decimal percent. Counts use thousands
separators. Missing data, a zero denominator, or an invalid cache count displays
an em dash; reported zero token counts display zero.

For the pictured Codex session, normalize `tokenUsage.total` directly into an
optional typed session snapshot. Do not sum repeated cumulative notifications,
use `last` as a session total, or change the existing billing event. Cache
fields missing from the source stay unknown. Other harnesses without this
cumulative breakdown display unknown values rather than inferred totals.

The user subsequently requested the same support for Pi and OpenCode. OpenCode
must deduplicate per-message counters and restore existing session usage on
connection; native cumulative reports are a separate authoritative source.
Pi's pinned ACP adapter omits billing details, so a host-owned read-only reader
uses the adapter's session map to select exactly one native file and checks its
session header. It reads usage metadata incrementally and refreshes at turn
settlement. The observer stops with the ACP session. Pi input is normalized as
`input + cacheRead + cacheWrite`, with `cacheRead` as the cached subset; its
native compaction, branch-summary and standalone usage entries also count.
Neither provider may turn replayed/repeated events into extra consumption.

Store the snapshot atomically in chat document metadata and carry it through
the existing transcript watch. Preserve it across local restart, replication,
thin-document rebuilds, and navigation caches. Clear it when starting a new
native session, preserve it when resuming. Usage-only events must update a
parked session without reopening its turn. Child events must not overwrite the
parent snapshot. Existing Codex sessions without a saved snapshot get data on
their next provider update; Pi and OpenCode also restore native usage on
attachment. There is no history scan or provider request on hover.

Verification targets: cumulative versus last-call selection, repeated updates,
missing cache versus measured zero, unknown and invalid ratios, old-host wire
compatibility, snapshot/import/rebuild, metadata-only watch updates, reconnect,
and session navigation.
