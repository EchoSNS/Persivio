# Persivio user guide

## Install

Download the file for your system from the
[releases page](https://github.com/EchoSNS/persivio-releases/releases/latest).

| System | File | Note |
|---|---|---|
| Windows 10, 11 | `…_x64-setup.exe` | No administrator rights needed. "Windows protected your PC": **More info → Run anyway**. The installer is not code-signed yet. |
| Windows, managed PCs | `…_x64_en-US.msi` | For IT departments: for all accounts, needs administrator rights. The same app; do not install both. |
| macOS 11 or newer | `…_universal.dmg` | The app is not notarized yet: after the first try, allow it under System Settings → Privacy & Security. |
| Linux | `….AppImage` or `….deb` | Make the AppImage executable first. |

To be sure a file is genuine, compare it with `SHA256SUMS.txt` from the same release. In
PowerShell: `Get-FileHash .\file`.

## Add downloads

- **Paste** a link anywhere in the window (Ctrl+V), **drop** links or `.torrent` files on it,
  or click **Add** (Ctrl+N). Several links at once work, one per line.
- **From the browser**: see below.
- **Magnet links**: turn on Settings → Desktop → *Open magnet links with Persivio*.

## What the states mean

| State | Meaning |
|---|---|
| Queued | Waits for a free slot (Settings → General). |
| Connecting, Metadata | Asks the server, or other participants, for name and size. |
| Checking files | Reads the data you already have to make sure it is intact. Not downloading. |
| Downloading | Data arrives. |
| Verifying | Compares the finished file with the checksum you gave. |
| Seeding | A finished torrent is shared with others, up to your limit. |
| Failed | The reason is shown. **Retry** continues where possible. |

Right-click a download for everything you can do with it; double-click for details.

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+V, Ctrl+N | Add pasted links, new download |
| ↑ ↓ Home End, with Shift to extend | Select |
| Space | Pause or resume |
| Enter, Esc | Details, close |
| Delete | Remove |
| Ctrl+F, Ctrl+A, Ctrl+, | Search, select all, settings |

## Browser extension

1. In Persivio: Settings → **Browser integration** → turn it on, **Save**, **Copy** the token.
2. Unpack `Persivio-extension_….zip` into a folder you keep.
3. In Brave or Chrome open `brave://extensions` or `chrome://extensions`, turn on **Developer
   mode**, click **Load unpacked** and choose the folder.
4. Open the extension's options, paste the token, **Save**, **Test connection**.

Persivio then asks before each download from the browser: file, size, site and folder. The
browser's own download does not start. **Cancel** means the file is not downloaded at all. To
skip the question: Settings → **Browser integration**. If Persivio is not running, the browser
downloads as usual.

After an update, replace the files in that folder and click the reload icon on the
extension's card.

Downloads behind a login work, because the extension passes the cookies of that one link to
Persivio on your computer. They are erased when the download ends.

## Torrents and privacy

Other participants of a torrent see your IP address, and by default you upload to them too.
Settings → **BitTorrent** has *Never seed*, seeding limits, DHT and UPnP. Security software
may report blocked addresses while a torrent runs: participants are strangers, and some of
their addresses are on blocklists. With no torrent running, Persivio leaves the network
after 30 seconds.

## Updates

Persivio checks when it starts (Settings → **Updates**) and asks before installing. Every
update is signed and verified. Running downloads stop safely and continue afterwards.

## Problems

- **A download fails**: the reason is in its details. Links that need a login usually need
  the browser extension.
- **Something else**: Settings → **Diagnostics** → *View report*. It contains no links, file
  names or addresses. Read it, then attach it to an issue.
- **The engine is not running**: another copy, or the command-line version, uses the same
  data folder. Close it and start again.
- **Effects are slow**: Settings → Appearance → turn *Glass effects* off.

Your list and settings are in `%APPDATA%\Persivio` (Windows), `~/Library/Application
Support/Persivio` (macOS) or `~/.local/share/persivio` (Linux). Removing the app keeps them.
