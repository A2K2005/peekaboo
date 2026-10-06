<#
Measure-Launch.ps1 - launch-time harness for LaunchProbe (Preview for Windows research). Windows PowerShell 5.1.
Modes:
  Warm  every run starts a new process; app files stay in the standby list. The app exits after its report.
  Hot   one resident instance; every run starts a second process that redirects to it (single-instance path).
Cold runs need a reboot or a standby-list purge (admin). See docs/research/winui-performance.md.
Run the harness NOT elevated, so the app runs at medium integrity like a double-click.
Examples:
  .\Measure-Launch.ps1 -SelfTest
  .\Measure-Launch.ps1 -Mode Warm -Runs 59 -Flags xcr,toolbar -Exe C:\path\LaunchProbe.exe   # unpackaged
  .\Measure-Launch.ps1 -Mode Hot  -Runs 59                                                    # packaged: opens .lprobe by file association
Gate: with n >= 59 runs, the script reports a distribution-free 95% upper bound on p95 and exits 1 above TargetMs + TolerancePct.
#>
param(
  [ValidateSet('Warm','Hot')] [string]$Mode = 'Warm',
  [int]$Runs = 59,
  [string[]]$Flags = @(),
  [string]$Exe = '',
  [int]$Width = 816,
  [int]$Height = 1056,
  [double]$TargetMs = 0,
  [double]$TolerancePct = 10,
  [string]$OutDir = (Join-Path $env:TEMP 'launchprobe'),
  [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Windows.Forms -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Windows.Forms;
public class ProbeSink : NativeWindow {
    public readonly List<string> Messages = new List<string>();
    [StructLayout(LayoutKind.Sequential)] struct CopyData { public IntPtr DwData; public int CbData; public IntPtr LpData; }
    [DllImport("user32.dll")] static extern bool ChangeWindowMessageFilterEx(IntPtr h, uint msg, uint action, IntPtr p);
    [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr w, ref CopyData c);
    public ProbeSink() {
        CreateParams cp = new CreateParams();
        cp.Caption = "LaunchProbeHarness";
        CreateHandle(cp);
        ChangeWindowMessageFilterEx(Handle, 0x004A, 1, IntPtr.Zero);
    }
    protected override void WndProc(ref Message m) {
        if (m.Msg == 0x004A) {
            CopyData cd = (CopyData)Marshal.PtrToStructure(m.LParam, typeof(CopyData));
            if (cd.DwData == (IntPtr)0x4C50) Messages.Add(Marshal.PtrToStringUni(cd.LpData, cd.CbData / 2 - 1));
            m.Result = (IntPtr)1;
            return;
        }
        base.WndProc(ref m);
    }
    public void SendSelf(string text) {
        IntPtr p = Marshal.StringToHGlobalUni(text);
        CopyData cd = new CopyData(); cd.DwData = (IntPtr)0x4C50; cd.CbData = (text.Length + 1) * 2; cd.LpData = p;
        SendMessageW(Handle, 0x004A, IntPtr.Zero, ref cd);
        Marshal.FreeHGlobal(p);
    }
    public static void Fill(byte[] b, int from, byte v) { for (int i = from; i < b.Length; i++) b[i] = v; }
}
'@

$freq = [Diagnostics.Stopwatch]::Frequency
function Get-Ms([long]$from, [long]$to) { ($to - $from) * 1000.0 / $freq }

function Get-Percentile([double[]]$v, [double]$p) {   # nearest rank
  $s = @($v | Sort-Object)
  $s[[math]::Max(0, [math]::Ceiling($p / 100 * $s.Count) - 1)]
}

# Smallest rank r such that the r-th smallest of n runs is >= the true p95 with 95% confidence.
# Returns 0 when n < 59. Distribution-free (binomial order statistics).
function Get-P95UpperRank([int]$n) {
  $best = 0; $pmf = [math]::Pow(0.95, $n); $tail = 0.0
  for ($k = $n; $k -ge 1; $k--) {
    $tail += $pmf                  # P(Bin(n, 0.95) >= k)
    if ($tail -gt 0.05) { break }
    $best = $k
    $pmf = $pmf * $k / ($n - $k + 1) * (0.05 / 0.95)
  }
  $best
}

if ($SelfTest) {
  if ((Get-P95UpperRank 58) -ne 0 -or (Get-P95UpperRank 59) -ne 59 -or (Get-P95UpperRank 100) -ne 99 -or (Get-P95UpperRank 200) -ne 196) { throw 'rank check failed' }
  if ((Get-Percentile @(1..20) 95) -ne 19 -or (Get-Percentile @(1..100) 50) -ne 50) { throw 'percentile check failed' }
  $s = New-Object ProbeSink
  $s.SendSelf('pid=1;rendered=2')
  $ok = ($s.Messages.Count -eq 1) -and ($s.Messages[0] -eq 'pid=1;rendered=2')
  $s.DestroyHandle()
  if (-not $ok) { throw 'WM_COPYDATA loopback failed' }
  'SelfTest passed'
  return
}

if (-not $TargetMs) { $TargetMs = if ($Mode -eq 'Hot') { 150 } else { 400 } }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$tokens = @('page') + $Flags
if ($Mode -eq 'Warm') { $tokens += 'exit' }
$file = Join-Path $OutDir (($tokens -join '.') + '.lprobe')
$bytes = New-Object byte[] (8 + $Width * $Height * 4)
[BitConverter]::GetBytes([int]$Width).CopyTo($bytes, 0)
[BitConverter]::GetBytes([int]$Height).CopyTo($bytes, 4)
[ProbeSink]::Fill($bytes, 8, 0xFF)
[IO.File]::WriteAllBytes($file, $bytes)

$sink = New-Object ProbeSink
function Start-Probe {
  $psi = New-Object Diagnostics.ProcessStartInfo
  if ($Exe) { $psi.FileName = $Exe; $psi.Arguments = '"' + $file + '"' } else { $psi.FileName = $file }
  $psi.UseShellExecute = $true
  $t0 = [Diagnostics.Stopwatch]::GetTimestamp()
  [void][Diagnostics.Process]::Start($psi)
  $t0
}
function Wait-Report([int]$timeoutMs = 10000) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sink.Messages.Count -eq 0) {
    [Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 1
    if ($sw.ElapsedMilliseconds -gt $timeoutMs) { throw 'No report from LaunchProbe within timeout' }
  }
  $h = @{}
  foreach ($kv in $sink.Messages[0].Split(';')) { $a = $kv.Split('=', 2); $h[$a[0]] = $a[1] }
  $sink.Messages.Clear()
  $h
}
function Wait-Exit([int]$procId) {
  $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
  if ($p -and -not $p.WaitForExit(3000)) { Stop-Process -Id $procId -Force }
}

$resident = $null
$rows = @()
try {
  if ($Mode -eq 'Hot') { [void](Start-Probe); $resident = Wait-Report; Start-Sleep -Seconds 2 }
  for ($i = 1; $i -le $Runs; $i++) {
    $t0 = Start-Probe
    $r = Wait-Report
    $mainMs = $null
    if ($Mode -eq 'Warm') { $mainMs = [math]::Round((Get-Ms $t0 ([long]$r['main'])), 2) }
    $rows += [pscustomobject]@{
      run = $i; mode = $Mode; flags = ($Flags -join '+'); first_run = ($i -eq 1)
      main_ms = $mainMs
      first_frame_ms = [math]::Round((Get-Ms $t0 ([long]$r['rendered'])), 2)
      win2d_ms = $r['win2dMs']; procid = $r['pid']
    }
    if ($Mode -eq 'Warm') { Wait-Exit ([int]$r['pid']) }
    Start-Sleep -Milliseconds 1500
  }
} finally {
  if ($resident) { Stop-Process -Id ([int]$resident['pid']) -Force -ErrorAction SilentlyContinue }
  $sink.DestroyHandle()
}

$csv = Join-Path $OutDir ("launch-{0}-{1}.csv" -f $Mode, (Get-Date -Format 'yyyyMMdd-HHmmss'))
$rows | Export-Csv -NoTypeInformation -Path $csv
$v = [double[]]@($rows | ForEach-Object { $_.first_frame_ms })
$sorted = @($v | Sort-Object)
$limit = $TargetMs * (1 + $TolerancePct / 100)
"{0} runs, {1}, flags [{2}]: median {3:F1} ms, p95 {4:F1} ms, max {5:F1} ms. CSV: {6}" -f $v.Count, $Mode, ($Flags -join ','), (Get-Percentile $v 50), (Get-Percentile $v 95), $sorted[-1], $csv
$rank = Get-P95UpperRank $v.Count
if ($rank -eq 0) { 'Gate not evaluated: at least 59 runs are needed for a 95% upper bound on p95.'; return }
$ub = $sorted[$rank - 1]
if ($ub -le $limit) { "Gate PASS: 95% upper bound on p95 = {0:F1} ms <= {1:F1} ms" -f $ub, $limit }
else { "Gate FAIL: 95% upper bound on p95 = {0:F1} ms > {1:F1} ms" -f $ub, $limit; exit 1 }
