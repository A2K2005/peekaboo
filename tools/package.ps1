#requires -Version 5.1
# Builds dist\Peekaboo-*-Windows-x64.zip and an unsigned dist\Peekaboo-*-x64.msix from the release build.
# Steps and owner tasks: docs/packaging.md.
[CmdletBinding()]
param(
    [switch]$IncludeAiPack,
    [switch]$SkipMsix,
    # Signs the MSIX with a self-signed test certificate in Cert:\CurrentUser\My, created on first use.
    # Without this switch, the script never reads or creates a certificate.
    [switch]$Sign,
    [string]$WindowsSdkBin = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64'
)
$ErrorActionPreference = 'Stop'
if ($IncludeAiPack) { throw 'The AI pack remains a local evaluation dependency. Its distribution review and exact model/runtime notices must be completed before packaging it.' }
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe = Join-Path $root 'target\release\peekaboo.exe'
$pdf = Join-Path $root 'runtime\pdfium.dll'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Build the release executable first.' }
if (-not (Test-Path -LiteralPath $pdf)) { throw 'Run tools/fetch-pdfium.ps1 first.' }
$pdfProvenance = Get-Content -LiteralPath (Join-Path $root 'runtime\x64\provenance.json') -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $pdf -Algorithm SHA256).Hash -ine $pdfProvenance.dll_sha256) { throw 'PDFium DLL does not match its pinned provenance. Fetch the verified runtime again.' }

# Never reuse or delete an older output folder; add a time stamp instead.
$destination = Join-Path $root 'dist\Peekaboo'
if (Test-Path -LiteralPath $destination) {
    $destination = Join-Path $root ('dist\Peekaboo-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff'))
}
[IO.Directory]::CreateDirectory($destination) | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $destination 'peekaboo.exe')
Copy-Item -LiteralPath $pdf -Destination (Join-Path $destination 'pdfium.dll')
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $destination
Copy-Item -LiteralPath (Join-Path $root 'THIRD-PARTY-NOTICES.md') -Destination $destination
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'register-file-associations.ps1') -Destination $destination
$notices = Join-Path $destination 'licenses'
$pdfiumNotices = Join-Path $notices 'pdfium'
[IO.Directory]::CreateDirectory($pdfiumNotices) | Out-Null
# runtime\ is a shared junction: read single files from it, never copy or delete it recursively.
Get-ChildItem -LiteralPath (Join-Path $root 'runtime\x64\licenses') -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $pdfiumNotices }
Copy-Item -LiteralPath (Join-Path $root 'runtime\x64\provenance.json') -Destination (Join-Path $notices 'pdfium-provenance.json')

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
$metadataText = & $cargoPath metadata --offline --locked --manifest-path (Join-Path $root 'Cargo.toml') --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Could not read locked dependency metadata.' }
$metadata = ($metadataText -join "`n") | ConvertFrom-Json
# Audit the known native binding gap first so its exact vendored notices and
# actionable failure are not hidden by an unrelated crate audit failure.
$packages = $metadata.packages | Sort-Object @{ Expression = { if ($_.name -eq 'libwebp-sys') { 0 } else { 1 } } }, name
foreach ($package in $packages) {
    if ($package.name -eq 'peekaboo') { continue }
    if (-not $package.license -or $package.license -match '(?<!L)GPL|AGPL') { throw "Review license for $($package.name): $($package.license)" }
    $licenseDirectory = Join-Path $notices "$($package.name)-$($package.version)"
    [IO.Directory]::CreateDirectory($licenseDirectory) | Out-Null
    $packageDirectory = Split-Path $package.manifest_path -Parent
    $licenseFiles = Get-ChildItem -LiteralPath $packageDirectory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE|license)' }
    if ($package.name -eq 'libwebp-sys') {
        # The published binding declares MIT in Cargo.toml but 0.14.4 contains no
        # binding license text or copyright notice. Preserve its exact declaration
        # and source revision plus every notice shipped for the vendored libwebp.
        foreach ($item in @(
            @('Cargo.toml.orig', 'binding-Cargo.toml.orig'),
            @('.cargo_vcs_info.json', 'binding-vcs-info.json'),
            @('vendor\COPYING', 'libwebp-COPYING'),
            @('vendor\PATENTS', 'libwebp-PATENTS')
        )) {
            $source = Join-Path $packageDirectory $item[0]
            if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
                throw "libwebp-sys $($package.version) is missing published provenance or vendor notice $($item[0])."
            }
            Copy-Item -LiteralPath $source -Destination (Join-Path $licenseDirectory $item[1])
        }
        @(
            "Package: $($package.name) $($package.version)"
            "Source: $($package.repository)"
            "Published Cargo license declaration: $($package.license)"
            'The published crate does not contain a license text or copyright notice for the Rust binding source.'
            'The copied Cargo manifest and VCS metadata are provenance records, not substitutes for the missing binding notice.'
            'The libwebp COPYING and PATENTS files apply to the vendored libwebp source.'
        ) | Set-Content -LiteralPath (Join-Path $licenseDirectory 'BINDING-LICENSE-STATUS.txt') -Encoding utf8
        if (-not $licenseFiles) {
            throw "Cannot package libwebp-sys $($package.version): its published Cargo manifest declares $($package.license), but the crate contains no binding LICENSE, LICENCE, COPYING, COPYRIGHT, or NOTICE file. Exact Cargo/VCS provenance and vendored libwebp COPYING/PATENTS were retained in $licenseDirectory, but they do not supply the binding's missing copyright and permission notice. Replace the binding or obtain the exact upstream notice before release."
        }
    }
    if (-not $licenseFiles) { throw "No license texts found for $($package.name)." }
    foreach ($file in $licenseFiles) { Copy-Item -LiteralPath $file.FullName -Destination $licenseDirectory }
}
$rustc = Join-Path (Split-Path $cargoPath -Parent) 'rustc.exe'
$sysroot = & $rustc --print sysroot
$rustNotices = Join-Path $sysroot 'share\doc\rust'
Copy-Item -LiteralPath (Join-Path $rustNotices 'COPYRIGHT-library.html') -Destination (Join-Path $notices 'Rust-standard-library.html')
Copy-Item -LiteralPath (Join-Path $rustNotices 'licenses') -Destination (Join-Path $notices 'rust') -Recurse

