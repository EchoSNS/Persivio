# ADR-012: SQLite (WAL) as the single source of truth

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Task lists, segment progress, torrent state and settings must survive crashes and restarts, be
inspectable for debugging, and be updated frequently (checkpoints) without blocking readers.

## Decision
Persist everything in one SQLite database via `sqlx` with migrations, in WAL journal mode with
foreign keys enabled. `pv-store` is the only crate that touches it. In-memory engine state is a
cache rebuilt from the DB at startup.

## Consequences
- Transactions make multi-row checkpoint updates atomic; WAL lets reads proceed during writes.
- One file is easy to back up, inspect with any SQLite tool and attach to bug reports.
- Schema changes require migrations; the DB file lives in the OS app-data directory.

## Amendment (Phase 1, 2026-11-22)
- Connections use `synchronous=NORMAL`. In WAL mode a committed transaction survives an
  application crash, and only an OS crash or power loss can drop the most recent commits.
  Under ADR-005 a lost checkpoint makes progress *lag* the data on disk, which is safe,
  so we skip `FULL`'s extra fsync on every checkpoint.
- Tables are `STRICT`, with `CHECK` constraints (non-negative sizes, `end >= start`,
  `downloaded <= segment length`, and "failed ⇔ fail_reason present") as a last line of
  defence against bugs.
- `u64` values cross into SQLite's `i64` only through checked conversions. Values above
  2^63−1 are rejected, never wrapped.
