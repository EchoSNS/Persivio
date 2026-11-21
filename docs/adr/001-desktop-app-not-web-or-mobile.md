# ADR-001: Desktop app (Tauri), not a pure web or mobile app

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
A download manager needs raw TCP/UDP sockets (BitTorrent peers, DHT), unrestricted filesystem
writes, cross-origin HTTP to arbitrary servers, and work that continues when no window is open.
Browsers forbid raw sockets, sandbox the filesystem, enforce CORS on arbitrary URLs and stop work
when the tab closes. Mobile OSes heavily restrict long-running background downloads and torrenting.

## Decision
Ship a desktop application built with Tauri v2 (Rust backend, system webview frontend). A headless
engine with a web UI is a possible v2, made cheap by ADR-002.

## Consequences
- Full OS capabilities from Rust; small installers (system webview instead of bundled Chromium).
- We handle per-OS concerns ourselves: packaging, signing, protocol handlers, WebKitGTK quirks on Linux.
- Mobile and browser-only users are out of scope for v1.
