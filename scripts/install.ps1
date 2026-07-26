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

    # Checksum: fetch the .sha256 sidecar (fatal if missing — refuse to
    # install unverified, matching `ctl self-update`'s policy). A prior
    # version wrapped both fetch and verify in one try/catch, which silently
    # downgraded a real checksum mismatch into a Write-Warning and then
    # installed the tampered binary anyway.
    $shaFile = "$zip.sha256"
    try {
        Invoke-WebRequest -Uri "$url.sha256" -OutFile $shaFile -UseBasicParsing
    } catch {
        throw "ctl-install: checksum file download failed ($($_.Exception.Message)); refusing to install unverified binary"
    }

    # Verify the archive hash. A mismatch is FATAL — never fall through to
    # Expand-Archive on a tampered download.
    $expected = (((Get-Content $shaFile -Raw) -split '\s+')[0]).Trim().ToLower()
    $actual   = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if (-not $expected) {
        throw "ctl-install: checksum file is empty; refusing to install unverified binary"
    }
    if ($expected -ne $actual) {
        throw "ctl-install: checksum mismatch (expected $expected, got $actual); refusing to install tampered binary"
    }
    Write-Host "ctl-install: checksum verified"

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $src = Join-Path $tmp 'ctl.exe'
    if (-not (Test-Path $src)) { throw "ctl-install: ctl.exe not found in archive" }
    Copy-Item -Path $src -Destination (Join-Path $InstallDir 'ctl.exe') -Force
    Write-Host "ctl-install: installed to $InstallDir\ctl.exe"

    # Persist to user PATH, ensuring our dir is FIRST so a freshly installed
    # ctl wins over any pre-existing one (e.g. a cargo- or npm-installed ctl)
    # already earlier in PATH. De-dup any prior occurrence so it moves to front.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries  = @($userPath -split ';' | Where-Object { $_ -ne '' })
    $rest     = @($entries | Where-Object { $_.TrimEnd('\') -ne $InstallDir.TrimEnd('\') })
    if ($entries.Count -eq 0 -or $entries[0].TrimEnd('\') -ne $InstallDir.TrimEnd('\')) {
        $newPath = (@($InstallDir) + $rest) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "ctl-install: put $InstallDir first on user PATH (open a new shell to use it)."
    }

    & (Join-Path $InstallDir 'ctl.exe') --version
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
