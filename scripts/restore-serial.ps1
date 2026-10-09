# Undo setup-winusb.ps1 (run elevated; CoolerScreen launches it via a UAC prompt).
# Removes the screen's CDC device node so Windows re-detects it and installs the standard usbser (COM port)
# driver again. The vendor app will see the screen again afterwards.
# Log: %TEMP%\coolerscreen-setup.log (last line is "RESULT: OK" or "RESULT: ERROR ...")
$log = Join-Path $env:TEMP 'coolerscreen-setup.log'
function Log($m) { "$(Get-Date -Format 'HH:mm:ss') $m" | Add-Content $log }
Remove-Item $log -ErrorAction SilentlyContinue
try {
  $devs = @(Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)&MI_00' })
  if (-not $devs) { throw 'screen device not found (plug it in first)' }
  foreach ($d in $devs) {
    Log "removing $($d.InstanceId)"
    Log (pnputil /remove-device "$($d.InstanceId)" | Out-String).Trim()
  }
  Start-Sleep 1
  Log (pnputil /scan-devices | Out-String).Trim()
  Start-Sleep 4
  $now = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1
  Log "now: $($now.FriendlyName) class $($now.Class) status $($now.Status)"
  Log 'RESULT: OK'
} catch { Log "RESULT: ERROR $_" }
