# Elevated: enable (or -Disable) the screen's COM port. Logs to work\port.log
param([string]$Root = (Split-Path -Parent $PSScriptRoot), [switch]$Disable)
$log = Join-Path $Root 'work\port.log'
$dev = Get-PnpDevice -Class Ports -PresentOnly | Where-Object { $_.InstanceId -match 'VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1
if (-not $dev) { 'screen port not present' | Set-Content $log; exit 1 }
$verb = if ($Disable) { '/disable-device' } else { '/enable-device' }
(pnputil $verb "$($dev.InstanceId)" | Out-String) | Set-Content $log
Start-Sleep 1
"status: $((Get-PnpDevice -InstanceId $dev.InstanceId).Status)" | Add-Content $log
