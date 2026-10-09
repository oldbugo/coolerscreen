# Elevated: give the WinUSB-bound screen a DeviceInterfaceGUIDs value so user-mode apps (libusb) can open it.
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$log = Join-Path $Root 'work\winusb-guid.log'
Remove-Item $log -ErrorAction SilentlyContinue
try {
  $dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1
  $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$($dev.InstanceId)\Device Parameters"
  New-ItemProperty -Path $key -Name DeviceInterfaceGUIDs -PropertyType MultiString -Value @('{6E45736A-2B1B-4078-B772-B3AF2B6FDE1C}') -Force | Out-Null
  "set on $key" | Add-Content $log
  (pnputil /restart-device "$($dev.InstanceId)" | Out-String) | Add-Content $log
} catch { "ERROR $_" | Add-Content $log }
