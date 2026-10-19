# Installs a CoolerScreen build quietly, but only if the running app is idle (user rule 2026-10-19).
# Usage: powershell -ExecutionPolicy Bypass -File tools\install-when-idle.ps1 -Setup <CoolerScreen_x.y.z_x64-setup.exe> [-Cs2 <cs2.exe>]
# Exit codes: 0 installed, 2 app busy (nothing done, try later), 1 failed.
param([Parameter(Mandatory)][string]$Setup, [string]$Cs2)

$exe = Join-Path $env:LOCALAPPDATA "CoolerScreen\CoolerScreen.exe"
$log = Join-Path $env:APPDATA "CoolerScreen\device.log"
$running = Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }

if ($running) {
    # A job is running when the last "device task:" line comes after the last "device tasks: none left".
    $lines = Get-Content $log -Tail 400
    $lastTask = ($lines | Select-String -Pattern 'device task:' | Select-Object -Last 1).LineNumber
    $lastDone = ($lines | Select-String -Pattern 'device tasks: none left' | Select-Object -Last 1).LineNumber
    if ($lastTask -and (-not $lastDone -or $lastTask -gt $lastDone)) { "busy: a device task is running"; exit 2 }

    # 4.9.1+: clean quit through the tray Quit path; it refuses (2) while a job runs. Older builds do not know --quit:
    # there a second launch is forwarded to the running app and shows its window, so never send it to them.
    $ver = [version]((Get-Item $exe).VersionInfo.ProductVersion -replace '[^0-9.].*$', '')
    if ($ver -ge [version]"4.9.1") {
        $q = Start-Process $exe -ArgumentList "--quit" -WindowStyle Hidden -PassThru -Wait
        if ($q.ExitCode -eq 2) { "busy: the app refused to quit (job running)"; exit 2 }
        foreach ($p in $running) { $p.WaitForExit(20000) | Out-Null }
    }
    $left = Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
    if ($left) {
        # Older builds without --quit: idle was checked above, so force-close.
        "no clean quit (older build): force-closing"
        $left | ForEach-Object { taskkill /F /PID $_.Id | Out-Null }
        Start-Sleep 1
    }
    if ($Cs2 -and (Test-Path $Cs2)) { & $Cs2 wake 2>&1 | Select-Object -Last 1 }
}

$i = Start-Process $Setup -ArgumentList "/S" -WindowStyle Hidden -PassThru -Wait
if ($i.ExitCode -ne 0) { "installer failed: $($i.ExitCode)"; exit 1 }
Start-Process $exe -ArgumentList "--background" -WindowStyle Hidden
Start-Sleep 8
Select-String -Path $log -Pattern 'started, pid' | Select-Object -Last 1 | ForEach-Object Line
exit 0
