param([ValidateSet('x64','arm64')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$release = 'chromium/8086'
$asset = "pdfium-win-$Architecture.tgz"
$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root 'runtime'
$destination = Join-Path $runtime $Architecture
New-Item -ItemType Directory -Force $destination | Out-Null
$url = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8086/$asset"
$digestSource = 'Locally recorded hash from official release HTTPS download; not independent publisher attestation'
if ($Architecture -eq 'x64') {
    $expected = '1fd8af952832dbb0eb16d9249f68fe09e5f5ebf7c3dd9f6066ea2720cc28487d'
} else {
    $metadata = Invoke-RestMethod "https://api.github.com/repos/bblanchon/pdfium-binaries/releases/tags/chromium%2F8086"
    $entry = $metadata.assets | Where-Object name -EQ $asset
    if (-not $entry.digest -or -not $entry.digest.StartsWith('sha256:')) { throw 'Pinned asset has no SHA-256 metadata.' }
    $expected = $entry.digest.Substring(7)
    $digestSource = 'GitHub release asset metadata'
}
$archive = Join-Path $destination $asset
if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest $url -OutFile $archive }
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($hash -ne $expected) { throw 'PDFium archive digest mismatch.' }
tar -xzf $archive -C $destination
if ($LASTEXITCODE -ne 0) { throw 'PDFium extraction failed.' }
$dll = Join-Path $destination 'bin/pdfium.dll'
if (-not (Test-Path -LiteralPath $dll)) { throw 'PDFium DLL is absent from the verified archive.' }
# The complete archive, headers, and license directory stay together for audit.
@{
    release = $release; architecture = $Architecture; source = $url
    archive_sha256 = $hash; dll_sha256 = (Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash.ToLowerInvariant()
    fetched_utc = [DateTime]::UtcNow.ToString('o'); digest_source = $digestSource
    build = 'non-V8'; upstream = 'https://github.com/bblanchon/pdfium-binaries'
} | ConvertTo-Json | Set-Content (Join-Path $destination 'provenance.json') -Encoding utf8
if ($Architecture -eq 'x64') {
    Copy-Item -LiteralPath $dll -Destination (Join-Path $runtime 'pdfium.dll') -Force
    foreach ($profile in @('debug','release')) {
        $output = Join-Path $root "target/$profile"
        if (Test-Path -LiteralPath $output) { Copy-Item -LiteralPath $dll -Destination (Join-Path $output 'pdfium.dll') -Force }
    }
}
Write-Output "Verified $release $asset SHA-256 $hash. Notices retained in $destination."
