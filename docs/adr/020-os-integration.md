# ADR-020: Operating-system integration

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
Phase 9 makes the desktop app behave like a native one: tray icon, single instance,
`magnet:` links from the browser, `.torrent` file association, notifications, opt-in
clipboard watching, autostart, and preventing sleep while downloading. Several of these
take input from outside the app (browser, file manager, any local program) or change
system-wide state (registry, login items).

## Decision
1. **External input never adds downloads by itself.**
   - Arguments of the first launch or a second launch, and macOS open-URL / open-file
     events, are parsed in Rust (`external.rs`).
   - Only `http(s)` and `magnet:?` links and `.torrent` paths are kept. Files are read
     with the 16 MiB limit.
   - The UI receives them in a queue and opens the Add dialog prefilled, one request at
     a time, for the user to confirm. A web page can trigger `magnet:` links and any
     program can launch Persivio with arguments, so confirmation is the safe default.
2. **Single instance + argument forwarding** with `tauri-plugin-single-instance`; the
   window starts hidden (`visible: false`) and is shown unless `--minimized` (autostart).
3. **`magnet:` handler is opt-in** (`handle_magnet_links`, off by default).
   - Windows and Linux: registered and unregistered at runtime through
     `tauri-plugin-deep-link`.
   - Its `unregister` deletes the whole `magnet` key whoever owns it, so Persivio
     unregisters only when `is_registered` confirms the key points to this executable.
     Another torrent client's handler is never removed.
   - macOS declares the scheme in `Info.plist`; the user picks the default handler.
   - The `.torrent` association is in the bundle configuration (installers).
4. **Sleep prevention** uses `keepawake` 0.6 (`default-features = false`; its `bin`
   feature would add a CLI's dependencies). It wraps `SetThreadExecutionState` on
   Windows, IOKit power assertions on macOS and the D-Bus inhibit API on Linux, keeps
   unsafe code out of Persivio, and is maintained.
   - Windows ties the request to the calling thread, so one dedicated thread owns the
     guard and receives on/off messages (`sleep.rs`).
   - It prevents idle sleep only; the display may still turn off.
   - On by default: a download manager is expected to finish overnight.
   - Alternatives considered: `nosleep` (unmaintained since 2022), and our own FFI per
     OS (unsafe code in three platform variants).
5. **Clipboard watching is opt-in.** No OS offers a portable change event, so the text
   is read once a second only while the setting is on (otherwise the task waits on a
   settings channel). Only text copied after watching starts is considered, and it is
   never logged or stored. Matching links appear as a toast with an "Add" button.
6. **Notifications** on completion and failure (each a setting), only when the window
   is hidden or not focused.
7. **Tray**: tooltip with active count and total speeds (updated at most once a second,
   set to idle when the last download stops); menu Show / Pause all / Resume all / Quit.
   Close-to-tray is on by default; Quit goes through the graceful engine shutdown.
8. **Desktop settings live in `Settings.desktop`** (pv-core), persisted with the engine's
   settings and ignored by the CLI. Saving applies OS-level changes (autostart entry,
   magnet handler) only where they differ, and reports failures without failing the save.

## Consequences
- Clicking a magnet link always needs one more click (Add). This is deliberate.
- With watching enabled, the clipboard is read every second (a few microseconds).
- New licenses: BSL-1.0 (Windows clipboard crates), CC0-1.0 (`tiny-keccak`, a build-time
  dependency of the deep-link plugin).
