#requires -Version 5.1
<#
.SYNOPSIS
Runs the performance suite and compares it with benchmarks/baseline.json.

.DESCRIPTION
Prints one table and exits 1 if any metric with a baseline value regresses by more than 10%,
has no current value, or is missing from the results. Latency compares p95, memory compares
the maximum over runs, and sizes compare bytes. PRD targets are shown for reference only:
they apply to the PRD reference laptop, not to this PC.
Pass -ResultsFile to compare an existing tools/perf/run-suite.ps1 result without running the suite.
#>
[CmdletBinding()]
param(
    [ValidateRange(1,1000)][int]$Runs = 30,
    [string]$BaselineFile,
    [string]$ResultsFile
)
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 leaves $PSScriptRoot empty in param defaults of advanced scripts.
if (-not $BaselineFile) { $BaselineFile = Join-Path $PSScriptRoot '..\benchmarks\baseline.json' }
if (-not $ResultsFile) {
    $ResultsFile = & (Join-Path $PSScriptRoot 'perf\run-suite.ps1') -Runs $Runs | Select-Object -Last 1
}
$baseline = @((Get-Content -Raw -LiteralPath $BaselineFile | ConvertFrom-Json).metrics)
$current = @((Get-Content -Raw -LiteralPath $ResultsFile | ConvertFrom-Json).metrics)

function Format-Value($value, $unit) {
    if ($null -eq $value) { return '-' }
    if ($unit -eq 'bytes') { return '{0:N2} MB' -f ($value / 1e6) }
    '{0:N1} {1}' -f $value, $unit
}

$failures = 0
$names = @($current | ForEach-Object name) + @($baseline | ForEach-Object name | Where-Object { $_ -notin @($current | ForEach-Object name) })
$rows = foreach ($name in $names) {
    $now = $current | Where-Object name -eq $name | Select-Object -First 1
    $base = $baseline | Where-Object name -eq $name | Select-Object -First 1
    $unit = if ($now) { $now.unit } else { $base.unit }
    $b = if ($base) { $base.value } else { $null }
    $c = if ($now) { $now.value } else { $null }
    $change = ''
    if ($null -eq $b) { $result = if ($null -eq $c) { 'not measured' } else { 'no baseline' } }
    elseif (-not $now) { $result = 'FAIL: missing from results'; $failures++ }
    elseif ($null -eq $c) { $result = 'FAIL: not measured'; $failures++ }
    else {
        $ratio = ($c - $b) / $b
        $change = '{0:+0.0;-0.0;0.0}%' -f (100 * $ratio)
        if ($ratio -gt 0.10) { $result = 'FAIL'; $failures++ } else { $result = 'pass' }
    }
    [pscustomobject][ordered]@{
        Metric = $name
        Baseline = Format-Value $b $unit
        Current = Format-Value $c $unit
        Change = $change
        'PRD target' = if ($now) { $now.prd_target_text } else { $base.prd_target_text }
        Result = $result
    }
}
Write-Host ($rows | Format-Table -AutoSize | Out-String -Width 300).Trim()
foreach ($note in @($current | Where-Object note | ForEach-Object { "$($_.name): $($_.note)" })) { Write-Host "Note: $note" }
Write-Host "Results: $ResultsFile"
if ($failures -gt 0) {
    Write-Host "Gate failed: $failures metric(s) regressed more than 10% or were not measured."
    exit 1
}
Write-Host 'Gate passed: no metric with a baseline regressed more than 10%.'
