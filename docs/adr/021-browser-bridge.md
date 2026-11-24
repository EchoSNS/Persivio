# ADR-021: Browser extension talks to a localhost bridge

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
Phase 10 sends browser downloads to Persivio, including downloads behind a login (the
browser's cookies must come along). The user tests in Brave (Chromium); Firefox should
work from the same code. Two transports exist.

**Native Messaging:** the browser starts a registered host executable and talks to it
over stdin/stdout with JSON messages.
- No network port, and only the declared extension IDs may connect.
- Requires a host manifest file plus a registry key (Windows) or a JSON file in
  browser-specific folders (macOS/Linux), for each browser separately (Chrome, Brave,
  Edge, Chromium, Firefox each look in different places). Installers must write them;
  unpacked development extensions have IDs that change with the folder.
- The browser starts a *new process* for the host. Since Persivio is single-instance,
  that process would still have to forward each request to the running app over some
  local IPC, adding a second hop and a second program to build and sign.

**Localhost HTTP bridge:** the running app listens on `127.0.0.1:<port>`; the extension
calls it with `fetch`.
- One small server inside the running app, the same for every browser and OS.
- Nothing to register per browser; trivial to test with plain HTTP requests.
- Risk: any local program, and web pages (through the browser), can reach a localhost
  port. This must be closed with authentication and origin checks.

## Decision
Use the **localhost bridge**, opt-in (off by default), hardened as follows:

1. Bound to `127.0.0.1` only (never `0.0.0.0`), port configurable (default 47321).
2. **Host header** must be `127.0.0.1:<port>` or `localhost:<port>`. This blocks DNS
   rebinding, where a web page's own domain resolves to 127.0.0.1.
3. **Origin** must be `chrome-extension://…` or `moz-extension://…`. Web pages cannot set
   these, so a page can neither call the bridge nor pass a CORS preflight.
   *Corrected 2026-11-24:* Chromium sends **no `Origin`** for an extension's GET (recorded
   in Brave: `Sec-Fetch-Site: none`), so "Test connection" was refused. A missing Origin is
   accepted if `Sec-Fetch-Site` is `none` or absent. Browsers set that header on requests
   from pages and pages cannot change it, and the token is still required.
4. **Pairing token**: 256 random bits (OS RNG), shown in Settings, pasted once into the
   extension, sent as `Authorization: Bearer`. Compared in constant time, and can be
   regenerated. This is what stops other local programs.
5. **Limits**: 64 KiB bodies (413), unknown JSON fields rejected (422), `http(s)` URLs
   only, cookies at most 16 KiB with CR/LF rejected (no header injection), file names
   sanitized.
6. Browser downloads are **added directly**, not confirmed in a dialog: the user just
   clicked "download". The cookies stay in Rust, and the redirect rules of ADR-017
   keep them on the download's own site.

The extension (Manifest V3, plain JavaScript, no build step):
- intercepts `downloads.onCreated` for chosen file types or sizes, pauses the browser
  download, reads the URL's cookies (`cookies.getAll`), posts them, and cancels the
  browser's copy only once Persivio accepted it (private windows are left alone);
- offers a link context-menu entry;
- resumes the same browser download if Persivio cannot take it (requesting it again
  would break one-time links).
It talks only to `127.0.0.1`.

## Consequences
- A local program that can read Persivio's settings database (same user) can read the
  token. That user could already read the browser's cookies, so this adds no new risk.
- `<all_urls>` host permission is needed to read cookies for arbitrary download hosts;
  the README explains it.
- Supervisor: enabling, disabling or changing the port restarts the server without
  restarting the app; a busy port is reported in Settings.
- Tests: token, origin and Host rejections, payload limits, and an end-to-end download
  behind a cookie login through the bridge into the engine, plus extension logic tests
  run with `node --test`. The real extension in real Brave: `scripts/extension-e2e.mjs`.
