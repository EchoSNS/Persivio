# ADR-026: Release engineering and diagnostics

- **Status:** Accepted
- **Date:** 2026-11-24

## Decision
1. **Tag builds a draft.** `release.yml` builds Windows (NSIS, MSI), macOS (universal DMG)
   and Linux (AppImage, deb) on a tag and uploads them to a draft release. A person
   publishes it. Started by hand it builds the same files as artifacts, without a release.
2. **Consistency is checked first** (`scripts/release-check.mjs`, also in CI): tag,
   versions, changelog entry, and that a configured signing key is not the local one.
3. **Two update keys.** The local channel keeps its own key in `tauri.local.conf.json`, so
   the online key in `tauri.conf.json` can be replaced without breaking local updates.
4. **`requireSignedVersion`**: the signature covers the version, so an old signed installer
   cannot be announced as a new one (a downgrade to a version with known flaws).
5. **Supply chain**: in the release workflow, actions are pinned to commits and no cache is
   used. Each job gets only the permissions it needs.
6. **Magnet links stay opt-in** (ADR-020): installers do not register the scheme, because
   that would silently replace the user's torrent client. `.torrent` files are offered
   through the normal file association.
7. **Update test** (`tests/updates.rs`): the real updater plugin with the shipped
   configuration finds a newer version and refuses a changed package, another key, another
   version's package and a signature without a version. Running an installer is covered by
   the manual tests.
8. **Diagnostics** are a plain text file the user can read before sharing. Settings are
   copied field by field, downloads are only counted, and log text is redacted (links,
   magnets, paths, addresses, hashes, e-mail addresses, quoted text). Redaction removes too
   much rather than too little. Saving uses the system dialog from Rust, so the webview
   still has no file access.
9. **Fuzzing** moved to its own workflow: weekly, by hand, and when a parser changes. The
   repository is private and its build minutes are limited.
10. **`.torrent` bytes are checked before librqbit parses them** (`bencode::check`). Found
    by fuzzing: numbers overflow in its parser, which panics in builds with overflow checks
    and wraps in release builds, and a piece length of zero, which it divides by (a panic in
    every build). Runs also get their own task, so a panic in a library fails that download
    instead of leaving it running forever.

## Consequences
- Until a release key exists, releases contain installers but no updates.
- Metadata that arrives from peers (magnet links) is parsed inside librqbit and does not
  pass `bencode::check`. Its nesting is limited there; a panic fails only that download.
- Installers are not code-signed, so systems warn. `docs/release.md` explains how to add it.
