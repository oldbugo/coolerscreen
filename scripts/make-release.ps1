# Builds dist\CoolerScreen-<version>-win-x64.zip (portable: bundled node.exe + production node_modules).
# Run from the repo root:  powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$pkg = Get-Content package.json -Raw | ConvertFrom-Json
$name = "CoolerScreen-$($pkg.version)-win-x64"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist "stage\CoolerScreen"
$zip = Join-Path $dist "$name.zip"

if (Test-Path (Join-Path $dist 'stage')) { Remove-Item (Join-Path $dist 'stage') -Recurse -Force }
Remove-Item $zip -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

# App files (tools/ and work/ are intentionally left out).
foreach ($d in 'bin', 'src', 'public', 'scripts') { Copy-Item $d (Join-Path $stage $d) -Recurse }
Copy-Item package.json, package-lock.json, README.md, LICENSE, CHANGELOG.md $stage

# Production dependencies from a clean install. Install scripts must run so ffmpeg-static downloads ffmpeg.exe.
Push-Location $stage
npm ci --omit=dev --ignore-scripts=false
if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
Pop-Location

# Verify the native/binary pieces, falling back to the dev copy of ffmpeg if the download was blocked.
$ff = Join-Path $stage 'node_modules\ffmpeg-static\ffmpeg.exe'
if (-not (Test-Path $ff)) {
  $dev = Join-Path $root 'node_modules\ffmpeg-static\ffmpeg.exe'
  if (Test-Path $dev) { Copy-Item $dev $ff } else { throw 'ffmpeg.exe missing (npm blocked install scripts? try: npm approve-scripts)' }
}
foreach ($p in 'node_modules\usb\prebuilds\win32-x64', 'node_modules\@serialport\bindings-cpp\prebuilds\win32-x64') {
  if (-not (Get-ChildItem (Join-Path $stage $p) -Filter *.node -ErrorAction SilentlyContinue)) { throw "missing prebuilt binary in $p" }
}
# Drop other platforms' prebuilds to keep the zip small.
foreach ($pb in 'node_modules\usb\prebuilds', 'node_modules\@serialport\bindings-cpp\prebuilds') {
  Get-ChildItem (Join-Path $stage $pb) -Directory | Where-Object { $_.Name -ne 'win32-x64' } | Remove-Item -Recurse -Force
}

# Bundled Node runtime.
Copy-Item (Get-Command node).Source (Join-Path $stage 'node.exe')

# Launcher.
@'
@echo off
title CoolerScreen - keep this window open while you use the app
cd /d "%~dp0"
echo.
echo  CoolerScreen is starting. Your browser will open in a moment.
echo  Keep THIS window open while you use the app. When you are done you can
echo  simply close it: the screen keeps playing its loop by itself.
echo.
"%~dp0node.exe" bin\serve.js
echo.
echo  CoolerScreen has stopped. You can close this window.
pause
'@ | Set-Content -Encoding ASCII (Join-Path $stage 'Start CoolerScreen.bat')

Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $dist 'stage\CoolerScreen'), $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
Remove-Item (Join-Path $dist 'stage') -Recurse -Force
"{0}  {1:N1} MB" -f $zip, ((Get-Item $zip).Length / 1MB)
