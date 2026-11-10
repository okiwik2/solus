# Renders the Solus logo at several sizes and packs them into res/solus.ico (PNG entries).
param([string]$Out = "$PSScriptRoot\..\res\solus.ico", [string]$PngOut = "$PSScriptRoot\..\ui\img\logo-256.png")
Add-Type -AssemblyName System.Drawing
function Render([int]$s) {
  $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'; $g.Clear([System.Drawing.Color]::Transparent)
  $pad = [Math]::Max(0.5, $s * 0.03)
  $rect = New-Object System.Drawing.RectangleF $pad, $pad, ($s - 2*$pad), ($s - 2*$pad)
  $c1 = [System.Drawing.Color]::FromArgb(255, 255, 196, 77); $c2 = [System.Drawing.Color]::FromArgb(255, 255, 84, 61)
  $br = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, $c1, $c2, 45.0
  $g.FillEllipse($br, $rect)
  # orbit ring
  $w = [Math]::Max(1.2, $s * 0.075)
  $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), $w
  $r = $s * 0.27; $cx = $s/2; $cy = $s/2
  $g.DrawEllipse($pen, [float]($cx - $r), [float]($cy - $r), [float](2*$r), [float](2*$r))
  # satellite dot at 45 degrees (top-right)
  $dr = [Math]::Max(1.5, $s * 0.085)
  $dx = $cx + $r * 0.7071; $dy = $cy - $r * 0.7071
  $g.FillEllipse([System.Drawing.Brushes]::White, [float]($dx - $dr), [float]($dy - $dr), [float](2*$dr), [float](2*$dr))
  $g.Dispose(); return $bmp
}
$sizes = 16,20,24,32,40,48,64,128,256
$pngs = @()
foreach ($s in $sizes) { $b = Render $s; $ms = New-Object System.IO.MemoryStream; $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $pngs += ,($ms.ToArray()); if ($s -eq 256) { New-Item -ItemType Directory -Force (Split-Path $PngOut) | Out-Null; $b.Save($PngOut, [System.Drawing.Imaging.ImageFormat]::Png) }; $b.Dispose() }
$fs = [System.IO.File]::Create($Out); $bw = New-Object System.IO.BinaryWriter $fs
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) { $s = $sizes[$i]; $d = if ($s -ge 256) {0} else {$s}
  $bw.Write([byte]$d); $bw.Write([byte]$d); $bw.Write([byte]0); $bw.Write([byte]0); $bw.Write([UInt16]1); $bw.Write([UInt16]32); $bw.Write([UInt32]$pngs[$i].Length); $bw.Write([UInt32]$offset); $offset += $pngs[$i].Length }
foreach ($p in $pngs) { $bw.Write($p) }
$bw.Close(); "wrote $Out"
