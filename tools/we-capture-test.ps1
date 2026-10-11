# Feasibility test: open a Wallpaper Engine scene in its own 2400x1080 window, record it with ffmpeg, close it.
# Leaves the desktop wallpaper alone. Output: work\we-test-gdigrab.mp4 and work\we-test-ddagrab.mp4
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$we = (Get-Process wallpaper64 -ErrorAction Stop | Select-Object -First 1).Path
$ffmpeg = Join-Path $Root 'node_modules\ffmpeg-static\ffmpeg.exe'
$ws = 'C:\Program Files (x86)\Steam\steamapps\workshop\content\431960'
$item = Get-ChildItem $ws -Directory | Where-Object {
  $p = Join-Path $_.FullName 'project.json'
  (Test-Path $p) -and ((Get-Content $p -Raw | ConvertFrom-Json).type -eq 'scene')
} | Select-Object -First 1
$proj = Join-Path $item.FullName 'project.json'
"scene: $((Get-Content $proj -Raw | ConvertFrom-Json).title) ($($item.Name))"
$name = 'CoolerScreen capture'
& $we -control openWallpaper -file $proj -playInWindow $name -width 2400 -height 1080 -x 0 -y 0
Start-Sleep 5
$w = Get-Process | Where-Object { $_.MainWindowTitle -eq $name } | Select-Object -First 1
"window found: $([bool]$w)  owner: $($w.ProcessName)"
& $ffmpeg -v error -y -f gdigrab -framerate 18 -t 4 -i "title=$name" -an -c:v libx264 -preset ultrafast (Join-Path $Root 'work\we-test-gdigrab.mp4')
& $ffmpeg -v error -y -f lavfi -i "ddagrab=output_idx=0:framerate=18:offset_x=0:offset_y=0:video_size=2400x1080,hwdownload,format=bgra" -t 4 -an -c:v libx264 -preset ultrafast (Join-Path $Root 'work\we-test-ddagrab.mp4')
& $we -control closeWallpaper -location $name
"closed"
