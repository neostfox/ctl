#!/usr/bin/env pwsh
<#
.SYNOPSIS
  One-time bootstrap publish of the @velo-ai npm packages from prebuilt GitHub
  Release binaries.

.DESCRIPTION
  npm trusted publishing (OIDC) cannot create a package on its FIRST publish
  (npm/cli#8544 — the npmjs.com UI requires a package to exist before a trusted
  publisher can be configured), and CI token publishing is blocked by npm's 2FA
  enforcement. This script bridges that gap: it publishes the packages from your
  machine using an interactive npm session (which can satisfy 2FA via an OTP
  prompt), reusing the binaries the release workflow already built and attached
  to the GitHub Release — so nothing is cross-compiled locally.

  Run it ONCE to make the packages exist. Afterwards, configure a trusted
  publisher for each package on npmjs.com (repo + release.yml) and let CI publish
  every future version via OIDC (see .github/workflows/release.yml).

  Publish order is dependency-correct: the five platform binary packages, then
  the @velo-ai/ctl meta-package (which bundles every platform binary under
  platforms/), then the @velo-ai/omp plugin (which depends on @velo-ai/ctl).

.PARAMETER Version
  Release version to publish, e.g. 0.0.7. Defaults to the version in Cargo.toml.
  Must match the committed npm package versions (bump them first via the release
  flow); the script refuses to publish a mismatch.

.PARAMETER Repo
  GitHub repository holding the release assets. Default: neostfox/ctl.

.EXAMPLE
  npm login                                   # interactive, 2FA-capable session
  ./scripts/npm-bootstrap-publish.ps1         # version read from Cargo.toml

.EXAMPLE
  ./scripts/npm-bootstrap-publish.ps1 -Version 0.0.7 -Repo neostfox/ctl
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Repo = "neostfox/ctl"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
# npm/gh/tar write progress to stderr and we check exit codes by hand, so do not
# let non-zero native exits auto-throw mid-pipeline.
$PSNativeCommandUseErrorActionPreference = $false

# Repo root = the parent of this script's directory (scripts/).
$RepoRoot = Split-Path -Parent $PSScriptRoot

function Invoke-Native {
    param([Parameter(Mandatory)][string]$What, [Parameter(Mandatory)][scriptblock]$Action)
    & $Action
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)." }
}

Push-Location $RepoRoot
try {
    # --- Resolve version (from Cargo.toml if not supplied) ----------------------
    if (-not $Version) {
        $m = Select-String -Path (Join-Path $RepoRoot 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
            Select-Object -First 1
        if (-not $m) { throw "Could not read version from Cargo.toml; pass -Version explicitly." }
        $Version = $m.Matches[0].Groups[1].Value
    }
    Write-Host "Bootstrap-publishing @velo-ai packages at v$Version from $Repo" -ForegroundColor Cyan

    # --- Preflight: tools + authenticated npm session ---------------------------
    foreach ($cmd in 'npm', 'gh', 'tar') {
        if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
            throw "Required command '$cmd' is not on PATH."
        }
    }
    $who = & npm whoami 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($who)) {
        throw "Not logged in to npm. Run 'npm login' first (enter your 2FA when prompted)."
    }
    Write-Host "npm user: $who"

    # --- Guard: committed versions must match the release ----------------------
    $metaVer = (Get-Content (Join-Path $RepoRoot 'npm/package.json') -Raw | ConvertFrom-Json).version
    if ($metaVer -ne $Version) {
        throw "npm/package.json is $metaVer but you asked for $Version. " +
              "Bump the versions first (release bump + 'ctl skills sync'), commit, then re-run."
    }

    # target archive -> platform package dir + binary name
    $platforms = @(
        @{ Archive = "ctl-x86_64-unknown-linux-gnu.tar.gz";  Dir = "linux-x64-gnu";   Bin = "ctl" }
        @{ Archive = "ctl-aarch64-unknown-linux-gnu.tar.gz"; Dir = "linux-arm64-gnu"; Bin = "ctl" }
        @{ Archive = "ctl-x86_64-apple-darwin.tar.gz";       Dir = "darwin-x64";      Bin = "ctl" }
        @{ Archive = "ctl-aarch64-apple-darwin.tar.gz";      Dir = "darwin-arm64";    Bin = "ctl" }
        @{ Archive = "ctl-x86_64-pc-windows-msvc.zip";       Dir = "win32-x64-msvc";  Bin = "ctl.exe" }
    )

    # --- Download the release assets -------------------------------------------
    $tmp = Join-Path $env:TEMP "ctl-bootstrap-$Version"
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    Write-Host "Downloading v$Version assets to $tmp ..."
    Invoke-Native "gh release download v$Version" {
        gh release download "v$Version" --repo $Repo --dir $tmp --clobber
    }

    # --- Stage each binary into its platform package ---------------------------
    Write-Host "Staging binaries into npm/platforms/* ..."
    foreach ($p in $platforms) {
        $archive = Join-Path $tmp $p.Archive
        $dest = Join-Path $RepoRoot "npm/platforms/$($p.Dir)"
        if (-not (Test-Path $archive)) { throw "Missing release asset: $($p.Archive)" }
        # Windows `tar` is bsdtar (libarchive): extracts both .tar.gz and .zip.
        Invoke-Native "extract $($p.Bin) from $($p.Archive)" {
            tar -xf $archive -C $dest $p.Bin
        }
        if (-not (Test-Path (Join-Path $dest $p.Bin))) {
            throw "Expected '$($p.Bin)' in $dest after extraction."
        }
    }

    # --- Publish in dependency order (npm prompts for OTP if 2FA is on) ---------
    function Publish-Pkg {
        param([Parameter(Mandatory)][string]$Dir)
        Write-Host "Publishing $Dir ..." -ForegroundColor Cyan
        Push-Location $Dir
        try {
            Invoke-Native "npm publish ($Dir)" { npm publish --access public }
        }
        finally { Pop-Location }
    }

    foreach ($p in $platforms) { Publish-Pkg (Join-Path $RepoRoot "npm/platforms/$($p.Dir)") }
    Publish-Pkg (Join-Path $RepoRoot "npm")        # @velo-ai/ctl meta
    Publish-Pkg (Join-Path $RepoRoot "npm-omp")    # @velo-ai/omp plugin

    Write-Host ""
    Write-Host "Done: all @velo-ai packages published at v$Version." -ForegroundColor Green
    Write-Host "Next steps:" -ForegroundColor Yellow
    Write-Host "  1. On npmjs.com, add a Trusted Publisher to each package:" -ForegroundColor Yellow
    Write-Host "       GitHub Actions | repo=$Repo | workflow=release.yml" -ForegroundColor Yellow
    Write-Host "     (so future versions publish via OIDC with no token)." -ForegroundColor Yellow
    Write-Host "  2. Discard the staged binaries: git clean -fd npm/platforms" -ForegroundColor Yellow
}
finally {
    Pop-Location
}
