@echo off
rem Try the CoolerScreen 3.0 test build without installing it. Quit the installed CoolerScreen first
rem (tray icon > Quit, or close its window): only one CoolerScreen can run at a time.
cd /d "%~dp0"
set EXE=src-tauri\target\release\coolerscreen.exe
if not exist "%EXE%" (
  echo Cannot find %EXE% - ask for the test build to be built first.
  pause
  exit /b 1
)
start "" "%EXE%"
