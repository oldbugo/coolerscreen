# Builds dist\CoolerScreen-<version>-win-x64\ (desktop app) and a zip of it.
# Layout: the official Electron runtime, byte-identical (electron.exe only renamed to CoolerScreen.exe, so its
# hash and any reputation/signature stay intact), with the app in resources\app (no asar) and production node_modules.
# Run from the repo root:  powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$pkg = Get-Content package.json -Raw | ConvertFrom-Json
$name = "CoolerScreen-$($pkg.version)-win-x64"
$dist = Join-Path $root 'dist'
$out = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"
$electronDist = Join-Path $root 'node_modules\electron\dist'
if (-not (Test-Path (Join-Path $electronDist 'electron.exe'))) { throw 'Electron is not installed (run: npm install, then node node_modules\electron\install.js)' }

if (Test-Path $out) { Remove-Item $out -Recurse -Force }
Remove-Item $zip -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist | Out-Null

# 1. Electron runtime, untouched.
Copy-Item $electronDist $out -Recurse
Move-Item (Join-Path $out 'electron.exe') (Join-Path $out 'CoolerScreen.exe')
Remove-Item (Join-Path $out 'resources\default_app.asar') -ErrorAction SilentlyContinue

# 2. App files (tools/ and work/ are intentionally left out).
$app = Join-Path $out 'resources\app'
New-Item -ItemType Directory -Force $app | Out-Null
foreach ($d in 'electron', 'bin', 'src', 'public', 'scripts') { Copy-Item $d (Join-Path $app $d) -Recurse }
Copy-Item package.json, package-lock.json, README.md, LICENSE, CHANGELOG.md $app

# 3. Production dependencies from a clean install (install scripts must run so ffmpeg-static fetches ffmpeg.exe).
Push-Location $app
npm ci --omit=dev --ignore-scripts=false
if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
Pop-Location
$ff = Join-Path $app 'node_modules\ffmpeg-static\ffmpeg.exe'
if (-not (Test-Path $ff)) {
  $dev = Join-Path $root 'node_modules\ffmpeg-static\ffmpeg.exe'
  if (Test-Path $dev) { Copy-Item $dev $ff } else { throw 'ffmpeg.exe missing (npm blocked install scripts? try: npm approve-scripts)' }
}
foreach ($p in 'node_modules\usb\prebuilds\win32-x64', 'node_modules\@serialport\bindings-cpp\prebuilds\win32-x64') {
  if (-not (Get-ChildItem (Join-Path $app $p) -Filter *.node -ErrorAction SilentlyContinue)) { throw "missing prebuilt binary in $p" }
}
foreach ($pb in 'node_modules\usb\prebuilds', 'node_modules\@serialport\bindings-cpp\prebuilds') {
  Get-ChildItem (Join-Path $app $pb) -Directory | Where-Object { $_.Name -ne 'win32-x64' } | Remove-Item -Recurse -Force
}

# 4. The exe must be byte-identical to Electron's.
$h1 = (Get-FileHash (Join-Path $out 'CoolerScreen.exe') -Algorithm SHA256).Hash
$h2 = (Get-FileHash (Join-Path $electronDist 'electron.exe') -Algorithm SHA256).Hash
if ($h1 -ne $h2) { throw 'CoolerScreen.exe differs from electron.exe' }
"CoolerScreen.exe SHA256 = $h1 (identical to electron.exe)"

# 5. Zip (folder is the top-level entry).
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($out, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
"{0}  {1:N1} MB" -f $zip, ((Get-Item $zip).Length / 1MB)
