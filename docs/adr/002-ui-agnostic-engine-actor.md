# ADR-002: UI-agnostic engine as an actor

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Download state is mutated by many sources: user commands, worker completions, timers, recovery.
Sharing it behind locks across the UI layer couples the engine to Tauri and invites lock-ordering
bugs. We also want a CLI and automated tests that drive the engine without any UI.

## Decision
`pv-engine` runs as an actor: a single task owns all task state. Front ends hold a cloneable
`EngineHandle` that sends commands over an `mpsc` channel (with `oneshot` replies) and subscribe
to `EngineEvent`s via a `broadcast` channel. The engine has no dependency on Tauri.

## Consequences
- State mutation is sequential inside the actor, so there is no lock juggling for task state.
- The same engine serves the CLI, the desktop app, tests and a possible future web UI.
- Slow broadcast subscribers can lag and miss events (`RecvError::Lagged`), so front ends must be
  able to resync from `list()`.
