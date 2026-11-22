# ADR-014: Protocol seam as traits in pv-core

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
The plan's `RunContext` sketch names concrete types (`Arc<SharedLimiter>`, `CheckpointSink`,
`ProgressSink`). Concrete limiter or checkpoint types would force `pv-core` to depend on
`governor` or `sqlx`, which breaks its "no I/O" boundary, and protocols could not be
tested without the real engine.

## Decision
- `RateLimiter`, `CheckpointSink` and `RunEvents` are **traits** in `pv-core`, passed as
  `Arc<dyn …>`. The engine implements the limiter with `governor` (Phase 5), and `pv-store`'s
  `Store` implements `CheckpointSink` directly.
- `ProgressSink` is a concrete struct of relaxed atomics, because it is the per-chunk hot
  path (no dynamic dispatch, no locks).
- Traits use `async_trait` because the engine stores protocols as `Arc<dyn Protocol>`, and
  native `async fn` in traits is not dyn-compatible.
- Protocols report phases (`RunPhase`) through `RunEvents`. The engine validates them with
  `pv_core::transition`, so protocols cannot put a task into an illegal state.

## Consequences
- Protocol crates can be tested with in-memory fakes (see `pv-core` `protocol::tests`).
- There is one boxed future per trait call. This is negligible: checkpoints run every
  1–2 s, and limiter calls are per chunk (≥ 16 KiB).
- The `CheckpointSink` contract (ADR-005: only positions already `sync_data`'d) is
  documented on the trait, but only the Phase 4 kill tests can enforce it.
