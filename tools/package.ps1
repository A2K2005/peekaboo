#requires -Version 7.0
[CmdletBinding()]
param([switch]$IncludeAiPack)
$ErrorActionPreference = 'Stop'
if ($IncludeAiPack) { throw 'The AI pack remains a local evaluation dependency. Its distribution review and exact model/runtime notices must be completed before packaging it.' }
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe = Join-Path $root 'target/release/preview-for-windows.exe'
$pdf = Join-Path $root 'runtime/pdfium.dll'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Build the release executable first.' }
if (-not (Test-Path -LiteralPath $pdf)) { throw 'Run tools/fetch-pdfium.ps1 first.' }
$pdfProvenance = Get-Content -LiteralPath (Join-Path $root 'runtime/x64/provenance.json') -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $pdf -Algorithm SHA256).Hash -ine $pdfProvenance.dll_sha256) { throw 'PDFium DLL does not match its pinned provenance. Fetch the verified runtime again.' }
$destination = Join-Path $root 'dist/Preview'
if (Test-Path -LiteralPath $destination) {
    $destination = Join-Path $root ('dist/Preview-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff'))
}
[IO.Directory]::CreateDirectory($destination) | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $destination 'preview-for-windows.exe') -Force
Copy-Item -LiteralPath $pdf -Destination (Join-Path $destination 'pdfium.dll') -Force
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $destination -Force
Copy-Item -LiteralPath (Join-Path $root 'THIRD-PARTY-NOTICES.md') -Destination $destination -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'register-file-associations.ps1') -Destination $destination -Force
$notices = Join-Path $destination 'licenses'
[IO.Directory]::CreateDirectory($notices) | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'runtime/x64/licenses') -Destination (Join-Path $notices 'pdfium') -Recurse -Force
Copy-Item -LiteralPath (Join-Path $root 'runtime/x64/provenance.json') -Destination (Join-Path $notices 'pdfium-provenance.json') -Force
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
$metadataText = & $cargoPath metadata --offline --locked --manifest-path (Join-Path $root 'Cargo.toml') --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Could not read locked dependency metadata.' }
$metadata = $metadataText | ConvertFrom-Json
foreach ($package in $metadata.packages) {
    if ($package.name -eq 'preview-for-windows') { continue }
    if (-not $package.license -or $package.license -match '(?<!L)GPL|AGPL') { throw "Review license for $($package.name): $($package.license)" }
    $licenseDirectory = Join-Path $notices "$($package.name)-$($package.version)"
    [IO.Directory]::CreateDirectory($licenseDirectory) | Out-Null
    $packageDirectory = Split-Path $package.manifest_path -Parent
    $licenseFiles = Get-ChildItem -LiteralPath $packageDirectory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE|license)' }
    if (-not $licenseFiles) { throw "No license texts found for $($package.name)." }
    foreach ($file in $licenseFiles) { Copy-Item -LiteralPath $file.FullName -Destination $licenseDirectory -Force }
}
$rustc = Join-Path (Split-Path $cargoPath -Parent) 'rustc.exe'
$sysroot = & $rustc --print sysroot
$rustNotices = Join-Path $sysroot 'share/doc/rust'
Copy-Item -LiteralPath (Join-Path $rustNotices 'COPYRIGHT-library.html') -Destination (Join-Path $notices 'Rust-standard-library.html') -Force
Copy-Item -LiteralPath (Join-Path $rustNotices 'licenses') -Destination (Join-Path $notices 'rust') -Recurse -Force
$files = Get-ChildItem -LiteralPath $destination -File -Recurse
$manifest = foreach ($file in $files) {
    [ordered]@{ path=[IO.Path]::GetRelativePath($destination,$file.FullName); bytes=$file.Length; sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8
$archive = $destination + '-Windows-x64.zip'
Compress-Archive -LiteralPath $destination -DestinationPath $archive -Force
[ordered]@{ directory=$destination; zip=$archive; zip_bytes=(Get-Item -LiteralPath $archive).Length; includes_ai=[bool]$IncludeAiPack; signed=$false } | ConvertTo-Json
