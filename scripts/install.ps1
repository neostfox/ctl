<#
.SYNOPSIS
    ctl installer (Windows / PowerShell).

.DESCRIPTION
    Downloads the ctl release archive for this machine from GitHub Releases,
    extracts ctl.exe into an install directory, and adds it to the user PATH.

    irm https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.ps1 | iex

.PARAMETER Version
    Release tag to install (e.g. v0.0.1). Defaults to the latest release.
    Also reads $env:CTL_VERSION.

.PARAMETER InstallDir
    Install directory. Defaults to %LOCALAPPDATA%\ctl\bin.
    Also reads $env:CTL_INSTALL_DIR.
#>
[CmdletBinding()]
param(
    [string]$Version = $env:CTL_VERSION,
    [string]$InstallDir = $env:CTL_INSTALL_DIR
)

$ErrorActionPreference = 'Stop'
$Repo = 'neostfox/ctl'

if ([string]::IsNullOrWhiteSpace($Version)) { $Version = 'latest' }

# Only an x86_64 Windows binary is published; ARM64 runs it via emulation.
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $archT = 'x86_64' }
    'ARM64' { $archT = 'x86_64'; Write-Warning 'No native ARM64 build; using x64 (emulation).' }
    default { throw "ctl-install: unsupported architecture: $arch" }
}

$target = "$archT-pc-windows-msvc"
$asset  = "ctl-$target.zip"

if ($Version -eq 'latest') {
    $base = "https://github.com/$Repo/releases/latest/download"
} else {
    $base = "https://github.com/$Repo/releases/download/$Version"
}
$url = "$base/$asset"

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'ctl\bin'
}
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$tmp = Join-Path $env:TEMP ("ctl-" + [System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $asset
    Write-Host "ctl-install: downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

    # Checksum verification (best effort).
    try {
        $shaFile = "$zip.sha256"
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $shaFile -UseBasicParsing
        $expected = (((Get-Content $shaFile -Raw) -split '\s+')[0]).Trim().ToLower()
        $actual   = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($expected -and $expected -ne $actual) {
            throw "ctl-install: checksum mismatch (expected $expected, got $actual)"
        }
        Write-Host "ctl-install: checksum verified"
    } catch {
        Write-Warning "ctl-install: checksum not verified: $($_.Exception.Message)"
    }

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $src = Join-Path $tmp 'ctl.exe'
    if (-not (Test-Path $src)) { throw "ctl-install: ctl.exe not found in archive" }
    Copy-Item -Path $src -Destination (Join-Path $InstallDir 'ctl.exe') -Force
    Write-Host "ctl-install: installed to $InstallDir\ctl.exe"

    # Persist to user PATH if missing.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $InstallDir) {
        $newPath = if ([string]::IsNullOrEmpty($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "ctl-install: added $InstallDir to user PATH (open a new shell to use it)."
    }

    & (Join-Path $InstallDir 'ctl.exe') --version
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
