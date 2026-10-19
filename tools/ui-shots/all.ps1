# One command: start a quiet test instance, take the whole screenshot set (or run the flow checks), stop the instance.
#   powershell -ExecutionPolicy Bypass -File tools\ui-shots\all.ps1 -Exe <debug exe> -Scratch <folder> -Out <dir> [-Port 9333] [-Flows]
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Scratch,
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$Port = 9333,
  [string]$Only = '',
  [switch]$Flows   # run flows.mjs (checks of the main flows) instead of the screenshot set
)
$id = & (Join-Path $PSScriptRoot 'run.ps1') -Exe $Exe -Scratch $Scratch -Port $Port
try {
  $ok = $false
  for ($i = 0; $i -lt 40 -and -not $ok; $i++) {
    Start-Sleep -Milliseconds 500
    try { Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json" | Out-Null; $ok = $true } catch {}
  }
  Start-Sleep 6   # the library and the first preview load
  if ($Flows) { node (Join-Path $PSScriptRoot 'flows.mjs') $Port }
  else { node (Join-Path $PSScriptRoot 'shots.mjs') $Port $Out $Only }
} finally {
  Stop-Process -Id $id -Confirm:$false -ErrorAction SilentlyContinue
}
