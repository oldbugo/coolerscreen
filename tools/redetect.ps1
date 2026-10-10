# Elevated: restart a USB device that failed enumeration (e.g. "Device Descriptor Request Failed"), then rescan.
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$log = Join-Path $Root 'work\redetect.log'
Remove-Item $log -ErrorAction SilentlyContinue
$bad = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '^USB\\VID_0000&PID_000[0-9A-F]' }
foreach ($d in $bad) {
  "restart $($d.InstanceId)" | Add-Content $log
  (pnputil /restart-device "$($d.InstanceId)" | Out-String) | Add-Content $log
}
Start-Sleep 3
(pnputil /scan-devices | Out-String) | Add-Content $log
Start-Sleep 3
Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_33C3|VID_0000' } | ForEach-Object { "$($_.Status) $($_.FriendlyName) $($_.InstanceId)" } | Add-Content $log
