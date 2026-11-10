# Time from process start until the browser's main window is visible. Prints milliseconds and the PID.
param([Parameter(Mandatory)][string]$Exe, [string]$ArgLine = "")
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Collections.Generic;
public class SW {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(P p, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  public static bool Has(HashSet<uint> pids) {
    bool found = false;
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid); if (pids.Contains(pid) && IsWindowVisible(h)) { var sb = new System.Text.StringBuilder(64); GetClassName(h, sb, 64); var c = sb.ToString(); if (c == "SolusWindow" || c == "Chrome_WidgetWin_1") { found = true; return false; } } return true; }, IntPtr.Zero);
    return found;
  }
}
"@
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -ArgumentList $ArgLine -PassThru
$pids = New-Object 'System.Collections.Generic.HashSet[uint32]'
[void]$pids.Add([uint32]$p.Id)
while ($sw.ElapsedMilliseconds -lt 20000) {
  if ([SW]::Has($pids)) { break }
  Start-Sleep -Milliseconds 10
}
"{0} {1}" -f $sw.ElapsedMilliseconds, $p.Id
