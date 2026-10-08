#requires -Version 5.1
<#
.SYNOPSIS
Runs the performance suite and writes results.json. Returns the path of results.json.

.DESCRIPTION
Measures launch to first content for four fixtures, memory with the 20-page PDF after 2 s idle,
the release executable size, and the package ZIP size from tools/package.ps1.
Use -Runs 59 for release gates: with 59 runs, the largest sample is a 95% upper confidence
bound on p95 (docs/research/winui-performance.md, section 7).
To record a baseline, copy results.json to benchmarks/baseline.json.
Exits 1 if any app run failed.
#>
[CmdletBinding()]
param(
    [ValidateRange(1,1000)][int]$Runs = 30,
    [string]$Executable,
    [string]$FixturesDirectory,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in param defaults of advanced scripts.
if (-not $Executable) { $Executable = Join-Path $root 'target\release\peekaboo.exe' }
if (-not $FixturesDirectory) { $FixturesDirectory = Join-Path $root 'fixtures' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $root 'artifacts\perf' }
$exe = (Resolve-Path -LiteralPath $Executable).Path
$out = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) ([DateTime]::UtcNow.ToString('yyyyMMddTHHmmss'))
[void][IO.Directory]::CreateDirectory($out)
$metrics = New-Object Collections.Generic.List[object]
$appFailed = $false

function Add-Metric($name, $unit, $value, $target, $note) {
    $metrics.Add([pscustomobject][ordered]@{ name = $name; unit = $unit; value = $value; prd_target_text = $target; note = $note })
}

function Invoke-Benchmark($name, $fixture, [hashtable]$extra = @{}) {
    $dir = Join-Path $out $name
    & (Join-Path $root 'tools\benchmark.ps1') -Executable $exe -InputFile (Join-Path $FixturesDirectory $fixture) -Runs $Runs -OutputDirectory $dir @extra
    Get-Content -Raw -LiteralPath (Get-ChildItem -LiteralPath $dir -Recurse -Filter summary.json).FullName | ConvertFrom-Json
}

$launches = @(
    @('launch_20_pages_pdf_p95', '20-pages.pdf', 'under 400 ms (cold launch to first page, p95)'),
    @('launch_500_pages_pdf_p95', '500-pages-50mb.pdf', 'under 300 ms (500-page PDF, first page)'),
    @('launch_image_24mp_p95', 'image-24mp.jpg', 'under 400 ms (cold launch to first image, p95)'),
    @('launch_image_small_p95', 'image-small.png', 'under 400 ms (cold launch to first image, p95)')
)
foreach ($launch in $launches) {
    $s = Invoke-Benchmark $launch[0] $launch[1]
    if ($s.failed -gt 0) { $appFailed = $true; Add-Metric $launch[0] 'ms' $null $launch[2] "$($s.failed) of $Runs runs failed. See $($s.evidence)." }
    else { Add-Metric $launch[0] 'ms' $s.first_content_ms.p95 $launch[2] $null }
}

$s = Invoke-Benchmark 'memory_20_pages_pdf' '20-pages.pdf' @{ IdleSeconds = 2 }
$memoryNote = if ($s.failed -gt 0) { $appFailed = $true; "$($s.failed) of $Runs runs failed. See $($s.evidence)." }
Add-Metric 'memory_20_pages_pdf_private_bytes_max' 'bytes' $(if (-not $memoryNote) { $s.idle_private_bytes.max }) 'under 120 MB (one 20-page PDF open)' $memoryNote
Add-Metric 'memory_20_pages_pdf_working_set_max' 'bytes' $(if (-not $memoryNote) { $s.idle_working_set_bytes.max }) 'under 120 MB (one 20-page PDF open)' $memoryNote

Add-Metric 'exe_bytes' 'bytes' (Get-Item -LiteralPath $exe).Length 'none (part of the 30 MB download)' $null

# package.ps1 builds dist/ from target/release, so its size matches -Executable only for the default path.
# -SkipMsix: the suite needs only the ZIP, and the MSIX step needs the Windows SDK.
$packageOutput = & { $ErrorActionPreference = 'Continue'; & (Join-Path $PSHOME 'powershell.exe') -NoProfile -File (Join-Path $root 'tools\package.ps1') -SkipMsix 2>&1 | ForEach-Object { "$_" } }
$packageText = $packageOutput -join "`n"
if ($LASTEXITCODE -eq 0 -and $packageText -match '"zip_bytes":\s*(\d+)') {
    Add-Metric 'package_zip_bytes' 'bytes' ([long]$Matches[1]) 'under 30 MB (download without AI model)' $null
} else {
    # The child console wraps error text at its width; join the pieces and drop the "At" and "+" detail lines.
    $reason = (@($packageOutput | Where-Object { $_ -notmatch '^\s*(\+|At )' }) -join '').Trim()
    Add-Metric 'package_zip_bytes' 'bytes' $null 'under 30 MB (download without AI model)' "tools/package.ps1 failed: $reason"
}

$os = Get-CimInstance Win32_OperatingSystem
$results = [pscustomobject][ordered]@{
    recorded_utc = [DateTime]::UtcNow.ToString('o')
    runs = $Runs
    commit = (& git -C $root rev-parse HEAD)
    machine = '{0} {1}; {2}; {3:N0} GB RAM' -f $os.Caption, $os.Version, (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim(), ((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
    executable_sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
    limits = 'Process relaunch with the OS file cache warm, not cold boot. First content is a DWM-flush proxy. PRD targets apply to the reference laptop only.'
    evidence = $out
    metrics = $metrics.ToArray()
}
$resultsFile = Join-Path $out 'results.json'
[IO.File]::WriteAllText($resultsFile, (ConvertTo-Json $results -Depth 5))
foreach ($m in $metrics) { if ($m.note) { Write-Host "$($m.name): $($m.note)" } }
Write-Host "Results: $resultsFile"
$resultsFile
if ($appFailed) { exit 1 }
