@echo off
rem Developer fallback: run the web app in your browser using the Node.js installed on this PC.
title CoolerScreen (browser mode) - keep this window open
cd /d "%~dp0.."
echo CoolerScreen is starting. Your browser will open in a moment. Close this window when done.
node bin\serve.js
pause
