# Installs CoolerScreen for the current user, no admin needed:
#   - copies the release folder to %LOCALAPPDATA%\Programs\CoolerScreen
#   - creates a Start Menu and a Desktop shortcut "CoolerScreen" (.lnk files only)
# It writes no registry values, no startup entries, no services. Undo: scripts\uninstall-local.ps1
# Usage: powershell -ExecutionPolicy Bypass -File scripts\install-local.ps1 [-Source <release folder>]
param([string]$Source)
$ErrorActionPreference = 'Stop'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Source) {
  # Inside a release the exe is 3 levels up (<release>\resources\app\scripts); in the repo use the newest dist build.
  $up = Resolve-Path (Join-Path $scriptDir '..\..\..') -ErrorAction SilentlyContinue
  if ($up -and (Test-Path (Join-Path $up 'CoolerScreen.exe'))) { $Source = $up.Path }
  else {
    $dist = Join-Path (Split-Path -Parent $scriptDir) 'dist'
    $Source = (Get-ChildItem $dist -Directory -Filter 'CoolerScreen-*-win-x64' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
  }
}
if (-not $Source -or -not (Test-Path (Join-Path $Source 'CoolerScreen.exe'))) { throw 'Release folder with CoolerScreen.exe not found. Pass -Source <folder>.' }

$dest = Join-Path $env:LOCALAPPDATA 'Programs\CoolerScreen'
$exe = Join-Path $dest 'CoolerScreen.exe'
$running = Get-Process -Name CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$dest*" }
if ($running) { throw 'CoolerScreen is running. Close it first, then run this again.' }

if (Test-Path $dest) { Remove-Item $dest -Recurse -Force }
New-Item -ItemType Directory -Force (Split-Path -Parent $dest) | Out-Null
Copy-Item $Source $dest -Recurse
Get-ChildItem $dest -Recurse -File | Unblock-File -ErrorAction SilentlyContinue   # drop any "downloaded from the internet" tag

$sh = New-Object -ComObject WScript.Shell
$links = @(
  (Join-Path ([Environment]::GetFolderPath('Programs')) 'CoolerScreen.lnk'),
  (Join-Path ([Environment]::GetFolderPath('Desktop')) 'CoolerScreen.lnk')
)
foreach ($l in $links) {
  $s = $sh.CreateShortcut($l)
  $s.TargetPath = $exe
  $s.WorkingDirectory = $dest
  $s.Description = 'CoolerScreen - Lovingcool SC70 screen manager'
  $s.Save()
  "Shortcut: $l"
}
"Installed to $dest"
