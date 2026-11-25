# Manual tests

Things automated tests cannot cover: OS shell integration, real browsers and real
networks. Run them before a release, on each OS listed. Use a separate data folder so
your real queue is untouched:

```powershell
# Windows (PowerShell)
$env:PERSIVIO_DATA_DIR = "$env:TEMP\persivio-manual"
```
```sh
# macOS / Linux
export PERSIVIO_DATA_DIR=/tmp/persivio-manual
```

Build the app first: `cd apps/desktop && npm run tauri dev` (or `npx tauri build` for an
installer; file associations and macOS URL schemes only exist in installed builds).

Record results in the table at the end (date, OS, version, pass/fail, notes).

## 1. Tray and window

| # | Steps | Expected |
|---|---|---|
| 1.1 | Start the app. | Window opens; a Persivio tray icon appears (macOS: menu bar). |
| 1.2 | Hover the tray icon with nothing downloading. | Tooltip "Persivio: idle" (Linux trays may show no tooltip). |
| 1.3 | Start a large download, hover the icon. | "Persivio: 1 active" and the download speed, updated about once a second. |
| 1.4 | Close the window (X). | Window hides; the app keeps downloading; the tray icon stays. |
| 1.5 | Left-click the tray icon. | Window comes back. |
| 1.6 | Tray menu → Pause all, then Resume all. | Downloads pause, then resume. |
| 1.7 | Settings → Desktop → untick "Keep running in the system tray…", save, close the window. | The app quits (gracefully: resumes later where it stopped). |
| 1.8 | Tray menu → Quit while downloading. | App exits within a few seconds; on restart the download is paused with its progress kept. |
| 1.9 | Windows: drag the top strip, double-click it, drag an edge; the same with a dialog open. | The window moves, maximizes, resizes. |
| 1.10 | Windows: update from a version with another icon. | The taskbar shows the new icon. |

## 2. Single instance and magnet links

