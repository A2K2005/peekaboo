# Proves which formats Windows can decode and encode on this PC through Windows.Graphics.Imaging (a WinRT layer over WIC).
# Run: powershell -ExecutionPolicy Bypass -File winrt_codec_roundtrip.ps1 -Dir <folder with sample.png, sample.webp, libheif_example.heic, libheif_example.avif>
param([Parameter(Mandatory = $true)][string]$Dir)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapEncoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]

$asTaskOp = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
$asTaskAct = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' } | Select-Object -First 1
function Await($op, [Type]$t) { $task = $asTaskOp.MakeGenericMethod($t).Invoke($null, @($op)); $task.Wait(-1) | Out-Null; $task.Result }
function AwaitAction($act) { $task = $asTaskAct.Invoke($null, @($act)); $task.Wait(-1) | Out-Null }
function HResult($e) { $x = $e.Exception; while ($x.InnerException) { $x = $x.InnerException }; '0x{0:X8} {1}' -f $x.HResult, $x.Message.Split("`n")[0].Trim() }

Write-Output '== WinRT decoders'
[Windows.Graphics.Imaging.BitmapDecoder]::GetDecoderInformationEnumerator() | ForEach-Object { '{0,-28} {1}' -f $_.FriendlyName, ($_.FileExtensions -join ',') }
Write-Output '== WinRT encoders'
[Windows.Graphics.Imaging.BitmapEncoder]::GetEncoderInformationEnumerator() | ForEach-Object { '{0,-28} {1}' -f $_.FriendlyName, ($_.FileExtensions -join ',') }

$folder = Await ([Windows.Storage.StorageFolder]::GetFolderFromPathAsync($Dir)) ([Windows.Storage.StorageFolder])
function Decode($name) {
    $file = Await ($folder.GetFileAsync($name)) ([Windows.Storage.StorageFile])
    $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
    try {
        $dec = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
        $px = Await ($dec.GetPixelDataAsync()) ([Windows.Graphics.Imaging.PixelDataProvider])
        $bytes = $px.DetachPixelData()
        return @{ Ok = $true; W = $dec.PixelWidth; H = $dec.PixelHeight; Bytes = $bytes }
    } finally { $stream.Dispose() }
}

Write-Output '== Decode test'
foreach ($n in 'sample.png', 'sample.webp', 'libheif_example.heic', 'libheif_example.avif') {
    try { $r = Decode $n; '{0,-22} OK   {1}x{2}' -f $n, $r.W, $r.H } catch { '{0,-22} FAIL {1}' -f $n, (HResult $_) }
}

Write-Output '== Encode test (1600x1200 BGRA from sample.png)'
$src = Decode 'sample.png'
$encoders = [ordered]@{
    'out.jpg' = [Windows.Graphics.Imaging.BitmapEncoder]::JpegEncoderId
    'out.png' = [Windows.Graphics.Imaging.BitmapEncoder]::PngEncoderId
    'out.tif' = [Windows.Graphics.Imaging.BitmapEncoder]::TiffEncoderId
    'out.gif' = [Windows.Graphics.Imaging.BitmapEncoder]::GifEncoderId
    'out.heic' = [Windows.Graphics.Imaging.BitmapEncoder]::HeifEncoderId
}
foreach ($k in $encoders.Keys) {
    $file = Await ($folder.CreateFileAsync($k, [Windows.Storage.CreationCollisionOption]::ReplaceExisting)) ([Windows.Storage.StorageFile])
    $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::ReadWrite)) ([Windows.Storage.Streams.IRandomAccessStream])
    try {
        $enc = Await ([Windows.Graphics.Imaging.BitmapEncoder]::CreateAsync($encoders[$k], $stream)) ([Windows.Graphics.Imaging.BitmapEncoder])
        $enc.SetPixelData([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Ignore, $src.W, $src.H, 96, 96, $src.Bytes)
        AwaitAction ($enc.FlushAsync())
        '{0,-9} OK   {1} bytes' -f $k, $stream.Size
    } catch { '{0,-9} FAIL {1}' -f $k, (HResult $_) } finally { $stream.Dispose() }
}
$webpId = [Windows.Graphics.Imaging.BitmapEncoder].GetProperty('WebpEncoderId')
'WebpEncoderId property exists: {0}' -f ($null -ne $webpId)
