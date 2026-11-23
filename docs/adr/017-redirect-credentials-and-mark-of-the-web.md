# ADR-017: Redirect handling, credentials and Mark-of-the-Web

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
A security review after Phase 6 found two gaps.

1. **Credentials leaked across hosts.** reqwest followed redirects and removed the
   `Cookie` header on a cross-host hop, but kept custom headers (e.g. `X-Api-Key`).
   Persivio then sent every range request straight to the post-redirect URL with the
   task's cookies and headers attached again. A site that redirects to a CDN on another
   domain handed the user's session cookie to that CDN. Redirects from `https` to
   `http` were also followed, which lets a network attacker replace the file.
2. **No Mark-of-the-Web.** Browsers tag downloaded files on Windows (an NTFS alternate
   data stream `Zone.Identifier` with `ZoneId=3`), so SmartScreen warns before an
   unknown program runs and Office opens documents in Protected View. Files downloaded
   by Persivio carried no such tag, so they skipped those warnings.

## Decision
1. **Persivio follows redirects itself** (`HttpClient::send`, reqwest's policy is
   `none`). Each hop is checked: `http`/`https` only, no `https` → `http` downgrade,
   at most 10 hops. Cookies and custom headers are sent only while the URL has the
   same origin (scheme, host, port) as the task's URL, both while following redirects
   and for later range requests to the final URL. The `Referer` is always sent: hosts
   that check it for hotlink protection are usually the CDN.
2. **Mark-of-the-Web on Windows.** `pv_core::motw` builds the stream text (pure); the
   protocols write it when files are complete: HTTP after the final rename, torrents
   when every wanted piece is on disk (before seeding). `HostUrl` is the origin without
   credentials, query or fragment (any local program can read the stream, and signed
   URLs carry tokens). Torrents use `about:internet`. Failures (FAT32, some network
   shares) are logged, never fatal. The torrent walk does not follow symlinks.

## Consequences
- A download whose server redirects `https` → `http` now fails with a clear message.
  Chrome also blocks this case; an explicit opt-in can be added if users need it.
- Authenticated downloads that redirect to another host work only if the target needs
  no cookies, which is how signed CDN URLs are designed.
- macOS (`com.apple.quarantine` extended attribute) and Linux have no equivalent yet;
  macOS can be added in the hardening phase.
- Cookies and headers are still stored in the database while a task exists (needed
  for resume). Clearing them after completion is a hardening-phase item.
