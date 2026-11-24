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

   With a signing key, also the update packages and their `.sig` files.
3. **finish** adds the browser extension and `SHA256SUMS.txt`. Every installer contains the
   license notices of the software inside (`scripts/third-party-notices.mjs`).

This draft stays in the source repository, where only its owner sees it. Then, from your
computer, **once per release** (not once per computer):

```powershell
node scripts/publish-release.mjs vX.Y.Z             # checks everything, makes a public draft
node scripts/publish-release.mjs vX.Y.Z --publish   # updates go out from this moment
```

The first command compares checksums, checks that every package is signed for this version
with the key the app trusts, and writes `latest.json` with the public addresses.

To try the builds without a release: Actions → Release → **Run workflow**. The installers
appear as artifacts of the run.

## One-time setup

### 0. Where releases live

The source repository was private when this was set up, and installed copies have no GitHub
login. Installers
and `latest.json` therefore go to a second, **public** repository that contains no source:
[persivio-releases](https://github.com/EchoSNS/persivio-releases). It is free.

The source is public since 2026-11-25. The second repository stays, because installed copies
look for updates there, and because it lets a release be tried before it is announced.

### 1. Create the release signing key

The local key has no password and must not sign public releases (the workflow refuses it).

```powershell
cd apps\desktop
npx tauri signer generate -w $env:USERPROFILE\.tauri\persivio-release.key
```

Choose a strong password. Then:

1. Copy the contents of `persivio-release.key.pub` into `tauri.conf.json` →
   `plugins.updater.pubkey`, and commit. (`tauri.local.conf.json` keeps the local key.)
2. Give the build two **secrets**. A secret is a locked note in the repository's settings:
   the release workflow can use it, but nobody can read it back, not even you. They are
   needed because GitHub's computers build and sign the release. In the project folder:

   ```powershell
   # The key file. Nothing is shown; "Set Actions secret" means it worked.
   Get-Content $env:USERPROFILE\.tauri\persivio-release.key -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY --repo EchoSNS/Persivio

   # The password: it asks "Paste your secret". Type the password you chose, press Enter.
   # The characters stay invisible. That is normal.
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo EchoSNS/Persivio

   # Check: lists the two names (never the values).
   gh secret list --repo EchoSNS/Persivio
   ```

   Or in the browser: the source repository → Settings → Secrets and variables → Actions →
   **New repository secret**, once per secret.

3. Store the key and its password in a password manager. **If they are lost, installed
   copies can never be updated again. If they leak, someone else can sign updates.**

Copies installed from the local channel trust the local key only. Install the first
online release by hand once.

### 2. Code signing (optional, removes the warnings)

This is separate from the update signature. Without it, Windows SmartScreen says "Unknown
publisher" and macOS asks for confirmation in System Settings. **The publisher name in the
installer is only text; only a certificate makes Windows show a verified publisher.**

**Free for open source**: [SignPath Foundation](https://signpath.org/terms) signs projects
whose source is public and openly licensed; its name is then shown as the publisher. The
source is public, so Persivio can apply.

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
6. When the workflow is done: `node scripts/publish-release.mjs vX.Y.Z`, download an installer
   from the public draft, install it, try it.
7. `node scripts/publish-release.mjs vX.Y.Z --publish`

If something is wrong, delete both drafts and the tag, fix it, and tag again. After
publishing, never replace files: release a new version.
