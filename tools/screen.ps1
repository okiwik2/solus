# Capture the screen area of the Solus window (what the user actually sees).
param([string]$Out = "screen.png")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class S2 {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte b, byte s, uint f, UIntPtr e);
  public struct RECT { public int L, T, R, B; }
}
"@
[S2]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null
$h = [S2]::FindWindow("SolusWindow", [NullString]::Value)
if ($h -eq [IntPtr]::Zero) { Write-Error "no window"; exit 1 }
[S2]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [S2]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
[S2]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 400
$r = New-Object S2+RECT; [S2]::GetWindowRect($h, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
"saved $Out"
