# Sum memory of every process whose command line contains -Match (e.g. a browser's --user-data-dir).
# Output: JSON {count, privateWs, workingSet, commit} in bytes. "privateWs" = private working set,
# the same measure Windows Task Manager's "Memory" column uses.
param([string]$Match = "", [int]$RootPid = 0, [switch]$Kill)
Add-Type @"
using System; using System.Runtime.InteropServices;
public class PM {
  [StructLayout(LayoutKind.Sequential)] public struct C { public uint cb, PageFaultCount; public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage, PrivateWorkingSetSize; public ulong SharedCommitUsage; }
  [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint a, bool i, int pid);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("psapi.dll")] public static extern bool GetProcessMemoryInfo(IntPtr h, ref C c, uint cb);
  [DllImport("kernel32.dll")] public static extern bool GetProcessTimes(IntPtr h, out long c, out long e, out long k, out long u);
  public static long Cpu(int pid) { var h = OpenProcess(0x1000, false, pid); if (h == IntPtr.Zero) return 0; long c, e, k, u; bool ok = GetProcessTimes(h, out c, out e, out k, out u); CloseHandle(h); return ok ? k + u : 0; }
  public static ulong[] Get(int pid) { var h = OpenProcess(0x1000, false, pid); if (h == IntPtr.Zero) return null; var c = new C(); c.cb = (uint)Marshal.SizeOf(typeof(C)); bool ok = GetProcessMemoryInfo(h, ref c, c.cb); CloseHandle(h); if (!ok) return null; return new ulong[] { (ulong)c.PrivateWorkingSetSize, (ulong)c.WorkingSetSize, (ulong)c.PrivateUsage }; }
}
"@
$all = Get-CimInstance Win32_Process
if ($RootPid -gt 0) {
  # the root process and all of its descendants
  $set = @{ $RootPid = $true }; $grew = $true
  while ($grew) { $grew = $false; foreach ($p in $all) { if ($set.ContainsKey([int]$p.ParentProcessId) -and -not $set.ContainsKey([int]$p.ProcessId)) { $set[[int]$p.ProcessId] = $true; $grew = $true } } }
  $procs = $all | Where-Object { $set.ContainsKey([int]$_.ProcessId) }
} else {
  $procs = $all | Where-Object { $Match -and $_.CommandLine -and $_.CommandLine.Contains($Match) }
}
if ($Kill) { $procs | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }; '{"killed":' + @($procs).Count + '}'; exit }
$pws = [uint64]0; $ws = [uint64]0; $commit = [uint64]0; $n = 0; $kinds = @{}; $cpu = [int64]0
foreach ($p in $procs) {
  $m = [PM]::Get([int]$p.ProcessId)
  $cpu += [PM]::Cpu([int]$p.ProcessId)
  if ($m) { $pws += $m[0]; $ws += $m[1]; $commit += $m[2]; $n++
    $t = if ($p.CommandLine -match '--type=([a-z-]+)') { $Matches[1] } else { $p.Name }
    if (-not $kinds.ContainsKey($t)) { $kinds[$t] = [uint64]0 }; $kinds[$t] += $m[0] }
}
@{ count = $n; cpuMs = [int64]($cpu / 10000); privateWs = $pws; workingSet = $ws; commit = $commit; byType = $kinds } | ConvertTo-Json -Compress