| # | Steps | Expected |
|---|---|---|
| 2.1 | With the app running, start it again (Start menu / Dock / terminal). | No second window; the existing window comes to the front. |
| 2.2 | Settings → Desktop → tick "Open magnet links with Persivio", save. | Saved without a warning (Windows, Linux). |
| 2.3 | In Brave (or any browser), click a magnet link, e.g. on the official torrent page of a Linux distribution. Allow the browser's "Open Persivio?" prompt. | The running Persivio comes to the front with the Add dialog prefilled with the link. Nothing downloads until **Add** is clicked. |
| 2.4 | Close Persivio completely (tray → Quit) and click the magnet link again. | Persivio starts and shows the same prefilled dialog. |
| 2.5 | Untick the setting and save. Click a magnet link. | The browser no longer offers Persivio (another torrent client may open, if installed). |
| 2.6 | If another torrent client is the magnet handler and the setting is **off**, save settings. | The other client still opens magnet links (Persivio removes only its own registration). |
| 2.7 | macOS: install the app, set Persivio as the magnet handler (e.g. through another app's preference or a tool like `duti`), click a magnet link. | Same as 2.3. |

## 3. .torrent files

| # | Steps | Expected |
|---|---|---|
| 3.1 | Installed build: double-click a `.torrent` file (or "Open with → Persivio"). | Add dialog with the torrent's name, size and file list; files can be unticked. |
| 3.2 | Drag a `.torrent` file onto the window. | Same as 3.1. |
| 3.3 | Drag a link from Brave onto the window. | Add dialog with the link. |
| 3.4 | Open two `.torrent` files at once. | Two dialogs, one after the other. |

## 4. Notifications

| # | Steps | Expected |
|---|---|---|
| 4.1 | Start a small download, switch to another app before it finishes. | "Download complete: <name>" notification. |
| 4.2 | Same, with the Persivio window in front. | No notification (the list shows it). |
| 4.3 | Add `https://127.0.0.1:9/x` (nothing listens there), switch away. | "Download failed: …" notification after the retries. |
| 4.4 | Untick both notification settings and repeat 4.1. | No notification. |
| 4.5 | First notification on macOS/Windows. | The OS may ask for permission; allowing it shows the notification. |

## 5. Clipboard

| # | Steps | Expected |
|---|---|---|
| 5.1 | Setting off (default). Copy a link. | Nothing happens. |
| 5.2 | Tick "Offer to add links I copy", save. Copy `https://example.com/file.iso` in Brave. | Within about a second, a toast "Copied: https://example.com/file.iso" with **Add**. |
| 5.3 | Click **Add**. | Add dialog with the link. |
| 5.4 | Copy plain text, or the same link again. | No toast. |
| 5.5 | Ctrl+V in the window (any setting). | Add dialog with the pasted links. |

## 6. Autostart

| # | Steps | Expected |
|---|---|---|
| 6.1 | Tick "Start Persivio when I sign in", save. | Windows: Settings → Apps → Startup lists Persivio. macOS: Login Items. Linux: `~/.config/autostart/` has an entry. |
| 6.2 | Sign out and in. | Persivio starts in the tray, window hidden. |
| 6.3 | Untick and save. | The entry is removed. |

## 7. Sleep prevention

| # | Steps | Expected |
|---|---|---|
| 7.1 | Start a long download. Windows: `powercfg /requests` (admin terminal). macOS: `pmset -g assertions`. Linux: `systemd-inhibit --list`. | A request from Persivio ("Downloads in progress"). |
| 7.2 | Pause all downloads, check again. | The request is gone. |
| 7.3 | Untick "Keep the computer awake…" while downloading, save, check again. | The request is gone immediately. |

## 8. Interface (Phase 8)

| # | Steps | Expected |
|---|---|---|
| 8.1 | Create 500 paused tasks with the CLI (below), start the app with the same `PERSIVIO_DATA_DIR`, scroll quickly. | Smooth scrolling, no blank rows. |
| 8.2 | Settings → Appearance → Light, Dark, System. | All screens readable in each; System follows the OS setting live. |
| 8.3 | Use only the keyboard: Tab, arrows, Space, Enter, Delete, Ctrl+A, Ctrl+N, Esc. | Every core action is possible; focus is always visible. |

| 8.4 | Right-click a download; use the arrow keys, Enter and Escape in the menu. | Actions match the task's state; Escape returns to the list. |
| 8.5 | Settings → Appearance → Glass effects off and on; Windows Settings → Personalization → Colors → Transparency effects off. | Solid surfaces at once; both readable. |
| 8.6 | Pause a large torrent and resume it; quit and start the app and resume again. | "Checking files" for a moment only, then it continues where it was. No fast bar that starts over. |
| 8.7 | With no torrent running, wait a minute and look at Malwarebytes or Resource Monitor. | No torrent connections from Persivio. |

To create 500 paused tasks (PowerShell, with `PERSIVIO_DATA_DIR` set as above):

```powershell
$urls = 1..500 | ForEach-Object { "http://127.0.0.1:9/sample-file-$_.iso" }
cargo run --release -p persivio-cli -- add @urls --dir "$env:TEMP\persivio-manual-dl" --no-wait
cargo run --release -p persivio-cli -- pause all
```

## 9. Browser extension (Brave)

Setup: `extension/README.md` (Load unpacked, paste the token).

| # | Steps | Expected |
|---|---|---|
| 9.1 | Options → **Test connection** with Persivio running and browser integration on. | "Connected to Persivio x.y.z." |
| 9.2 | Change one character of the token, save, test again. | "Wrong pairing token…" |
| 9.3 | Turn browser integration off in Persivio, test again. | "Persivio is not reachable…" |
| 9.4 | Download a public ISO or zip from a site. | Brave's download disappears; Persivio shows the task. |
| 9.5 | **Log in** to a site that serves a private file (e.g. an attachment in your webmail or cloud storage), download it. | The file downloads in Persivio and opens correctly (not a login page). |
| 9.6 | Quit Persivio, download a zip in Brave. | Brave downloads it itself; a notification explains why. |
| 9.7 | Right-click a link → **Download with Persivio**. | Added in Persivio. |
| 9.8 | Download a small image (< 1 MB, not in the list). | Stays in Brave. |
| 9.9 | Open http://127.0.0.1:47321/v1/ping in a Brave tab. | "only the Persivio browser extension may call this" (web pages are refused). |

## 10. Updates

Setup: `docs/updates.md`.

| # | Steps | Expected |
|---|---|---|
| 10.1 | Install version N from `dist-updates/`. | Installs; on first start, What's new shows version N. |
| 10.2 | Bump to N+1, add a changelog entry, run `release-local.ps1`, run `serve-updates.mjs`, Settings → Updates → **Check now**. | "Update available" with the notes. |
| 10.3 | **Install and restart** while a download runs. | Progress, restart; the download resumes; What's new shows only N+1. |
| 10.4 | Stop the server, Check now. | A clear "could not check" message; nothing breaks. |
| 10.5 | Edit a byte of the `.exe` in `dist-updates/`, check and install. | Install refused (signature); the app keeps running. |

## Results

| Date | OS / version | App version | Sections | Result | Notes |
|---|---|---|---|---|---|
| 2026-11-23 | Windows 11 Pro | 0.1.0 (dev) | 2.1, 2.3 (simulated: second launch with a magnet argument), 1.4, 1.5 (relaunch shows the window), autostart `--minimized` start | Pass | Registry registration (2.2) and a real browser click are left to the user. |
| 2026-11-24 | Windows 11 Pro, Brave | 0.4.0 (dev) | 9.1, 9.2, 9.3, 9.4, 9.5, 9.9 (scripted: `scripts/extension-e2e.mjs`) | Pass | Test connection failed in 0.3.0 and was fixed. |
| 2026-11-24 | Windows 11 Pro | 0.2.0 to 0.3.0 | 10.2, 10.3 (by the user, local channel) | Pass | 10.5 (tampering) is automated in `tests/updates.rs`. |
