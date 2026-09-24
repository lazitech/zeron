# Session usage tooltip implementation

1. Add an optional cumulative session-usage DTO and event; normalize Codex's
   cumulative notification without modifying existing per-turn billing data.
2. Persist the complete snapshot in session metadata, include it in full and
   incremental transcript watches, and retain it in thin rebuilds.
3. Carry the snapshot through the UI state and navigation cache; render only
   the three approved rows in the ring's hover card.
4. Add focused coverage for normalization, missing values, snapshot replacement,
   replication and usage-only watch updates. Review the final diff for scope
   and compatibility.

## Pi and OpenCode extension

5. Restore and aggregate OpenCode's native usage, separating per-message
   updates from cumulative session reports and excluding child sessions.
6. Add a Pi metadata reader scoped by pi-acp's exact session mapping, with
   header validation, incremental reads, deduplication and bounded lifecycle.
7. Exercise resumed sessions, final-turn flushes, duplicate notifications,
   incomplete records and provider accounting conventions with local fixtures.
