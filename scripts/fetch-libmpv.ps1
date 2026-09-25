# Fetches libmpv-2.dll (mpv dev build) for local dev / CI on Windows.
# Output: src-tauri\libmpv\libmpv-2.dll  (+ copied into target\debug if the dev exe exists)
#
# Primary source: GitHub releases of shinchiro/mpv-winbuild-cmake — release
# assets are direct CDN downloads (no Cloudflare challenge, unlike SourceForge).
# The archive is .7z, so we also fetch 7zr.exe (7-Zip standalone, extract-only)
# into %TEMP% once.
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$DestDir = Join-Path $Root "src-tauri\libmpv"
New-Item -ItemType Directory -Force -Path $DestDir | Out-Null

# Pin the mpv dev build (tag = release date, commit = mpv git hash).
$Tag = "20260924"
$Commit = "2a4eb8067c"
$ArcName = "mpv-dev-x86_64-$Tag-git-$Commit.7z"
$ArcUrl = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/$Tag/$ArcName"
$Arc = Join-Path $env:TEMP $ArcName

# --- download the archive ---
if (-not (Test-Path $Arc) -or (Get-Item $Arc).Length -lt 1000000) {
  Remove-Item $Arc -ErrorAction SilentlyContinue
  & curl.exe -L -sS --max-time 600 -o $Arc $ArcUrl
  if (-not (Test-Path $Arc)) { throw "download failed: $ArcUrl" }
  $b = [System.IO.File]::ReadAllBytes($Arc)[0..1]
  if ($b[0] -ne 0x37) {
    throw ("downloaded file is not a 7z archive (magic {0:X2}{1:X2})" -f $b[0], $b[1])
  }
}

# --- ensure an extractor (7-Zip standalone handles .7z) ---
$7zr = Join-Path $env:TEMP "7zr.exe"
if (-not (Test-Path $7zr)) {
  & curl.exe -L -sS --max-time 120 -o $7zr "https://www.7-zip.org/a/7zr.exe"
  if (-not (Test-Path $7zr)) { throw "could not download 7zr.exe" }
}

# --- extract ---
$Out = Join-Path $env:TEMP ("mpv-dev-x-" + $Commit)
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Out | Out-Null
& $7zr x $Arc "-o$Out" -y | Out-Null

$Dll = Get-ChildItem -Recurse -Path $Out -Filter *.dll |
  Where-Object { $_.Name -match 'mpv' } | Select-Object -First 1
if (-not $Dll) { throw "no libmpv dll inside the archive" }
Copy-Item $Dll.FullName (Join-Path $DestDir "libmpv-2.dll") -Force

# --- also drop it next to the dev exe when a debug build exists ---
$DebugDir = Join-Path $Root "target\debug"
if (Test-Path (Join-Path $DebugDir "iwaks.exe")) {
  Copy-Item $Dll.FullName (Join-Path $DebugDir "libmpv-2.dll") -Force
  Write-Host "copied libmpv-2.dll to target\debug"
}

$Size = (Get-Item (Join-Path $DestDir "libmpv-2.dll")).Length
Write-Host ("libmpv-2.dll ready: {0} bytes" -f $Size)