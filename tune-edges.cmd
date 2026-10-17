@echo off
rem CoolerScreen edge tuning (temporary): double-click this file. Quit CoolerScreen from the tray first
rem (only one program can use the screen at a time). Follow the instructions printed in the window.
cd /d "%~dp0"
set EXE=src-tauri\target\release\cs2.exe
if not exist "%EXE%" (
  echo Cannot find %EXE%
  echo Ask for the tool to be built first: cargo build --release --bin cs2
  pause
  exit /b 1
)
"%EXE%" edge-tune %*
echo.
echo Finished. If you pressed S, your values are in %%APPDATA%%\CoolerScreen\edge-offsets.json
pause
