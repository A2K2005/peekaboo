#requires -Version 5.1
<#
.SYNOPSIS
Measures process-relaunch time from launch to first content, and memory, for one input file.

.DESCRIPTION
Each run takes a QPC timestamp just before it starts the app. The app writes the QPC of its first
drawn content to the file named by PFW_BENCH_OUT (see docs/contracts.md, "Measurement").
Without -IdleSeconds, PFW_BENCH_AUTOCLOSE=1 makes the app close after the marker.
With -IdleSeconds N, the harness waits N seconds after the marker, reads memory, then asks the
window to close. Each call writes samples.json, samples.csv, and summary.json to a new folder
under -OutputDirectory. The script exits 1 if any run failed.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$InputFile,
    [ValidateRange(1,1000)][int]$Runs = 30,
    [ValidateRange(1,300)][int]$TimeoutSeconds = 15,
    [ValidateRange(0,60)][int]$IdleSeconds = 0,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in param defaults of advanced scripts.
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $PSScriptRoot '..\artifacts\benchmarks' }
if (-not ('PfwMemory' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PfwMemory {
    [StructLayout(LayoutKind.Sequential)] struct Counters {
        public uint cb, faults;
        public UIntPtr peakWorkingSet, workingSet, peakPaged, paged, peakNonPaged, nonPaged, pagefile, peakPagefile, privateUsage;
    }
    [DllImport("psapi.dll", SetLastError=true)] static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
    // Peak working set, peak private bytes, working set, private bytes. Null if the query fails.
    public static long[] Read(IntPtr process) {
        Counters c;
        if (!GetProcessMemoryInfo(process, out c, (uint)Marshal.SizeOf(typeof(Counters)))) return null;
        return new long[] { (long)c.peakWorkingSet.ToUInt64(), (long)c.peakPagefile.ToUInt64(), (long)c.workingSet.ToUInt64(), (long)c.privateUsage.ToUInt64() };
    }
}
"@
}

# Nearest rank: the value at position ceil(percent/100 * n) in the sorted list.
function Get-Stats([object[]]$Values) {
    $sorted = @($Values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($sorted.Count -eq 0) { return $null }
    $rank = { param($percent) $sorted[[int][Math]::Ceiling($percent * $sorted.Count / 100) - 1] }
    [pscustomobject][ordered]@{ p95 = & $rank 95; median = & $rank 50; min = $sorted[0]; max = $sorted[-1] }
}

$exe = (Resolve-Path -LiteralPath $Executable).Path
$inputPath = (Resolve-Path -LiteralPath $InputFile).Path
$mode = if ($IdleSeconds -gt 0) { 'process-relaunch-idle' } else { 'process-relaunch' }
$frequency = [Diagnostics.Stopwatch]::Frequency
$session = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) ([DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8))
[void][IO.Directory]::CreateDirectory($session)

$samples = New-Object Collections.Generic.List[object]
for ($i = 1; $i -le $Runs; $i++) {
    $marker = Join-Path $session "sample-$i.marker.json"
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = '"' + $inputPath + '"'
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['PFW_BENCH_OUT'] = $marker
    if ($IdleSeconds -gt 0) { $psi.EnvironmentVariables.Remove('PFW_BENCH_AUTOCLOSE') } else { $psi.EnvironmentVariables['PFW_BENCH_AUTOCLOSE'] = '1' }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $psi
    $sample = [ordered]@{ run = $i; mode = $mode; status = 'error'; error = $null; first_content_ms = $null; launch_qpc = 0L; first_content_qpc = $null; exit_code = $null
        peak_working_set_bytes = $null; peak_private_bytes = $null; idle_working_set_bytes = $null; idle_private_bytes = $null }
    $started = $false
    try {
        $start = [Diagnostics.Stopwatch]::GetTimestamp()
        $sample.launch_qpc = $start
        [void]$process.Start()
        $started = $true
        $handle = $process.Handle
        $closeRequested = $false
        while (-not $process.WaitForExit(10)) {
            if (([Diagnostics.Stopwatch]::GetTimestamp() - $start) / $frequency -gt $TimeoutSeconds) { throw "The app did not finish within the $TimeoutSeconds-second timeout." }
            if ($IdleSeconds -gt 0 -and -not $closeRequested -and (Test-Path -LiteralPath $marker)) {
                Start-Sleep -Seconds $IdleSeconds
                $memory = [PfwMemory]::Read($handle)
                if ($null -eq $memory) { throw 'Windows could not read the app memory after the idle wait.' }
                $sample.idle_working_set_bytes = $memory[2]
                $sample.idle_private_bytes = $memory[3]
                $process.Refresh()
                if (-not $process.CloseMainWindow()) { throw 'The app window did not accept the close request.' }
                $closeRequested = $true
            }
        }
        $process.WaitForExit()
        # Peak counters stay readable after exit while the harness holds the process handle.
        $memory = [PfwMemory]::Read($handle)
        if ($null -ne $memory) { $sample.peak_working_set_bytes = $memory[0]; $sample.peak_private_bytes = $memory[1] }
        $sample.exit_code = $process.ExitCode
        if ($process.ExitCode -ne 0) { throw "The app exited with code $($process.ExitCode)." }
        if (-not (Test-Path -LiteralPath $marker)) { throw 'The app wrote no first-content marker.' }
        try {
            $m = Get-Content -Raw -LiteralPath $marker | ConvertFrom-Json
            $qpc = [long]$m.first_content_qpc; $markerFrequency = [long]$m.qpc_frequency
            $width = [long]$m.width; $height = [long]$m.height; $pages = [long]$m.page_count
        } catch { throw 'The marker is not JSON with numeric fields.' }
        if ($markerFrequency -ne $frequency -or $qpc -lt $start -or $qpc -gt [Diagnostics.Stopwatch]::GetTimestamp()) { throw 'The marker QPC values are not valid for this run.' }
        if ($width -le 0 -or $height -le 0 -or $pages -le 0) { throw 'The marker has no content size or page count.' }
        $sample.first_content_qpc = $qpc
        $sample.first_content_ms = [Math]::Round(1000.0 * ($qpc - $start) / $frequency, 3)
        $sample.status = 'ok'
    } catch {
        $sample.error = $_.Exception.Message
    } finally {
        if ($started) {
            try { if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() } }
            catch { $sample.error = "$($sample.error) Cleanup failed: $($_.Exception.Message)"; $sample.status = 'error' }
        }
        $process.Dispose()
    }
    $samples.Add([pscustomobject]$sample)
    if ($sample.status -ne 'ok') { Write-Host "Run $i failed: $($sample.error)" }
}

