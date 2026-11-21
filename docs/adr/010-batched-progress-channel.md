# ADR-010: Batched progress at ~4 Hz via a Tauri Channel

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Per-chunk progress events from dozens of tasks would produce thousands of IPC messages per second,
burning CPU on serialization and webview re-renders for changes no human can perceive.

## Decision
The engine aggregates progress for all active tasks into one `ProgressBatch` every ~250 ms. The
desktop shell forwards batches to the frontend through a Tauri `ipc::Channel` (built for ordered,
high-throughput streaming); rarer events (state changes, metadata) use regular app events.

## Consequences
- UI updates stay smooth (CSS transitions interpolate between samples) and CPU stays low.
- Progress latency is bounded by the tick (≤ 300 ms target).
- Project rule: progress to the UI is batched and ≤ 10 Hz.
