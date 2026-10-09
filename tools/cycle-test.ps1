# Elevated test: enable the screen's COM port, restart the screen, disable the port again
# so the screen boots with no serial driver (no DTR watchdog). Logs to work\cycle-test.log
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$log = Join-Path $Root 'work\cycle-test.log'
function Log($m) { "$(Get-Date -Format 'HH:mm:ss.fff') $m" | Tee-Object -FilePath $log -Append }
Set-Location $Root
Remove-Item $log -ErrorAction SilentlyContinue
$dev = Get-PnpDevice -Class Ports -PresentOnly | Where-Object { $_.InstanceId -match 'VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1
if (-not $dev) { Log 'screen port not present'; exit 1 }
$id = $dev.InstanceId
Log "device $id status $($dev.Status)"
Log (pnputil /enable-device "$id" | Out-String)
for ($i = 0; $i -lt 50; $i++) {
  $s = (Get-PnpDevice -InstanceId $id).Status
  if ($s -eq 'OK') { break }
  Start-Sleep -Milliseconds 200
}
Log "after enable: $((Get-PnpDevice -InstanceId $id).Status)"
Start-Sleep -Milliseconds 500
Log (node tools\send.js 01 2>&1 | Out-String)
for ($i = 0; $i -lt 40; $i++) {
  $out = pnputil /disable-device "$id" 2>&1 | Out-String
  if ($LASTEXITCODE -eq 0) { Log "disable ok after $i tries: $out"; break }
  Start-Sleep -Milliseconds 250
}
Start-Sleep 3
Log "final: $((Get-PnpDevice -InstanceId $id).Status)"
