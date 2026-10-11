# Installs CoolerScreen 2.x for the current user from a build folder, no administrator and no registry entries:
#   - copies the app to %LOCALAPPDATA%\Programs\CoolerScreen (the same place 1.x used)
#   - removes a CoolerScreen 1.x (Electron) install in that folder, only if its Electron marker file is there
#   - creates / updates the Start Menu and Desktop shortcut "CoolerScreen" in place (a pinned shortcut keeps working)
# Your data (%APPDATA%\CoolerScreen, Documents\CoolerScreen\Gallery) is never touched. Undo: scripts\uninstall-local.ps1
# The normal way to install is the setup .exe from a release; this script is for installing your own build.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\install-local.ps1 [-Source <folder with CoolerScreen.exe>]
# (-InstallDir / -StartMenuDir / -DesktopDir exist so the script can be tested against scratch folders.)
param(
  [string]$Source,
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\CoolerScreen'),
  [string]$StartMenuDir = [Environment]::GetFolderPath('Programs'),
  [string]$DesktopDir = [Environment]::GetFolderPath('Desktop')
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Source) {
  # Inside the portable zip the exe is next to scripts\; in the repo use the release build.
  if (Test-Path (Join-Path $repo 'CoolerScreen.exe')) { $Source = $repo }
  else { $Source = Join-Path $repo 'src-tauri\target\release' }
}
$exeSrc = Join-Path $Source 'CoolerScreen.exe'
if (-not (Test-Path $exeSrc)) { throw "CoolerScreen.exe not found in $Source. Build first (scripts\make-release.ps1) or pass -Source <folder>." }
$ffmpeg = @((Join-Path $Source 'ffmpeg.exe'), (Join-Path $repo 'node_modules\ffmpeg-static\ffmpeg.exe')) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $ffmpeg) { throw 'ffmpeg.exe not found next to CoolerScreen.exe or in node_modules (run npm install).' }
$scripts = @((Join-Path $Source 'scripts'), (Join-Path $repo 'scripts')) | Where-Object { Test-Path (Join-Path $_ 'setup-winusb.ps1') } | Select-Object -First 1

$exe = Join-Path $InstallDir 'CoolerScreen.exe'
$running = Get-Process -Name CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$InstallDir*" }
if ($running) { throw 'CoolerScreen is running. Close it first, then run this again.' }

# 1.x (Electron) leftovers: only when the Electron marker is present, so an unrelated folder is never emptied.
if (Test-Path (Join-Path $InstallDir 'resources\app\electron\main.js')) { Remove-Item $InstallDir -Recurse -Force }
New-Item -ItemType Directory -Force (Join-Path $InstallDir 'scripts') | Out-Null
Copy-Item $exeSrc $exe -Force
Copy-Item $ffmpeg (Join-Path $InstallDir 'ffmpeg.exe') -Force
$cs2 = Join-Path $Source 'cs2.exe'
if (Test-Path $cs2) { Copy-Item $cs2 (Join-Path $InstallDir 'cs2.exe') -Force }
Copy-Item (Join-Path $scripts '*.ps1') (Join-Path $InstallDir 'scripts') -Force
Get-ChildItem $InstallDir -Recurse -File | Unblock-File -ErrorAction SilentlyContinue   # drop any "downloaded from the internet" tag

$sh = New-Object -ComObject WScript.Shell
foreach ($l in @((Join-Path $StartMenuDir 'CoolerScreen.lnk'), (Join-Path $DesktopDir 'CoolerScreen.lnk'))) {
  $s = $sh.CreateShortcut($l)   # an existing .lnk of the same name is updated in place
  $s.TargetPath = $exe
  $s.WorkingDirectory = $InstallDir
  $s.IconLocation = "$exe,0"
  $s.Description = 'CoolerScreen - Lovingcool SC70 screen manager'
  $s.Save()
  "Shortcut: $l"
}
"Installed to $InstallDir"
