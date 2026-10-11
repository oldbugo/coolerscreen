# Removes what install-local.ps1 created: the two shortcuts and %LOCALAPPDATA%\Programs\CoolerScreen.
# Your gallery (Documents\CoolerScreen\Gallery) and settings/cache (%APPDATA%\CoolerScreen) are NOT touched.
# (An installation made with the setup .exe is removed from Windows Settings > Apps instead.)
param(
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\CoolerScreen'),
  [string]$StartMenuDir = [Environment]::GetFolderPath('Programs'),
  [string]$DesktopDir = [Environment]::GetFolderPath('Desktop')
)
$ErrorActionPreference = 'Stop'
$running = Get-Process -Name CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$InstallDir*" }
if ($running) { throw 'CoolerScreen is running. Close it first, then run this again.' }
foreach ($l in @((Join-Path $StartMenuDir 'CoolerScreen.lnk'), (Join-Path $DesktopDir 'CoolerScreen.lnk'))) {
  if (Test-Path $l) { Remove-Item $l -Force; "Removed $l" }
}
if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force; "Removed $InstallDir" }
'Done. Your gallery and settings were left in place.'
