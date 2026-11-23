# ADR-018: Desktop app IPC: UI types, generated bindings, least privilege

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
Phase 7 adds a Tauri v2 + Svelte 5 desktop app over `pv-engine`. The webview is the
least trusted part of the app: it renders names chosen by servers and torrent authors,
so a rendering bug there must not be able to read secrets or touch arbitrary files. The
UI also needs TypeScript types that cannot drift from the Rust ones.

## Decision
1. **Separate UI types** (`apps/desktop/src-tauri/src/dto.rs`) instead of exposing
   `pv-core` types. A `Source` holds cookies, custom headers, a URL password or
   `.torrent` bytes; `TaskView` carries only a display string (URL without credentials,
   or the magnet link). A test serializes a task full of secrets and checks that none
   reach the JSON. It also keeps `pv-core` free of the pre-release `specta` crate.
2. **Generated bindings.** `tauri-specta` 2.0.0-rc.29 with `specta` pinned `=2.0.0-rc.28`
   (its documentation requires exact pins during the release-candidate phase) writes
   `src/lib/api/bindings.ts` on every debug run. A Rust test fails when the committed file
   is stale. One TypeScript type per Rust type (`disable_serde_phases`).
3. **64-bit integers as numbers.** Sizes and speeds are `u64` and exported as `number`
   (`dangerously_cast_bigints_to_number`): exact up to 2^53 bytes (8 PiB), far above any
   file or speed.
4. **Least privilege.**
   - `build.rs` lists every app command, so Tauri denies any command the capability
     file does not grant.
   - The main window may: call those commands, listen to events, open the folder
     picker, write log lines. No file-system, shell or opener permission.
   - "Show in folder" is a Rust command taking a task id; the path is resolved and
     contained in Rust. The UI never sends a path, and nothing is opened or executed.
   - A strict CSP (`script-src 'self'`, `connect-src ipc:`, no frames/objects) and
     `freezePrototype`. Dev mode allows inline styles and the Vite websocket only.
5. **Event delivery.**
   - Progress goes through a Tauri `Channel` per UI subscription, coalesced to at most
     one message per 250 ms. The newest batch replaces a pending one; progress is
     absolute, so drops are harmless.
   - Other engine events are typed app events. A lagging receiver sends `resync`, and
     the UI reloads the list.
   - On load the UI listens first, then lists, then applies buffered events, so nothing
     is lost in between.
6. **Startup and shutdown.**
   - The app uses the same data directory and lock as the CLI (`pv_engine::DataDir`).
     If the CLI holds it, the app still opens and explains why the engine is not running.
   - `PERSIVIO_DATA_DIR` overrides the directory.
   - On exit the engine shuts down gracefully before the process ends.
7. **Idle cost.** The engine's progress timer runs only while a task is running. The
   forwarders wake only on engine events, and the UI has no polling.

## Consequences
- Each new command must be added in three places: the command, `build.rs`, and
  `capabilities/main.json`. Forgetting one fails loudly (the command is denied).
- tauri-specta upgrades within the release-candidate line may change generated code; the
  pins make that an explicit decision.
- Measured (release build, Windows, empty queue, after startup): the app process used
  0.000 s of CPU in 30 s; the webview processes used 0.3% of one core.
