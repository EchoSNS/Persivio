# Changelog

What changed in each version of Persivio. The app shows the new entries once after an
update (Settings → Updates → What's new shows them all).

Format: one `## [version] - date` heading per release, then `### Added`, `### Changed`,
`### Fixed` or `### Security` sections with one line per change, written for users.

## [0.2.0] - 2026-11-23

### Added
- Browser integration for Brave (and other Chromium browsers): the Persivio extension
  sends downloads to the app, including downloads behind a login.
- "Download with Persivio" in the browser's right-click menu for links.
- Settings → Browser integration: turn the bridge on, copy the pairing token.
- Update log: this list, shown once after each update.
- Updates: Persivio checks for new versions and installs them after you confirm.

### Security
- The browser bridge listens only on this computer (127.0.0.1) and accepts requests
  only from the browser extension with the right pairing token.
- Updated the `cookie` library for RUSTSEC-2026-0594.

### Fixed
- Very long file names starting with a reserved Windows name (like `CON`) could end up
  one character too long.

## [0.1.0] - 2026-11-23

### Added
- Fast HTTP downloads over several connections, with pause, resume and recovery after
  a crash or restart.
- BitTorrent downloads from magnet links and .torrent files, with file selection and
  seeding limits.
- Download queue, speed limits, categories and checksum verification.
- Desktop app: task list for hundreds of downloads, details panel, add dialog,
  settings, light and dark themes, keyboard shortcuts, drag and drop.
- Tray icon, magnet links from the browser, .torrent file association, notifications,
  optional clipboard watching, start with the system, and keeping the computer awake
  while downloading.

### Security
- File names from servers and torrents are cleaned and kept inside the download folder.
- Cookies and login headers are sent only to the site they belong to.
- Downloaded files are marked as coming from the internet on Windows.
