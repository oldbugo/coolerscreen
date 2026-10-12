@echo off
rem CoolerScreen live-mode trials: double-click this file. It shows a menu and asks what you see on the screen.
rem Close the CoolerScreen app first (only one program can use the screen at a time).
cd /d "%~dp0"
set EXE=src-tauri\target\release\cs2.exe
if not exist "%EXE%" (
  echo Cannot find %EXE%
  echo Ask for the tool to be built first: cargo build --release --bin cs2
  pause
  exit /b 1
)
"%EXE%" trial %*
echo.
echo Finished. The results were saved in the work\trials folder.
pause
