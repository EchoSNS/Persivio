# Installing Persivio and updating it

How to run Persivio as an installed app on this computer while developing it, how to
deliver updates locally, and how to move to online updates later.

## How updates work

1. A new version is built into a **signed installer**. The signature is made with a
   private key that never leaves this computer.
2. A small **manifest** (`latest.json`) says which version is newest, where its
   installer is, its signature and its release notes.
3. The installed app reads the manifest (at startup, or via **Settings → Updates → Check
   now**). If the version is newer, it shows the notes and an **Install and restart**
   button.
4. On install, the app downloads the installer and **verifies the signature against the
   public key built into the app**. An installer that was tampered with or signed by
   someone else is refused. Only then are running downloads stopped (they resume
   later), the installer runs, and the app restarts.
5. After the restart, **What's new** shows the `CHANGELOG.md` entries you have not seen
   yet.

Where the manifest is read from depends on the build:

| Build | Manifest URL | Configured in |
|---|---|---|
| Local channel (now) | `http://127.0.0.1:8787/latest.json` | `apps/desktop/src-tauri/tauri.local.conf.json` |
| Online (later) | `https://github.com/EchoSNS/Persivio/releases/latest/download/latest.json` | `apps/desktop/src-tauri/tauri.conf.json` |

Plain `http://` is acceptable for the local channel because it never leaves this
computer and the signature check still protects the installer. Online builds use
HTTPS.

## One-time setup (done)

- **Signing key:** `C:\Users\yvans\.tauri\persivio-local.key` (private, no password) and
  `persivio-local.key.pub` (public). The public key is in `tauri.conf.json`.
  - **Never commit the private key, or copy it into the repository.** Anyone who has it
    can sign "updates" that installed copies will accept.
  - Back it up somewhere safe (e.g. a password manager). If it is lost, installed copies
    cannot be updated and must be reinstalled by hand.
- Tools already used by the project: Node.js, Rust, and `npm ci` in `apps/desktop`.

## Install Persivio on this computer (first time)

```powershell
cd C:\DEV\millipede
powershell -ExecutionPolicy Bypass -File scripts\release-local.ps1
```

This builds `dist-updates\<version>\Persivio_<version>_x64-setup.exe`. Run that file.
It installs for your user only (no administrator rights), adds Start-menu entries, and
registers `.torrent` files.

- Windows SmartScreen will say "Windows protected your PC" because the installer is not
  code-signed yet (see [Online updates](#online-updates-later)). Click **More info →
  Run anyway**.
- The installed app and `npm run tauri dev` share the same data folder and lock, so only
  one can run at a time. Quit the installed app (tray → Quit) before developing, or
  start the dev build with a separate folder:
  `$env:PERSIVIO_DATA_DIR = "$env:TEMP\persivio-dev"; npm run tauri dev`.

## Release a new version locally

1. **Bump the version** in `Cargo.toml` under `[workspace.package]` (e.g. `0.2.0` →
   `0.2.1`). The app, CLI and installer all read it from there. Also bump
   `extension/manifest.json` if the extension changed.
2. **Add the update log entry** at the top of `CHANGELOG.md`:

   ```markdown
   ## [0.2.1] - 2026-12-01

   ### Fixed
   - What changed, in words a user understands.
   ```

   A test fails if the newest changelog entry does not match the version, so this
   cannot be forgotten.
3. **Build and publish** to the local channel:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\release-local.ps1
   ```

4. **Serve** the updates (leave it running in its own terminal):

   ```powershell
   node scripts\serve-updates.mjs
   ```

5. In the installed Persivio: **Settings → Updates → Check now** (or restart it; it
   checks at startup). Click **View → Install and restart**. After the restart, **What's
   new** lists the changes.

The server only answers on `127.0.0.1`, serves files from `dist-updates/` only, and can
be stopped when you are not updating. `dist-updates/` is ignored by git.

## Online updates (later)

When Persivio should update for other people (or on other computers):

1. **Create a production signing key** with a password, and store both as GitHub
   repository secrets (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`):

   ```powershell
   cd apps\desktop
   npx tauri signer generate -w $env:USERPROFILE\.tauri\persivio-release.key
   ```

   Put the new `.pub` contents in `tauri.conf.json` → `plugins.updater.pubkey`. Copies
   installed with the local key must be reinstalled once by hand (they trust the old
   key only). That is fine while only this computer uses Persivio.
2. **Release workflow** (Phase 12): a GitHub Actions workflow on tag push (`v0.3.0`) runs
   `tauri-action`, which builds installers for Windows, macOS and Linux, signs them with
   the secrets, creates a GitHub Release with the changelog entry as notes, and uploads
   `latest.json`. The endpoint in `tauri.conf.json` already points at it.
3. **Code signing** (removes the SmartScreen and macOS Gatekeeper warnings): a Windows
   code-signing certificate (or Azure Trusted Signing) and an Apple Developer ID with
   notarization. They are separate from the update signature, and Phase 12 documents
   them in `docs/release.md`.
4. Optionally keep the local channel for testing builds before publishing them.

## Troubleshooting

| Problem | Fix |
|---|---|
| "Could not check for updates" | Is `serve-updates.mjs` running? Open http://127.0.0.1:8787/latest.json in a browser. |
| Update found but install fails with a signature error | The installer was built with a different key than the installed app trusts. Rebuild with the same key, or reinstall by hand. |
| "Persivio is up to date" after a release | The version in `Cargo.toml` was not increased, or `latest.json` was not regenerated. |
| The script stops: "CHANGELOG.md has no entry" | Add the `## [version] - date` entry (step 2). |
| The installed app does not start, "another Persivio instance" | `npm run tauri dev` or the CLI is using the data folder. |
