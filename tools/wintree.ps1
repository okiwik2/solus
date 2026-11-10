# Diagnostic: print a process's top-level windows and their child-window tree (class, visible, rect).
param([int]$ProcessId)
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public class WT {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetWindowRgnBox(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
  public static List<IntPtr> Tops(uint pid) { var l = new List<IntPtr>(); EnumWindows((h, x) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) l.Add(h); return true; }, IntPtr.Zero); return l; }
  public static List<IntPtr> Kids(IntPtr h) { var l = new List<IntPtr>(); EnumChildWindows(h, (c, x) => { l.Add(c); return true; }, IntPtr.Zero); return l; }
}
"@
foreach ($t in [WT]::Tops($ProcessId)) {
  $r = New-Object WT+RECT; [void][WT]::GetWindowRect($t, [ref]$r)
  "TOP {0} {1} [{2},{3},{4},{5}]" -f $t, [WT]::Cls($t), $r.L, $r.T, $r.R, $r.B
  foreach ($k in [WT]::Kids($t)) {
    $d = 0; $p = $k; while (($p = [WT]::GetParent($p)) -ne $t -and $p -ne 0) { $d++ }
    $r = New-Object WT+RECT; [void][WT]::GetWindowRect($k, [ref]$r)
    $b = New-Object WT+RECT; $rt = [WT]::GetWindowRgnBox($k, [ref]$b)
    $rg = if ($rt -gt 1) { " rgn=[{0},{1},{2},{3}]" -f $b.L, $b.T, $b.R, $b.B } else { "" }
    $ownPid = 0; [void][WT]::GetWindowThreadProcessId($k, [ref]$ownPid)
    "{0}{1} {2} vis={3} [{4},{5},{6},{7}] pid={8}{9}" -f ('  ' * ($d + 1)), $k, [WT]::Cls($k), [WT]::IsWindowVisible($k), $r.L, $r.T, $r.R, $r.B, $ownPid, $rg
  }
}
