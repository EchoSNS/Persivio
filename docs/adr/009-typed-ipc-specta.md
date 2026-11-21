# ADR-009: Typed IPC via specta / tauri-specta

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
The frontend (TypeScript) and backend (Rust) exchange commands, snapshots and events.
Hand-maintained duplicate type definitions drift silently and cause runtime errors the compiler
cannot catch.

## Decision
Generate TypeScript bindings for all Tauri commands and shared types from Rust with `specta` +
`tauri-specta`, regenerated automatically in debug builds into
`apps/desktop/src/lib/api/bindings.ts`. No hand-written duplicates of these types in TypeScript.
Exact crate versions and API are verified on docs.rs in Phase 7 (as of 2026-11-22 the Tauri v2
integration is on the specta 2 / tauri-specta 2 release-candidate line).

## Consequences
- Rust is the single source of truth; a type change becomes a TypeScript compile error.
- Shared types need specta derives, which pulls `specta` into `mp-core` behind a feature flag.
- We depend on tauri-specta keeping pace with Tauri releases.