$ok = @($samples | Where-Object status -eq 'ok')
$summary = [pscustomobject][ordered]@{
    mode = $mode
    input = $inputPath
    runs = $Runs
    succeeded = $ok.Count
    failed = $Runs - $ok.Count
    idle_seconds = $IdleSeconds
    first_content_ms = Get-Stats ($ok | ForEach-Object { $_.first_content_ms })
    peak_working_set_bytes = Get-Stats ($ok | ForEach-Object { $_.peak_working_set_bytes })
    peak_private_bytes = Get-Stats ($ok | ForEach-Object { $_.peak_private_bytes })
    idle_working_set_bytes = Get-Stats ($ok | ForEach-Object { $_.idle_working_set_bytes })
    idle_private_bytes = Get-Stats ($ok | ForEach-Object { $_.idle_private_bytes })
    errors = @($samples | Where-Object status -ne 'ok' | ForEach-Object { "Run $($_.run): $($_.error)" })
    limits = 'Process relaunch with the OS file cache warm. Not a cold boot and not a resident warm launch. First content is a DWM-flush proxy, not display scan-out. This PC is not the PRD reference laptop.'
    qpc_frequency = $frequency
    executable_sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
    input_sha256 = (Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    os = [Environment]::OSVersion.VersionString
    utc = [DateTime]::UtcNow.ToString('o')
    evidence = $session
}
[IO.File]::WriteAllText((Join-Path $session 'samples.json'), (ConvertTo-Json -InputObject $samples.ToArray() -Depth 4))
$samples | Export-Csv -LiteralPath (Join-Path $session 'samples.csv') -NoTypeInformation
[IO.File]::WriteAllText((Join-Path $session 'summary.json'), (ConvertTo-Json $summary -Depth 4))

$name = Split-Path $inputPath -Leaf
Write-Host "${name}: $($ok.Count) of $Runs runs succeeded ($mode)."
if ($summary.first_content_ms) {
    $t = $summary.first_content_ms
    Write-Host ('  First content: p95 {0:N1} ms, median {1:N1} ms, min {2:N1} ms, max {3:N1} ms.' -f $t.p95, $t.median, $t.min, $t.max)
}
$labels = [ordered]@{ peak_private_bytes = 'Peak private bytes'; peak_working_set_bytes = 'Peak working set'; idle_private_bytes = 'Private bytes after idle'; idle_working_set_bytes = 'Working set after idle' }
foreach ($field in $labels.Keys) {
    if ($summary.$field) { Write-Host ('  {0}: max {1:N1} MB, median {2:N1} MB.' -f $labels[$field], ($summary.$field.max / 1e6), ($summary.$field.median / 1e6)) }
}
Write-Host "  Raw samples and summary.json: $session"
if ($summary.failed -gt 0) { exit 1 }
