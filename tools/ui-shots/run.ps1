# Start a QUIET CoolerScreen test instance for UI screenshots (never visible, never focused, fake screen).
#   powershell -ExecutionPolicy Bypass -File tools\ui-shots\run.ps1 -Exe <debug coolerscreen.exe> -Scratch <folder> [-Port 9333] [-Fake winusb]
# Prints the process id; stop it with Stop-Process -Id <id>. Needs a debug build (COOLERSCREEN_MULTI only works there).
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Scratch,
  [int]$Port = 9333,
  [string]$Fake = 'winusb',
  [string]$HomeDir = 'home',
  [string]$Gallery = 'gallery'
)
$env:COOLERSCREEN_FAKE_DEVICE = $Fake
$env:COOLERSCREEN_MULTI = '1'
$env:COOLERSCREEN_TEST_QUIET = '1'
$env:COOLERSCREEN_HOME = Join-Path $Scratch $HomeDir
$env:COOLERSCREEN_GALLERY = Join-Path $Scratch $Gallery
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $Scratch 'wv2'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
New-Item -ItemType Directory -Force $env:COOLERSCREEN_HOME, $env:COOLERSCREEN_GALLERY | Out-Null
$p = Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden
$p.Id
