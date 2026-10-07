#requires -Version 5.1
# Self-test for tools/benchmark.ps1 and tools/perf-gate.ps1. Fake apps are PowerShell scripts, so these timings are not app benchmarks.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\artifacts\harness-self-test'))
[void][IO.Directory]::CreateDirectory($root)
$ps = Join-Path $PSHOME 'powershell.exe'
$bench = Join-Path $PSScriptRoot 'benchmark.ps1'
$gate = Join-Path $PSScriptRoot 'perf-gate.ps1'

function Test-Harness($name, $script, $expectedExit, [string[]]$extra = @()) {
    $fake = Join-Path $root "$name.ps1"
    Set-Content -LiteralPath $fake -Value $script
    & $ps -NoProfile -File $bench -Executable $ps -InputFile $fake -Runs 1 -OutputDirectory (Join-Path $root $name) @extra
    if ($LASTEXITCODE -ne $expectedExit) { throw "Harness case '$name' exited with $LASTEXITCODE, expected $expectedExit." }
}
Test-Harness 'valid-marker' '@{first_content_qpc=[Diagnostics.Stopwatch]::GetTimestamp(); qpc_frequency=[Diagnostics.Stopwatch]::Frequency; width=10; height=10; page_count=1} | ConvertTo-Json | Set-Content -LiteralPath $env:PFW_BENCH_OUT' 0
Test-Harness 'missing-marker' 'exit 0' 1
Test-Harness 'invalid-marker' '@{first_content_qpc=1; qpc_frequency=1; width=0; height=0; page_count=0} | ConvertTo-Json | Set-Content -LiteralPath $env:PFW_BENCH_OUT' 1
Test-Harness 'timeout' 'Start-Sleep -Seconds 10' 1 @('-TimeoutSeconds', '1')

function Test-Gate($name, $baseline, $current, $expectedExit) {
    $doc = { param($values) @{ metrics = @($values.GetEnumerator() | ForEach-Object { @{ name = $_.Key; unit = 'ms'; value = $_.Value; prd_target_text = 'test' } }) } | ConvertTo-Json -Depth 4 }
    $baselineFile = Join-Path $root "gate-$name-baseline.json"; Set-Content -LiteralPath $baselineFile -Value (& $doc $baseline)
    $currentFile = Join-Path $root "gate-$name-current.json"; Set-Content -LiteralPath $currentFile -Value (& $doc $current)
    & $ps -NoProfile -File $gate -BaselineFile $baselineFile -ResultsFile $currentFile
    if ($LASTEXITCODE -ne $expectedExit) { throw "Gate case '$name' exited with $LASTEXITCODE, expected $expectedExit." }
}
Test-Gate 'within-10-percent' @{ a = 100; b = $null } @{ a = 110; b = 5 } 0
Test-Gate 'over-10-percent' @{ a = 100 } @{ a = 110.1 } 1
Test-Gate 'not-measured' @{ a = 100 } @{ a = $null } 1
Test-Gate 'metric-removed' @{ a = 100; b = 100 } @{ a = 100 } 1

Write-Host 'Self-test passed. The harness accepts a valid marker and rejects a missing marker, an invalid marker, and a timeout. The gate passes a 10% change and fails a larger regression, a missing value, and a removed metric.'
