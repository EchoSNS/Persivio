# Changelog

What changed in each version of Persivio. The app shows the new entries once after an
update (Settings → Updates → What's new shows them all).

Format: one `## [version] - date` heading per release, then `### Added`, `### Changed`,
`### Fixed` or `### Security` sections with one line per change, written for users.

## [0.6.0] - 2026-11-25

### Security
- A crafted magnet link can no longer use up the memory and close Persivio.

### Changed
- A calmer window: one continuous surface instead of panels, and on Windows its own title bar.
- The Windows installer looks like the app, and updates without asking to uninstall first.
- Total speeds moved to the foot of the sidebar.

### Fixed
- Windows keeps showing the old icon in the taskbar after an update.
- A malformed magnet link is refused with a clear message.
- Magnet links with a lower-case base32 hash, or starting with `MAGNET:`, work.

## [0.5.1] - 2026-11-25

### Changed
- A logo: a persimmon with a download arrow.
- Dropdown lists match the rest of the interface, with a check mark on the chosen entry.

### Added
- A user guide, linked from the project page.

## [0.5.0] - 2026-11-25

### Added
- Online updates: new versions come from the public releases page, signed with a new key.
- The installers include the license notices of the software inside.

### Changed
- A copy installed from a local build must be replaced once by hand with the online
  installer. Your downloads and settings stay.

## [0.4.0] - 2026-11-24

### Added
- Settings → Diagnostics: a report without personal data, and the logs folder.

### Fixed
- Browser extension: "Test connection" failed with "not from this extension" in Brave and
  Chrome (downloads were not affected).
- A crash inside one download no longer leaves it running forever; it fails and can be
  retried.

### Security
- .torrent files with impossible numbers or nesting are refused before they are parsed;
  a crafted file could crash the app.
- Updates: an older signed installer cannot be passed off as a newer version.

### Changed
- The publisher shown by Windows is Yvan Sinues.
- The log keeps more history (up to 10 MB) and no longer fills with tracker warnings.

## [0.3.0] - 2026-11-24

### Added
- New glass look for light and dark: translucent surfaces that the list scrolls under.
  Settings → Appearance has a switch for solid surfaces.
- Right-click menu on downloads, sorting, and a bar with the total speeds.
- "Checking files" state for torrents, with its own progress.

### Changed
- Resuming a paused torrent continues at once instead of reading all its data again.
  After a crash, or if the files changed, it is still checked completely.
- With no torrent running, Persivio leaves the BitTorrent network after 30 seconds: no
  open port and no background traffic.
- The global speed limit now also applies to torrents that are already running.
- Downloads are listed newest first; the Save button in Settings stays visible.

### Security
- Cookies, headers and logins of a download are erased from disk when it completes or is
  cancelled, also those stored by earlier versions.
- On Linux and macOS, the data folder is readable by you only.

### Fixed
- A full disk is reported clearly, and the download can be retried after making room.
- Automatic retries now also cover connecting, and servers without resume support.
- A server that never answers no longer blocks a download: it can be paused, and times out.
- A torrent with few peers no longer waits up to an hour to try them again.
- A download whose state could not be saved no longer looks as if it were still running.
- A resumed torrent looked as if it downloaded very fast and then started over. The fast
  part was the check of existing data.
- Torrents showed 0% after a restart until they ran again.
- With the details panel open, file names could disappear from the list.
- Adding a torrent that is already running is refused instead of both using the same
  files.

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
