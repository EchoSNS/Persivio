# Releasing Persivio online

How to publish installers and online updates with GitHub Actions. For the local channel on
one computer, see [updates.md](updates.md).

## What a release does

Pushing a tag `vX.Y.Z` starts `.github/workflows/release.yml`:

1. **prepare** checks that the tag, `Cargo.toml`, `package.json`, the extension and
   `CHANGELOG.md` agree, and creates a **draft** release with the changelog entry.
2. **build** makes the installers and uploads them to the draft:

   | System | Files |
   |---|---|
   | Windows | `*-setup.exe` (NSIS, for one user) and `.msi` |
   | macOS 11+ | universal `.dmg` (Apple silicon and Intel) |
   | Linux | `.AppImage` and `.deb` (built on Ubuntu 22.04) |

   With a signing key, also the update packages, their `.sig` files and `latest.json`.
3. **finish** adds the browser extension and `SHA256SUMS.txt`.

**Nothing is public until you publish the draft** on the Releases page. Installed copies
see the update from that moment.

To try the builds without a release: Actions → Release → **Run workflow**. The installers
appear as artifacts of the run.

## One-time setup

### 0. Decide where releases live

The repository is **private**, and installed copies have no GitHub login, so they cannot
read `releases/latest/download/latest.json`. Choose one:

| Option | Effect |
|---|---|
| Make the repository public | Simplest: everything works as configured. The code becomes public. |
| A second, public repository for releases only | The code stays private. `release.yml` needs a token for that repository, and the endpoint in `tauri.conf.json` changes. |
| Keep the local channel | Nothing to do; updates work on this computer only. |

### 1. Create the release signing key

The local key has no password and must not sign public releases (the workflow refuses it).

```powershell
cd apps\desktop
npx tauri signer generate -w $env:USERPROFILE\.tauri\persivio-release.key
```

Choose a strong password. Then:

1. Copy the contents of `persivio-release.key.pub` into `tauri.conf.json` →
   `plugins.updater.pubkey`, and commit. (`tauri.local.conf.json` keeps the local key.)
2. Add two repository secrets (GitHub asks for the value; it is not shown on screen):

   ```powershell
   Get-Content $env:USERPROFILE\.tauri\persivio-release.key -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```

3. Store the key and its password in a password manager. **If they are lost, installed
   copies can never be updated again. If they leak, someone else can sign updates.**

Copies installed from the local channel trust the local key only. Install the first
online release by hand once.

### 2. Code signing (optional, removes the warnings)

This is separate from the update signature. Without it, Windows SmartScreen says "Unknown
publisher" and macOS asks for confirmation in System Settings. **The publisher name in the
installer is only text; only a certificate makes Windows show a verified publisher.**

**Windows**: a code-signing certificate, or Azure Artifact Signing. Configure Tauri's
`bundle.windows.signCommand` (Azure) or `certificateThumbprint` (certificate store) in
`tauri.release.conf.json`. New certificates gain SmartScreen reputation over time.

**macOS**: an Apple Developer ID. Add these as secrets and as `env` of the build step in
`release.yml`: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`,
`APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. Then set `bundle.macOS.signingIdentity` to the identity's name; Tauri then
signs and notarizes. Until then the app is signed ad hoc (`-`), which Apple silicon needs
to start it at all.

## Release checklist

1. `cargo nextest run --workspace`, `npm test`, and the [manual tests](manual-tests.md)
   for what changed.
2. Bump the version in `Cargo.toml`, `apps/desktop/package.json` and
   `extension/manifest.json`; add the `CHANGELOG.md` entry.
3. `node scripts/release-check.mjs`
4. Commit, push, wait for CI.
5. `git tag vX.Y.Z` and `git push origin vX.Y.Z`
6. When the workflow is done: download an installer from the draft, install it, try it.
7. Publish the draft.

If something is wrong, delete the draft and the tag, fix it, and tag again. After
publishing, never replace files: release a new version.
