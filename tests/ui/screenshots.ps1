# Captures the window in each theme with PrintWindow, without changing
# system settings (PFW_THEME). Windows PowerShell 5.1. Opens windows: run
# only in a GUI session that may be disturbed.
#
#   powershell -NoProfile -File tests/ui/screenshots.ps1 -Executable target/release/peekaboo.exe -InputFile fixtures/20-pages.pdf
#
# Output: artifacts/screenshots/<theme>-document.png and <theme>-empty.png.
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$InputFile,
    [string]$OutDir = (Join-Path $PSScriptRoot '../../artifacts/screenshots'),
    [int]$TimeoutSeconds = 20
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if (-not ('PfwCapture' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PfwCapture {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr value);
}
"@
}
# Per-monitor aware, so window sizes come back in physical pixels.
[void][PfwCapture]::SetProcessDpiAwarenessContext([IntPtr]-4)
New-Item -ItemType Directory -Force $OutDir | Out-Null
$exe = (Resolve-Path -LiteralPath $Executable).Path
$file = (Resolve-Path -LiteralPath $InputFile).Path

function Capture([string]$theme, [string]$path, [string]$name) {
    $marker = Join-Path $OutDir ('marker-' + [Guid]::NewGuid().ToString('N') + '.json')
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    if ($path) { $psi.Arguments = '"' + $path + '"' }
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['PFW_THEME'] = $theme
    $psi.EnvironmentVariables['PFW_BENCH_OUT'] = $marker
    $p = [Diagnostics.Process]::Start($psi)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            $p.Refresh()
            if ($p.MainWindowHandle -ne [IntPtr]::Zero -and ((-not $path) -or (Test-Path -LiteralPath $marker))) { break }
            Start-Sleep -Milliseconds 100
        }
        Start-Sleep -Milliseconds 500
        $rect = New-Object PfwCapture+RECT
        [void][PfwCapture]::GetWindowRect($p.MainWindowHandle, [ref]$rect)
        $bitmap = New-Object Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $hdc = $graphics.GetHdc()
        # PW_RENDERFULLCONTENT (2) includes Direct2D content.
        [void][PfwCapture]::PrintWindow($p.MainWindowHandle, $hdc, 2)
        $graphics.ReleaseHdc($hdc)
        $out = Join-Path $OutDir "$theme-$name.png"
        $bitmap.Save($out, [Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose(); $bitmap.Dispose()
        Write-Output "Wrote $out"
    } finally {
        if (-not $p.HasExited) { [void]$p.CloseMainWindow(); [void]$p.WaitForExit(3000) }
        if (-not $p.HasExited) { $p.Kill() }
        Remove-Item -LiteralPath $marker -ErrorAction SilentlyContinue
    }
}

foreach ($theme in 'light', 'dark', 'contrast') {
    Capture $theme $file 'document'
    Capture $theme '' 'empty'
}
