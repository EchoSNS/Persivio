# ADR-005: Durability ordering: data → sync_data → checkpoint

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
After a crash we resume from persisted segment positions. If a checkpoint records position P but
the bytes before P were still in the OS page cache when power was lost, resuming from P leaves a
hole of zeros or garbage that no later step detects.

## Decision
The checkpointer always (1) has the writer write out all queued data, (2) calls
`File::sync_data()` and waits for it, and only then (3) persists segment positions via
`CheckpointSink`. Persisted positions may lag behind the data on disk, but never lead it.

## Consequences
- Crash safety: at worst we re-download one checkpoint interval (1–2 s) of data.
- `sync_data` costs latency, so checkpoints are periodic, never per chunk.
- The Phase 4 kill-simulation test (50 consecutive runs with hash checks) enforces this ordering.
