# Statistics Startup Preload Design

## Goal

Make the Statistics page ready sooner by preparing its complete snapshot while
Zeron is starting, without making the window wait for local history scans.

## Design

When the UI attaches to an engine, it requests `UsageStatistics` once in the
background. This works for both an embedded engine and an already-running local
daemon because the request uses the attached RPC client. `AppState` retains the
latest successful snapshot in memory. The statistics page renders that snapshot
immediately when available; if it is older than one minute, it remains visible
while a fresh snapshot is requested in the background. If startup preloading is
still running when the page opens, the existing loading state remains until the
request completes.

Preload failures do not affect application startup or discard a previously
successful snapshot. If no snapshot exists, the page reports the error and a
later page visit can retry. Replacing the engine clears the snapshot so data
from another runtime is never shown.

The cache is process-local. Native history summaries remain in the existing
SQLite index; this change does not persist full dashboard snapshots or alter
the statistics RPC format.

## Implementation steps

1. Add an in-memory snapshot, freshness timestamp, and single-flight status to
   `AppState`.
2. Start a background statistics request when the engine attaches, and request
   a refresh on a later statistics-page visit when the cache is stale.
3. Have `StatisticsPage` observe the shared snapshot and display it immediately,
   while preserving the current loading and error behavior when no snapshot is
   available.
4. Run formatting and `cargo check -p zeron`.
