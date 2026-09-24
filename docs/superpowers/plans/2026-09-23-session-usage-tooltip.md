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
