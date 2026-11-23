# Builds a signed Persivio installer for the LOCAL update channel and publishes it to
# dist-updates/ (served by scripts/serve-updates.mjs on http://127.0.0.1:8787).
# See docs/updates.md.
#
#   powershell -ExecutionPolicy Bypass -File scripts/release-local.ps1
#
# Before running: bump [workspace.package] version in Cargo.toml and add a matching
# "## [x.y.z] - date" entry to CHANGELOG.md.

param(
    [string]$KeyPath = "$env:USERPROFILE\.tauri\persivio-local.key",
    [string]$BaseUrl = "http://127.0.0.1:8787"
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# 1. Version (single source: the workspace Cargo.toml).
$cargo = Get-Content "$root\Cargo.toml" -Raw -Encoding UTF8
if ($cargo -notmatch '(?m)^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"') { throw 'version not found in Cargo.toml' }
$version = $Matches[1]
Write-Host "Persivio $version"

# 2. Release notes from the changelog (required: the app shows them after updating).
$changelog = Get-Content "$root\CHANGELOG.md" -Raw -Encoding UTF8
# (Windows PowerShell 5.1 would otherwise read it as ANSI and garble non-ASCII text.)
$pattern = '(?ms)^## \[' + [regex]::Escape($version) + '\][^\n]*\n(.*?)(?=^## \[|\z)'
if ($changelog -notmatch $pattern) { throw "CHANGELOG.md has no entry for $version" }
$notes = $Matches[1].Trim()

# 3. Build the signed NSIS installer with the local update endpoint.
if (-not (Test-Path $KeyPath)) { throw "signing key not found: $KeyPath (see docs/updates.md)" }
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $KeyPath -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
Push-Location "$root\apps\desktop"
try {
    npx tauri build --config src-tauri/tauri.local.conf.json
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
}

# 4. Copy installer + signature and write the update manifest.
$bundle = "$root\target\release\bundle\nsis"
$installer = Get-ChildItem $bundle -Filter "*_${version}_*-setup.exe" | Select-Object -First 1
if (-not $installer) { throw "installer for $version not found in $bundle" }
$signature = Get-Content "$($installer.FullName).sig" -Raw
$out = "$root\dist-updates\$version"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $installer.FullName, "$($installer.FullName).sig" $out

$manifest = [ordered]@{
    version   = $version
    notes     = $notes
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{
            signature = $signature.Trim()
            url       = "$BaseUrl/$version/$([uri]::EscapeDataString($installer.Name))"
        }
    }
}
$json = $manifest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText("$root\dist-updates\latest.json", $json, (New-Object System.Text.UTF8Encoding $false))

Write-Host ""
Write-Host "Published $version to dist-updates\."
Write-Host "  Installer: $out\$($installer.Name)"
Write-Host "  Serve it:  node scripts/serve-updates.mjs"
