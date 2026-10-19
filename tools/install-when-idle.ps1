# Installs a CoolerScreen build quietly, but only when the running app is idle (user rules 2026-10-19).
# Usage: powershell -ExecutionPolicy Bypass -File tools\install-when-idle.ps1 -Setup <CoolerScreen_x.y.z_x64-setup.exe>
#
# Running build knows --update (the build contains the --relaunch-after helper): `CoolerScreen.exe --update <setup>` makes the
# APP quit cleanly, run the installer /S and start the new build with --background, all by itself (the helper is spawned by the
# real app, so everything runs outside a Claude shell's private MSIX copy of %APPDATA%). Exit codes of --update: 0 accepted and the
# app has exited, 2 busy (a job is running: nothing done, try again later), 3 no running instance, 4 not a CoolerScreen installer.
# Older running builds (only --quit, 4.9.1+): quit, install /S, and the app is NOT restarted (a process started from a Claude shell
# would save into the private copy): the user reopens it from the Start menu. Builds before 4.9.1: never touched.
# This script does not read device.log (from a Claude shell it may be a stale copy).
# Exit codes of this script: 0 installed (see the message: restarted by the app, or closed), 2 app busy or did not quit (nothing
# done), 1 failed.
param([Parameter(Mandatory)][string]$Setup)

$exe = Join-Path $env:LOCALAPPDATA "CoolerScreen\CoolerScreen.exe"
$running = @(Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })

if ($running.Count -gt 0) {
    $ver = [version]((Get-Item $exe).VersionInfo.ProductVersion -replace '[^0-9.].*$', '')
    if ($ver -lt [version]"4.9.1") { "running build $ver has no --quit: not installing"; exit 2 }
    # does this build contain the update helper? (its flag is a literal in the exe)
    $canUpdate = Select-String -Path $exe -Pattern '--relaunch-after' -SimpleMatch -Quiet
    if ($canUpdate) {
        $p = Start-Process $exe -ArgumentList '--update', "`"$Setup`"" -WindowStyle Hidden -Wait -PassThru
        switch ($p.ExitCode) {
            0 {
                # the app quit; its helper now runs the installer and starts the new build. Wait for the new version to show.
                $want = [version](([IO.Path]::GetFileName($Setup)) -replace '^CoolerScreen_(\d+\.\d+\.\d+)_.*$', '$1')
                for ($i = 0; $i -lt 240; $i++) {
                    Start-Sleep -Milliseconds 500
                    $now = [version]((Get-Item $exe).VersionInfo.ProductVersion -replace '[^0-9.].*$', '')
                    if ($now -ge $want -and (Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })) { "updated to $now; CoolerScreen restarted itself quietly"; exit 0 }
                }
                "the app quit and the installer was started, but the new build is not running yet: check again in a minute"; exit 1
            }
            2 { "the app is busy (a job is running): not installing, try again when idle"; exit 2 }
            3 { }  # it stopped between the check above and now: install as if it was not running
            4 { "that file is not a CoolerScreen installer: $Setup"; exit 1 }
            default { "the app did not quit (--update exit code $($p.ExitCode)): not installing"; exit 2 }
        }
    } else {
        # 4.9.1+ knows --quit (the tray Quit path; it refuses while a job runs). Never force-close.
        $p = Start-Process $exe -ArgumentList "--quit" -WindowStyle Hidden -Wait -PassThru
        foreach ($r in $running) { $r.WaitForExit(25000) | Out-Null }
        if (Get-Process CoolerScreen -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
            "the app did not quit (a job is running, or it refused): not installing"; exit 2
        }
    }
}

$i = Start-Process $Setup -ArgumentList "/S" -WindowStyle Hidden -PassThru -Wait
if ($i.ExitCode -ne 0) { "installer failed: $($i.ExitCode)"; exit 1 }
"installed $((Get-Item $exe).VersionInfo.ProductVersion); CoolerScreen is closed: ask the user to reopen it from the Start menu"
exit 0
