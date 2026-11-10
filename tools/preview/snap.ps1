# Render a preview URL to PNG with headless Edge.  usage: snap.ps1 -Url <url> -Out <png> [-W 1280] [-H 800]
param([string]$Url, [string]$Out, [int]$W = 1280, [int]$H = 800)
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
$tmp = Join-Path $env:TEMP ("solus-prev-" + [guid]::NewGuid())
& $edge --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --user-data-dir="$tmp" --window-size="$W,$H" --virtual-time-budget=3000 --screenshot="$Out" $Url 2>$null | Out-Null
Start-Sleep -Milliseconds 300
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
if (Test-Path $Out) { "saved $Out" } else { "failed" }
