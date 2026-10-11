# Builds the 2.0 release into dist\:
#   dist\CoolerScreen_<version>_x64-setup.exe   NSIS installer (per user, no administrator)
#   dist\CoolerScreen-<version>-win-x64.zip     portable zip (unzip anywhere, run CoolerScreen.exe)
# Needs: Rust (stable, MSVC), Node 18+ (npm install fetches the Tauri CLI and ffmpeg.exe). Run from the repo root:
#   powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$pkg = Get-Content package.json -Raw | ConvertFrom-Json
$ver = $pkg.version
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null

if (-not (Test-Path 'node_modules\ffmpeg-static\ffmpeg.exe')) { npm install; if ($LASTEXITCODE -ne 0) { throw 'npm install failed' } }
if (-not (Test-Path 'node_modules\ffmpeg-static\ffmpeg.exe')) { throw 'ffmpeg.exe missing (npm blocked install scripts? try: npm approve-scripts)' }

# Installer (also builds cs2.exe first, see tauri.conf.json beforeBuildCommand).
npx tauri build
if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }
$rel = Join-Path $root 'src-tauri\target\release'
$setup = Join-Path $rel "bundle\nsis\CoolerScreen_${ver}_x64-setup.exe"
if (-not (Test-Path $setup)) { throw "installer not found: $setup" }
Copy-Item $setup $dist -Force

# Portable zip: the exe, ffmpeg, the one-time driver scripts, the recovery tool and the docs, in one folder.
$name = "CoolerScreen-$ver-win-x64"
$stage = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
Remove-Item $zip -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $stage 'scripts') | Out-Null
Copy-Item (Join-Path $rel 'CoolerScreen.exe') $stage
Copy-Item (Join-Path $rel 'cs2.exe') $stage
Copy-Item 'node_modules\ffmpeg-static\ffmpeg.exe' $stage
Copy-Item 'scripts\setup-winusb.ps1', 'scripts\restore-serial.ps1', 'scripts\reconnect.ps1' (Join-Path $stage 'scripts')
Copy-Item 'README.md', 'LICENSE', 'CHANGELOG.md' $stage
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
Remove-Item $stage -Recurse -Force
Get-ChildItem $dist -File | ForEach-Object { '{0}  {1:N1} MB' -f $_.FullName, ($_.Length / 1MB) }
