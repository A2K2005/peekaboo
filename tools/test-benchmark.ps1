#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$root = Join-Path $PSScriptRoot '../artifacts/harness-self-test'
[IO.Directory]::CreateDirectory([IO.Path]::GetFullPath($root)) | Out-Null
$fake = Join-Path $root 'successful-marker.ps1'
'@{first_content_qpc=[Diagnostics.Stopwatch]::GetTimestamp(); qpc_frequency=[Diagnostics.Stopwatch]::Frequency; width=10; height=10; page_count=1} | ConvertTo-Json | Set-Content -LiteralPath $env:PFW_BENCH_OUT' | Set-Content -LiteralPath $fake
$pwsh = (Get-Process -Id $PID).Path
& $pwsh -NoProfile -File (Join-Path $PSScriptRoot 'benchmark.ps1') -Executable $pwsh -InputFile $fake -Runs 2 -OutputDirectory (Join-Path $root 'success')
if ($LASTEXITCODE -ne 0) { throw 'Valid marker test failed.' }
$missing = Join-Path $root 'missing-marker.ps1'
'exit 0' | Set-Content -LiteralPath $missing
& $pwsh -NoProfile -File (Join-Path $PSScriptRoot 'benchmark.ps1') -Executable $pwsh -InputFile $missing -Runs 1 -OutputDirectory (Join-Path $root 'missing')
if ($LASTEXITCODE -ne 1) { throw 'Missing marker was not rejected.' }
$invalid = Join-Path $root 'invalid-marker.ps1'
'@{first_content_qpc=1; qpc_frequency=1; width=0; height=0; page_count=0} | ConvertTo-Json | Set-Content -LiteralPath $env:PFW_BENCH_OUT' | Set-Content -LiteralPath $invalid
& $pwsh -NoProfile -File (Join-Path $PSScriptRoot 'benchmark.ps1') -Executable $pwsh -InputFile $invalid -Runs 1 -OutputDirectory (Join-Path $root 'invalid')
if ($LASTEXITCODE -ne 1) { throw 'Invalid marker was not rejected.' }
$slow = Join-Path $root 'timeout-marker.ps1'
'Start-Sleep -Seconds 10' | Set-Content -LiteralPath $slow
& $pwsh -NoProfile -File (Join-Path $PSScriptRoot 'benchmark.ps1') -Executable $pwsh -InputFile $slow -Runs 1 -TimeoutSeconds 1 -OutputDirectory (Join-Path $root 'timeout')
if ($LASTEXITCODE -ne 1) { throw 'Timeout was not rejected.' }
Write-Output 'Harness self-test passed: valid markers accepted; missing/invalid markers and timeout rejected. These timings are not app benchmarks.'