$files = Get-ChildItem -LiteralPath $destination -File -Recurse
# ZIP stores times from 1980 on; some crates ship files dated 1970.
$files | Where-Object { $_.LastWriteTime.Year -lt 1980 } | ForEach-Object { $_.LastWriteTime = Get-Date }
$manifest = foreach ($file in $files) {
    [ordered]@{ path = $file.FullName.Substring($destination.Length + 1); bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8
$archive = $destination + '-Windows-x64.zip'
Compress-Archive -LiteralPath $destination -DestinationPath $archive
$result = [ordered]@{ directory = $destination; zip = $archive; zip_bytes = (Get-Item -LiteralPath $archive).Length; includes_ai = [bool]$IncludeAiPack }
if ($SkipMsix) { $result | ConvertTo-Json; return }

# MSIX: the ZIP payload without the registration script (the manifest declares file types),
# plus generated logos and the manifest with the Cargo version.
$makeappx = Join-Path $WindowsSdkBin 'makeappx.exe'
$signtool = Join-Path $WindowsSdkBin 'signtool.exe'
if (-not (Test-Path -LiteralPath $makeappx)) { throw "makeappx.exe not found in $WindowsSdkBin. Install the Windows SDK or pass -WindowsSdkBin." }
$layout = $destination + '-msix-layout'
[IO.Directory]::CreateDirectory($layout) | Out-Null
Copy-Item -LiteralPath (Join-Path $destination 'peekaboo.exe') -Destination $layout
Copy-Item -LiteralPath (Join-Path $destination 'pdfium.dll') -Destination $layout
Copy-Item -LiteralPath (Join-Path $destination 'THIRD-PARTY-NOTICES.md') -Destination $layout
Copy-Item -LiteralPath $notices -Destination (Join-Path $layout 'licenses') -Recurse
$assets = Join-Path $layout 'Assets'
[IO.Directory]::CreateDirectory($assets) | Out-Null
Add-Type -AssemblyName System.Drawing
function New-Logo([string]$Path, [int]$Size) {
    # Placeholder art: a blue rounded square with a white P.
    $bitmap = New-Object System.Drawing.Bitmap $Size, $Size
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $inset = [Math]::Max(1, [int]($Size * 0.08)); $side = $Size - 2 * $inset; $radius = [int]($side * 0.22)
        $shape = New-Object System.Drawing.Drawing2D.GraphicsPath
        $shape.AddArc($inset, $inset, 2 * $radius, 2 * $radius, 180, 90)
        $shape.AddArc($inset + $side - 2 * $radius, $inset, 2 * $radius, 2 * $radius, 270, 90)
        $shape.AddArc($inset + $side - 2 * $radius, $inset + $side - 2 * $radius, 2 * $radius, 2 * $radius, 0, 90)
        $shape.AddArc($inset, $inset + $side - 2 * $radius, 2 * $radius, 2 * $radius, 90, 90)
        $shape.CloseFigure()
        $graphics.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 0, 103, 192))), $shape)
        $font = New-Object System.Drawing.Font 'Segoe UI', ([float]($side * 0.55)), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
        $format = New-Object System.Drawing.StringFormat
        $format.Alignment = [System.Drawing.StringAlignment]::Center; $format.LineAlignment = [System.Drawing.StringAlignment]::Center
        $graphics.DrawString('P', $font, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF 0, 0, $Size, $Size), $format)
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
New-Logo (Join-Path $assets 'Square44x44Logo.png') 44
New-Logo (Join-Path $assets 'Square150x150Logo.png') 150
New-Logo (Join-Path $assets 'StoreLogo.png') 50
$version = ((Get-Content -LiteralPath (Join-Path $root 'Cargo.toml')) -match '^version\s*=' | Select-Object -First 1) -replace '.*"(.+)".*', '$1'
$appx = New-Object System.Xml.XmlDocument
$appx.PreserveWhitespace = $true
$appx.Load((Join-Path $root 'packaging\AppxManifest.xml'))
$appx.Package.Identity.Version = "$version.0"
$appx.Save((Join-Path $layout 'AppxManifest.xml'))
$msix = $destination + '-x64.msix'
# makeappx validates the manifest against the schema while packing.
& $makeappx pack /o /h SHA256 /d $layout /p $msix | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makeappx could not pack or validate the MSIX.' }
$signed = $false
if ($Sign) {
    $publisher = $appx.Package.Identity.Publisher
    $certificate = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $publisher -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date).AddDays(1) } | Select-Object -First 1
    if (-not $certificate) {
        # Self-signed code-signing certificate for local tests only. Windows does not trust it, so the package will not install as is.
        $certificate = New-SelfSignedCertificate -Type Custom -KeyUsage DigitalSignature -CertStoreLocation 'Cert:\CurrentUser\My' -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}') -Subject $publisher -FriendlyName 'Peekaboo test signing'
    }
    & $signtool sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $msix | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'signtool could not sign the MSIX.' }
    $signed = $true
    $result.certificate_thumbprint = $certificate.Thumbprint
}
# Round trip: unpack and compare the file list with the layout.
$unpacked = $destination + '-msix-check'
& $makeappx unpack /o /p $msix /d $unpacked | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makeappx could not unpack the MSIX.' }
$expected = Get-ChildItem -LiteralPath $layout -File -Recurse | ForEach-Object { $_.FullName.Substring($layout.Length + 1) } | Sort-Object
$actual = Get-ChildItem -LiteralPath $unpacked -File -Recurse | ForEach-Object { $_.FullName.Substring($unpacked.Length + 1) } | Where-Object { $_ -notmatch '^(AppxBlockMap\.xml|AppxSignature\.p7x|\[Content_Types\]\.xml|AppxMetadata\\.*)$' } | Sort-Object
if (Compare-Object $expected $actual) { throw 'The MSIX file list does not match its layout.' }
$result.msix = $msix
$result.msix_bytes = (Get-Item -LiteralPath $msix).Length
$result.msix_signed = $signed
$result | ConvertTo-Json
