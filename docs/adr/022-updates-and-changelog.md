# ADR-022: Signed updates, local update channel, built-in changelog

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
The user wants Persivio installed on this computer during development, updated from a
local source for now and online later, and every update's changes shown after it is
installed. Phase 12 (release engineering) will publish online; this ADR covers what is
needed earlier.

## Decision
1. **`tauri-plugin-updater`** (rustls, no OpenSSL), driven from Rust commands:
   `check_for_update`, then `install_update`. The frontend gets no updater permission.
2. **Signatures are mandatory.** The public key is in `tauri.conf.json`. The private key
   lives outside the repository (`%USERPROFILE%\.tauri\persivio-local.key`, no password,
   local channel only). Online releases will get a separate password-protected key in
   CI secrets, and local installs are then reinstalled once.
3. **Endpoints per build.** `tauri.conf.json` points at GitHub Releases over HTTPS.
   `tauri.local.conf.json` (merged by `scripts/release-local.ps1`) points at
   `http://127.0.0.1:8787/latest.json` and enables `dangerousInsecureTransportProtocol`.
   This is acceptable only because the URL is loopback and the signature check still
   applies. It also turns on updater artifacts (needs the key at build time) for NSIS.
4. **Install order:** download and verify first. Only then shut the engine down
   gracefully (downloads checkpoint and resume), install, and restart. A failed download
   leaves everything running.
5. **One version source:** `[workspace.package] version` in `Cargo.toml`;
   `tauri.conf.json` no longer has its own `version`.
6. **Changelog:** `CHANGELOG.md` (Keep a Changelog style) is embedded at compile time
   and parsed into sections. After an update, the app shows the entries newer than the
   last version seen (stored per machine), or only the current one on first run. A test
   fails if the newest entry is not the crate version, so a release cannot ship without
   its notes. Notes are rendered as plain text, never HTML.
7. Startup checks run in release builds only (setting, default on); errors are logged,
   not shown, so an offline computer is not nagged.

## Consequences
- Each local release: bump the version, write the changelog entry, run
  `release-local.ps1`, run `serve-updates.mjs`, then "Check now" (see `docs/updates.md`).
- The installer is not code-signed yet, so SmartScreen warns on first install. Updates
  install silently ("passive" mode) because the app verified the signature itself.
