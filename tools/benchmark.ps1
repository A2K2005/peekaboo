#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$InputFile,
    [ValidateRange(1,1000)][int]$Runs = 30,
    [ValidateRange(1,300)][int]$TimeoutSeconds = 15,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../artifacts/benchmarks')
)
$ErrorActionPreference = 'Stop'
if (-not ('PreviewMemory' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PreviewMemory {
 [StructLayout(LayoutKind.Sequential)] public struct Counters {
   public uint cb, faults; public UIntPtr peakWorkingSet, workingSet, peakPaged, paged, peakNonPaged, nonPaged, pagefile, peakPagefile, privateBytes;
 }
 [DllImport("psapi.dll", SetLastError=true)] static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
 public static ulong[] Read(IntPtr process) {
   Counters c; if(!GetProcessMemoryInfo(process,out c,(uint)Marshal.SizeOf<Counters>())) return null;
   return new ulong[]{c.peakWorkingSet.ToUInt64(),c.peakPagefile.ToUInt64()};
 }
}
"@
}
$exe = (Resolve-Path -LiteralPath $Executable).Path
$inputPath = (Resolve-Path -LiteralPath $InputFile).Path
$out = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($out) | Out-Null
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
$session = Join-Path $out $runId
[IO.Directory]::CreateDirectory($session) | Out-Null
$samples = @()
for ($i=1; $i -le $Runs; $i++) {
    $marker = Join-Path $session "sample-$i.marker.json"
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $exe
    $psi.UseShellExecute = $false
    $psi.ArgumentList.Add($inputPath)
    $psi.Environment['PFW_BENCH_OUT'] = $marker
    $psi.Environment['PFW_BENCH_AUTOCLOSE'] = '1'
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $psi
    $processStarted = $false
    $sample = [ordered]@{run=$i; mode='process-relaunch'; input=$inputPath; launch_qpc=0L; first_content_qpc=$null; qpc_frequency=[Diagnostics.Stopwatch]::Frequency; first_content_ms=$null; sampled_peak_private_bytes=0L; sampled_peak_working_set_bytes=0L; os_peak_working_set_bytes=0L; os_peak_private_commit_bytes=0L; exit_code=$null; status='error'; error=$null}
    try {
        $start = [Diagnostics.Stopwatch]::GetTimestamp()
        $sample.launch_qpc = $start
        if (-not $process.Start()) { throw 'Process did not start.' }
        $processStarted = $true
        $handle = $process.Handle
        while (-not $process.HasExited) {
            if (([Diagnostics.Stopwatch]::GetTimestamp()-$start)/[Diagnostics.Stopwatch]::Frequency -gt $TimeoutSeconds) {
                $process.Kill($true)
                $process.WaitForExit()
                throw "Timeout after $TimeoutSeconds seconds."
            }
            try {
                $process.Refresh()
                $sample.sampled_peak_private_bytes = [Math]::Max($sample.sampled_peak_private_bytes,$process.PrivateMemorySize64)
                $sample.sampled_peak_working_set_bytes = [Math]::Max($sample.sampled_peak_working_set_bytes,$process.WorkingSet64)
            } catch [InvalidOperationException] { }
            $peaks = [PreviewMemory]::Read($handle)
            if ($null -ne $peaks) {
                $sample.os_peak_working_set_bytes = [Math]::Max($sample.os_peak_working_set_bytes,$peaks[0])
                $sample.os_peak_private_commit_bytes = [Math]::Max($sample.os_peak_private_commit_bytes,$peaks[1])
            }
            Start-Sleep -Milliseconds 5
        }
        $process.WaitForExit()
        $peaks = [PreviewMemory]::Read($handle)
        if ($null -ne $peaks) {
            $sample.os_peak_working_set_bytes = [Math]::Max($sample.os_peak_working_set_bytes,$peaks[0])
            $sample.os_peak_private_commit_bytes = [Math]::Max($sample.os_peak_private_commit_bytes,$peaks[1])
        }
        $sample.exit_code = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw "Exit code $($process.ExitCode)." }
        if (-not (Test-Path -LiteralPath $marker)) { throw 'No successful content marker.' }
        $m = Get-Content -Raw -LiteralPath $marker | ConvertFrom-Json
        if ($m.qpc_frequency -ne [Diagnostics.Stopwatch]::Frequency -or $m.first_content_qpc -lt $start -or $m.first_content_qpc -gt [Diagnostics.Stopwatch]::GetTimestamp()) { throw 'Invalid QPC marker.' }
        if ($m.width -le 0 -or $m.height -le 0 -or $m.page_count -le 0) { throw 'Marker has no content dimensions/pages.' }
        $sample.first_content_qpc = $m.first_content_qpc
        $sample.first_content_ms = 1000.0 * ($m.first_content_qpc-$start)/$m.qpc_frequency
        $sample.status = 'ok'
    } catch { $sample.error=$_.Exception.Message }
    finally {
        if ($processStarted) {
            try { if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() } }
            catch { $sample.error = "$($sample.error) Cleanup failed: $($_.Exception.Message)"; $sample.status = 'error' }
        }
        $process.Dispose()
    }
    $samples += [pscustomobject]$sample
}
$samples | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $session 'samples.json') -Encoding utf8
$samples | Export-Csv -LiteralPath (Join-Path $session 'samples.csv') -NoTypeInformation
$times = @($samples | Where-Object status -eq 'ok' | ForEach-Object first_content_ms | Sort-Object)
$summary = [ordered]@{mode='process-relaunch'; runs=$Runs; succeeded=$times.Count; failed=$Runs-$times.Count; p95_ms=$null; memory='OS peak working set and peak private commit through last successful query; sampled private/working bytes also retained; not steady-state memory'; cold_launch='not measured'; resident_warm_launch='not measured'; reference_device_gate='unverified'; executable_sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash; input_sha256=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash; os=[Environment]::OSVersion.VersionString; process_architecture=[Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString(); utc=[DateTime]::UtcNow.ToString('o')}
if ($times.Count -gt 0) { $summary.p95_ms=$times[[int][Math]::Ceiling(0.95*$times.Count)-1] }
$summary | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $session 'summary.json') -Encoding utf8
$summary | ConvertTo-Json
Write-Output "Raw evidence: $session"
if ($summary.failed -gt 0) { exit 1 }

