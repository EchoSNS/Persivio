# ADR-012: SQLite (WAL) as the single source of truth

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Task lists, segment progress, torrent state and settings must survive crashes and restarts, be
inspectable for debugging, and be updated frequently (checkpoints) without blocking readers.

## Decision
Persist everything in one SQLite database via `sqlx` with migrations, in WAL journal mode with
foreign keys enabled. `mp-store` is the only crate that touches it. In-memory engine state is a
cache rebuilt from the DB at startup.

## Consequences
- Transactions make multi-row checkpoint updates atomic; WAL lets reads proceed during writes.
- One file is easy to back up, inspect with any SQLite tool and attach to bug reports.
- Schema changes require migrations; the DB file lives in the OS app-data directory.
