# Elevated: restart the USB root hub the screen hangs off (re-enumerates everything on it), then report.
param([string]$Root = (Split-Path -Parent $PSScriptRoot), [string]$Hub = 'USB\ROOT_HUB30\7&360c8ead&0&0')
$log = Join-Path $Root 'work\reset-hub.log'
Remove-Item $log -ErrorAction SilentlyContinue
"restart $Hub" | Add-Content $log
(pnputil /restart-device "$Hub" | Out-String) | Add-Content $log
Start-Sleep 6
Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_33C3|VID_0000' } | ForEach-Object { "$($_.Status) $($_.Class) $($_.FriendlyName) $($_.InstanceId)" } | Add-Content $log
