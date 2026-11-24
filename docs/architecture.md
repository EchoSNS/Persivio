# Persivio architecture

Persivio is a desktop download manager: a UI-agnostic Rust engine (multi-connection HTTP and
BitTorrent) driven by a Tauri v2 + Svelte 5 desktop app or a headless CLI. This document is the
map; the *why* behind each choice lives in [`adr/`](adr/).

## Layers

```
┌───────────────────────────────────────────────────────────────────────┐
│ Frontend (Svelte, in Tauri webview)                                    │
│   stores ◄── progress channel / events      commands ──►               │
└───────────────────────────────▲─────────────────────────┬──────────────┘
                                │ IPC (typed via specta)   │
┌───────────────────────────────┴─────────────────────────▼──────────────┐
│ apps/desktop/src-tauri  — thin adapter: commands, event forwarding,     │
│                           tray, plugins, OS integration                 │
└───────────────────────────────▲─────────────────────────┬──────────────┘
                                │ EngineEvent (broadcast)  │ EngineHandle (mpsc)
┌───────────────────────────────┴─────────────────────────▼──────────────┐
│ pv-engine  — actor: queue, scheduler, state machine, limits, recovery  │
│      │                     │                      │                     │
│      ▼                     ▼                      ▼                     │
│ pv-http (Protocol)    pv-torrent (Protocol)   pv-store (SQLite)         │
│  probe/segments/       librqbit adapter        tasks/segments/settings  │
│  writer/retry                                                           │
│                    pv-core: domain types, traits, errors, events        │
└────────────────────────────────────────────────────────────────────────┘
          ▲
          │ also used by
     apps/cli  (headless debug client)       extension/ (browser) ──► local bridge
```

| Layer | Responsibility | Must not |
|---|---|---|
| Frontend | Render state; send commands | Hold authoritative state |
| `src-tauri` | Map IPC ↔ `EngineHandle`, forward events, OS integration | Contain download logic |
| `pv-engine` | Own all task state (actor), schedule, rate-limit, persist, recover | Know about any UI |
| `pv-http` / `pv-torrent` | Implement the `Protocol` trait for one source type | Touch the DB directly |
| `pv-store` | The only code that talks to SQLite | Contain business rules |
| `pv-core` | Types, traits, errors, events, the state-transition function | Do I/O or spawn tasks |

## Crate dependency graph (target)

Dependencies only point downward. They are added to each `Cargo.toml` in the phase that first
uses them (Cargo warns on unused dependencies), but no crate may ever add an edge that breaks
this graph:

```
apps/cli ─┐
apps/desktop/src-tauri ─┴─► pv-engine ─► pv-http ────► pv-core
                                       ├► pv-torrent ─► pv-core   (only crate using librqbit)
                                       ├► pv-store ───► pv-core   (only crate using sqlx)
                                       └► pv-core
pv-testkit: dev-dependency only, never in shipped binaries
```

## Key flows

### Add an HTTP download
1. UI → `add_download` command → `EngineHandle::add(spec)`.
2. Engine persists the task as `Queued`, emits `TaskAdded`.
3. Scheduler acquires a concurrency permit → `Probing`.
4. `pv-http` probes → plans segments → persists segments → `Downloading`.
5. Segment workers stream bytes → the single per-task writer writes at offsets → progress atomics.
6. A 250 ms ticker emits `ProgressBatch`; a 1–2 s checkpointer runs `sync_data` **then** persists
   segment progress (ADR-005).
7. All segments done → optional checksum verify → rename `.part` to the final name → `Completed`.

### Crash recovery
On startup, tasks found in a running state (`Probing`, `Checking`, `Downloading`, …) become `Paused` (or auto-resume
per settings). Segments are loaded from the DB, and each `.part` file's existence and size are
validated before resuming; inconsistent tasks are marked `Failed` with a clear reason.

## Concurrency model in one paragraph
The engine is an **actor**: one task owns the state and processes commands from an `mpsc`
channel, replying via `oneshot`. Events fan out via `broadcast` so any number of front ends can
listen. Each active HTTP task has N async segment workers and exactly **one** blocking writer
thread fed by a **bounded** channel, which gives positional writes without races and caps
memory through backpressure (ADR-004).
