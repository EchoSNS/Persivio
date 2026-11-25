# ADR-028: Browser downloads are taken over earlier, and confirmed

- **Status:** Accepted (changes ADR-021, point 6)
- **Date:** 2026-11-25

## Context
With the browser's "ask where to save each file" on, its save window opened next to
Persivio. The extension acted on `downloads.onCreated`, but the browser does not wait for
that event, and an extension that was asleep (after 30 idle seconds) is too late. Pausing the
download does not stop the window either. Measured in Brave with
`scripts/extension-save-window.mjs`. Persivio also added these downloads without asking.

## Decision
1. **Chromium: `downloads.onDeterminingFilename`.** The browser waits for this event (up to 15
   seconds) *before* it asks where to save. The extension offers the file to Persivio and
   cancels the browser's copy only once Persivio has it. Otherwise the browser goes on as
   usual, with its own window. Firefox has no such event and keeps the previous way.
2. **Persivio asks** (`POST /v1/offers`, 202): name, size, type, site, link, folder, category,
   whether cookies go along, and a warning for programs. The setting "Ask before a download
   from the browser starts" turns it off (then 201, added at once).
3. **The answer does not go back to the browser.** It waits only 15 seconds, a person may take
   longer. So "Cancel" means the file is not downloaded at all.
4. **Secrets stay in Rust**: an offer keeps cookies and headers; the UI gets a view without
   them (and without a user name or password in the link) and answers with an id.
5. **Not answered by accident**: the dialog comes up by itself, so it cannot be answered for
   half a second, and a click next to it does nothing. Escape and Cancel decline.
6. **Limits**: 20 offers wait at most (429), the same link is one offer, names and types are
   cleaned. Nothing is requested from the site before the answer.
7. The window comes to the front for the question and goes back to where it was (tray,
   minimized) after the last answer.
8. **Versions mix**: `/v1/downloads` still works for older extensions (201, also asks), and the
   extension uses it when `/v1/offers` does not exist.

## Consequences
- One extra click per browser download, unless the setting is off.
- Offers live in memory: quitting Persivio with a question open drops that download.
- Persivio requests the file again, as before; a link that works only once fails there.
