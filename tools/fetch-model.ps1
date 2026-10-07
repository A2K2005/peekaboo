# Download the background-removal models (withoutbg Snap, Apache-2.0) into
# runtime/ai, and check their SHA-256 hashes. Windows PowerShell 5.1.
# Source: https://huggingface.co/withoutbg/snap
[CmdletBinding()]
param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path (Split-Path $PSScriptRoot -Parent) 'runtime/ai'
}
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$base = 'https://huggingface.co/withoutbg/snap/resolve/main'
$models = @{
    'depth_anything_v2_vits_slim.onnx' = '396BC234301510F59FD45ADA24CBB72CDC7CF201F6578DFCE76F42B67BC609F7'
    'snap_matting_0.1.0.onnx'          = '094D9B674939CF0F75EDC1DDB768A786345205564E01A727F017A5457335197D'
    'snap_refiner_0.1.0.onnx'          = '9C10A55FCB01F871B35ACC5DB03DC30A8C92A15C0E445D4D8BA3C8F4DA3C3A80'
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
foreach ($name in $models.Keys) {
    $target = Join-Path $OutputDirectory $name
    if ((Test-Path $target) -and (Get-FileHash $target -Algorithm SHA256).Hash -eq $models[$name]) {
        Write-Host "$name is already present."
        continue
    }
    $partial = "$target.download"
    Write-Host "Downloading $name..."
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $partial
    $hash = (Get-FileHash $partial -Algorithm SHA256).Hash
    if ($hash -ne $models[$name]) {
        Remove-Item $partial
        throw "$name has SHA-256 $hash, expected $($models[$name]). The file was not kept."
    }
    Move-Item -Force $partial $target
}
Write-Host "Models are in $OutputDirectory. ONNX Runtime (onnxruntime.dll) must be there too."
