# Interleaved launch comparison (A, B, A, B) of two builds, using the
# PFW_BENCH_OUT and PFW_BENCH_AUTOCLOSE contract in docs/contracts.md.
# Interleaving spreads background load (parallel builds) over both builds.
# Windows PowerShell 5.1. Opens windows: run only in a GUI session that may
# be disturbed. These are process-relaunch timings, not cold launches.
#
#   powershell -NoProfile -File tests/ui/launch-ab.ps1 -A artifacts/baseline/peekaboo.exe -B target/release/peekaboo.exe -InputFile fixtures/500-pages-50mb.pdf -Runs 20
param(
    [Parameter(Mandatory)][string]$A,
    [Parameter(Mandatory)][string]$B,
    [Parameter(Mandatory)][string]$InputFile,
    [int]$Runs = 20,
    [string]$OutDir = (Join-Path $PSScriptRoot ('../../artifacts/launch-ab/' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmss')))
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$inputPath = (Resolve-Path -LiteralPath $InputFile).Path
$builds = @{ A = (Resolve-Path -LiteralPath $A).Path; B = (Resolve-Path -LiteralPath $B).Path }
$rows = @()
for ($i = 1; $i -le $Runs; $i++) {
    foreach ($label in 'A', 'B') {
        $marker = Join-Path $OutDir "$label-$i.json"
        $psi = New-Object Diagnostics.ProcessStartInfo
        $psi.FileName = $builds[$label]
        $psi.Arguments = '"' + $inputPath + '"'
        $psi.UseShellExecute = $false
        $psi.EnvironmentVariables['PFW_BENCH_OUT'] = $marker
        $psi.EnvironmentVariables['PFW_BENCH_AUTOCLOSE'] = '1'
        $start = [Diagnostics.Stopwatch]::GetTimestamp()
        $p = [Diagnostics.Process]::Start($psi)
        if (-not $p.WaitForExit(15000)) { $p.Kill(); $ms = $null; $status = 'timeout' }
        elseif (-not (Test-Path -LiteralPath $marker)) { $ms = $null; $status = "no marker, exit $($p.ExitCode)" }
        else {
            $m = Get-Content -Raw -LiteralPath $marker | ConvertFrom-Json
            $ms = 1000.0 * ($m.first_content_qpc - $start) / $m.qpc_frequency
            $status = 'ok'
        }
        $rows += [pscustomobject]@{ run = $i; build = $label; ms = $ms; status = $status }
    }
}
$rows | Export-Csv -NoTypeInformation -LiteralPath (Join-Path $OutDir 'samples.csv')
$summary = foreach ($label in 'A', 'B') {
    $t = @($rows | Where-Object { $_.build -eq $label -and $_.status -eq 'ok' } | ForEach-Object { $_.ms } | Sort-Object)
    $n = $t.Count
    if ($n -eq 0) { "$label ($($builds[$label])): no successful runs"; continue }
    $median = if ($n % 2) { $t[[int][Math]::Floor($n / 2)] } else { ($t[$n / 2 - 1] + $t[$n / 2]) / 2 }
    $p95 = $t[[int][Math]::Ceiling(0.95 * $n) - 1]
    '{0}: n={1} min={2:N1} median={3:N1} p95={4:N1} max={5:N1} ms ({6})' -f $label, $n, $t[0], $median, $p95, $t[$n - 1], $builds[$label]
}
$summary | Set-Content -LiteralPath (Join-Path $OutDir 'summary.txt')
$summary
Write-Output "Raw samples: $OutDir"
