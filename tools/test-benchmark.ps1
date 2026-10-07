#requires -Version 5.1
# Self-test for tools/benchmark.ps1. Fake apps are PowerShell scripts, so these timings are not app benchmarks.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\artifacts\harness-self-test'))
[void][IO.Directory]::CreateDirectory($root)
$ps = Join-Path $PSHOME 'powershell.exe'
$bench = Join-Path $PSScriptRoot 'benchmark.ps1'

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

Write-Host 'Self-test passed. The harness accepts a valid marker and rejects a missing marker, an invalid marker, and a timeout.'
