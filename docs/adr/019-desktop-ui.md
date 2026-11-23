# ADR-019: Desktop UI building blocks

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
Phase 8 builds the full interface: sidebar, a list for hundreds of tasks, a detail
drawer, add dialog, settings, drag and drop, themes and keyboard use. The plan suggested
shadcn-svelte / Bits UI. The app runs under a strict CSP (ADR-018: `style-src 'self'`,
no inline style attributes).

## Decision
1. **Native `<dialog>` instead of Bits UI.** Bits UI positions and locks scrolling with
   inline `style` attributes, which the CSP blocks. Relaxing the CSP would not help
   either: once Tauri adds hashes to `style-src`, browsers ignore `'unsafe-inline'`.
   The native `<dialog>` gives focus trapping, Escape, the top layer and modal
   semantics with no dependency. Inline positions (progress widths, virtual rows) are
   set through CSSOM (`style:` directives), which CSP allows. Icons: `@lucide/svelte`.
2. **Virtualized list** with `@tanstack/svelte-virtual` and fixed 56 px rows. The task
   array is `$state.raw` and replaced on each change, so Svelte compares references
   instead of deep-proxying hundreds of rows. Virtualizer reads inside effects go
   through `get(store)` (a tracked read there loops forever; a test caught it).
3. **Pure logic, small components.** Filters, selection (file-manager rules), speed
   history, shortcuts and link detection are plain TypeScript modules with unit tests;
   components get data and callbacks.
4. **Input without new permissions.**
   - Ctrl+V uses the `paste` event, which carries the clipboard text, so no clipboard
     permission is needed.
   - Drag and drop is HTML5 (`dragDropEnabled: false`), so links dropped from a browser
     work.
   - `.torrent` files are read in the webview and sent as base64. Rust never receives a
     path it would have to trust.
   - The add dialog previews a `.torrent`'s files (parsed in Rust without network) so a
     selection can be made before anything downloads. Magnet files are chosen in the
     drawer once metadata arrives.
5. **Theme** is a per-machine view preference in `localStorage` (guarded), applied as a
   `dark` class; `system` follows `prefers-color-scheme` live.
6. **Engine additions for the drawer:** torrent file lists are persisted (migration
   0004) and `EngineHandle::detail` returns segments, files and the final URL. The
   drawer polls it once a second only while open on a running task. Categories became
   a validated setting; a task's category is validated too.
7. **Binary names:** the desktop executable is `persivio-desktop`. With `persivio` (the
   CLI's name), both builds wrote `target/release/persivio.exe` (also case-insensitively
   on Windows).

## Consequences
- No ready-made component kit: dialogs, menus and tooltips are written by hand (tooltips
  are `title` attributes for now).
- Notification preferences arrive with notifications in Phase 9.
- Measured (release, Windows): with 500 paused tasks the app used 0.2 s of CPU in 20 s
  idle, and only the visible rows exist in the DOM (component test: fewer than 40 of 500).
