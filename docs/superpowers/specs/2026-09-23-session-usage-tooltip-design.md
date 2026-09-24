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

Store the snapshot atomically in chat document metadata and carry it through
the existing transcript watch. Preserve it across local restart, replication,
thin-document rebuilds, and navigation caches. Clear it when starting a new
native session, preserve it when resuming. Usage-only events must update a
parked session without reopening its turn. Child events must not overwrite the
parent snapshot. Existing sessions get data on their next provider update;
there is no history scan or provider request on hover.

Verification targets: cumulative versus last-call selection, repeated updates,
missing cache versus measured zero, unknown and invalid ratios, old-host wire
compatibility, snapshot/import/rebuild, metadata-only watch updates, reconnect,
and session navigation.
