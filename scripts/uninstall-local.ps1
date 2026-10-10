# Removes what install-local.ps1 created: the two shortcuts and %LOCALAPPDATA%\Programs\CoolerScreen.
# Your gallery (Documents\CoolerScreen\Gallery) and settings/cache (%APPDATA%\CoolerScreen) are NOT touched.
$ErrorActionPreference = 'Stop'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\CoolerScreen'
$running = Get-Process -Name CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -like "$dest*" }
if ($running) { throw 'CoolerScreen is running. Close it first, then run this again.' }
foreach ($l in @((Join-Path ([Environment]::GetFolderPath('Programs')) 'CoolerScreen.lnk'), (Join-Path ([Environment]::GetFolderPath('Desktop')) 'CoolerScreen.lnk'))) {
  if (Test-Path $l) { Remove-Item $l -Force; "Removed $l" }
}
if (Test-Path $dest) { Remove-Item $dest -Recurse -Force; "Removed $dest" }
'Done. Your gallery and settings were left in place.'
