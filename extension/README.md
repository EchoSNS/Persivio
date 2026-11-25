# Persivio browser extension

Sends downloads from your browser to Persivio, including downloads that need you to be
logged in (the extension passes the site's cookies along). Works in Brave, Chrome, Edge
and other Chromium browsers; Firefox 128+ is supported by the same files.

## Install in Brave (from this folder)

1. In Persivio: **Settings → Browser integration**, tick **Accept downloads from the
   Persivio browser extension**, click **Save**, then **Copy** the pairing token.
2. In Brave, open `brave://extensions`.
3. Turn on **Developer mode** (top right).
4. Click **Load unpacked** and choose this `extension` folder.
5. Click **Details → Extension options** (or right-click the Persivio icon → Options).
6. Paste the token, click **Save**, then **Test connection**. It should say
   "Connected to Persivio 0.5.2."

Brave keeps an unpacked extension until you remove it. After pulling new code, click
the reload icon on the extension's card.

## What it does

- **Downloads you start in the browser**: files of the listed types (zip, iso, exe, mp4,
  …) or at least the chosen size (1 MB by default) move to Persivio. The
  browser's copy is paused, and cancelled only once Persivio accepted it.
- **Right-click a link → Download with Persivio**: always sends that link.
- **If Persivio is not running** (or the token is wrong), the browser simply
  continues its download and a notification explains why. You can turn this fallback off.

Small files and other types stay in the browser, so everyday downloads such as a
receipt are not affected.

## Privacy and security

- It talks **only to Persivio on this computer** (`http://127.0.0.1:<port>`), never to
  the internet.
- Every request carries the pairing token; Persivio refuses requests without it and
  requests from web pages.
- It reads the cookies **of a download's own URL**, only when that download is sent to
  Persivio, so files behind a login work. Persivio sends those cookies only to that site.
- Why it asks for access to all sites: reading the cookies of a download needs access
  to that download's site, and downloads can come from anywhere.
- The token is stored in the browser's local extension storage (not synced).

## Development

`npm test` runs the logic tests (`src/lib.js`) with Node's built-in test runner; no
dependencies. The service worker (`src/background.js`) and options page use only the
`chrome.*` APIs, which Firefox also provides.
