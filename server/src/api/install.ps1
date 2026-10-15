# TERESA client installer (Windows) — no Rust toolchain needed.
#
#   irm __TERESA_BASE__/install.ps1 | iex
#
# Later updates: `teresa update` (the client also offers them on launch).
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # PowerShell 5's progress bar is slower than the download

$base = '__TERESA_BASE__'
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
$platform = "windows-$arch"

# Replace an existing teresa (e.g. an old `cargo install`) in place so PATH
# keeps finding it; otherwise %LOCALAPPDATA%\teresa\bin.
$dir = Join-Path $env:LOCALAPPDATA 'teresa\bin'
$existing = Get-Command teresa -ErrorAction SilentlyContinue
if ($existing) { $dir = Split-Path $existing.Source }
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$dest = Join-Path $dir 'teresa.exe'
$tmp = "$dest.download"

Write-Host "* downloading teresa for $platform from $base"
try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri "$base/api/v1/client/download/$platform" -OutFile $tmp -PassThru
} catch {
    Write-Error "no build for $platform on this node yet - ask whoever deploys it"
    return
}

$want = "$($response.Headers['x-teresa-sha256'])"
$got = (Get-FileHash -Algorithm SHA256 $tmp).Hash.ToLower()
if ($want -and $got -ne $want) {
    Remove-Item $tmp
    Write-Error 'checksum mismatch - download corrupted, try again'
    return
}

# A running teresa.exe cannot be overwritten, but it can be renamed.
if (Test-Path $dest) { Move-Item -Force $dest "$dest.old" }
Move-Item -Force $tmp $dest
Write-Host "* installed $dest"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $dir)) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
    Write-Host "! added $dir to your PATH - open a new terminal"
}
Write-Host '  run: teresa'
