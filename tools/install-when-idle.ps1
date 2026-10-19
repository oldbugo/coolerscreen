# Installs a CoolerScreen build quietly, but only if the running app agrees to quit (user rules 2026-10-19).
# The app is NOT restarted afterwards: the user reopens it from the Start menu. A process started from a Claude shell runs
# inside the Claude app's private (MSIX) copy of %APPDATA%, so a restart from here would make CoolerScreen save its settings
# and library into a hidden folder. For the same reason this script does not read device.log (it may be a stale copy).
# Usage: powershell -ExecutionPolicy Bypass -File tools\install-when-idle.ps1 -Setup <CoolerScreen_x.y.z_x64-setup.exe>
# Exit codes: 0 installed (app closed: tell the user to reopen it), 2 app busy or did not quit (nothing done), 1 failed.
param([Parameter(Mandatory)][string]$Setup)

$exe = Join-Path $env:LOCALAPPDATA "CoolerScreen\CoolerScreen.exe"
$running = @(Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })

if ($running.Count -gt 0) {
    # 4.9.1+ knows --quit (the tray Quit path; it refuses while a job runs). Older builds would treat it as a second launch and
    # show their window, so never send it to them; never force-close either.
    $ver = [version]((Get-Item $exe).VersionInfo.ProductVersion -replace '[^0-9.].*$', '')
    if ($ver -lt [version]"4.9.1") { "running build $ver has no --quit: not installing"; exit 2 }
    Start-Process $exe -ArgumentList "--quit" -WindowStyle Hidden -Wait | Out-Null
    foreach ($p in $running) { $p.WaitForExit(25000) | Out-Null }
    if (Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
        "the app did not quit (a job is running, or it refused): not installing"; exit 2
    }
}

$i = Start-Process $Setup -ArgumentList "/S" -WindowStyle Hidden -PassThru -Wait
if ($i.ExitCode -ne 0) { "installer failed: $($i.ExitCode)"; exit 1 }
"installed $((Get-Item $exe).VersionInfo.ProductVersion); CoolerScreen is closed: ask the user to reopen it from the Start menu"
exit 0
