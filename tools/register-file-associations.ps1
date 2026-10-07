#requires -Version 5.1
# Adds Preview to "Open with", Default apps, and the Explorer verbs Convert, Resize,
# and Combine into PDF for this Windows user. It never changes a default app.
# Keep in step with register_at in src/integration.rs; tests/integration_shell.rs compares the two.
[CmdletBinding()]
param(
    [string]$Executable = (Join-Path $PSScriptRoot 'preview-for-windows.exe'),
    [switch]$Unregister,
    # Key under HKEY_CURRENT_USER. Tests use a scratch key; leave it as Software otherwise.
    [string]$Root = 'Software'
)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
if ([IO.Path]::GetExtension($exe) -ine '.exe') { throw 'Select the Preview executable.' }
$exeName = Split-Path -Leaf $exe
$appName = 'Preview for Windows'
$classes = "$Root\Classes"
$capabilities = "$Root\PreviewForWindows\Capabilities"
$application = "$classes\Applications\$exeName"
$open = '"' + $exe + '" "%1"'
$icon = '"' + $exe + '",0'
$types = @(
    @('.pdf', 'PreviewForWindows.Pdf', 'PDF document'),
    @('.jpg', 'PreviewForWindows.Jpeg', 'JPEG image'),
    @('.jpeg', 'PreviewForWindows.Jpeg', 'JPEG image'),
    @('.png', 'PreviewForWindows.Png', 'PNG image'),
    @('.webp', 'PreviewForWindows.Webp', 'WebP image'),
    @('.heic', 'PreviewForWindows.Heif', 'HEIF image'),
    @('.heif', 'PreviewForWindows.Heif', 'HEIF image'),
    @('.gif', 'PreviewForWindows.Gif', 'GIF image'),
    @('.tif', 'PreviewForWindows.Tiff', 'TIFF image'),
    @('.tiff', 'PreviewForWindows.Tiff', 'TIFF image'),
    @('.bmp', 'PreviewForWindows.Bmp', 'BMP image')
)
# Verb key, menu text, command-line flag, and whether PDFs get it.
$verbs = @(
    @('PreviewForWindows.Convert', 'Convert', '--convert', $false),
    @('PreviewForWindows.Resize', 'Resize', '--resize', $false),
    @('PreviewForWindows.Combine', 'Combine into PDF', '--combine', $true)
)
$hive = [Microsoft.Win32.Registry]::CurrentUser

function Set-Value([string]$Key, [string]$Name, [string]$Value) {
    $handle = $hive.CreateSubKey($Key)
    try { $handle.SetValue($Name, $Value, [Microsoft.Win32.RegistryValueKind]::String) } finally { $handle.Close() }
}
function Remove-Value([string]$Key, [string]$Name) {
    $handle = $hive.OpenSubKey($Key, $true)
    if ($handle) { try { $handle.DeleteValue($Name, $false) } finally { $handle.Close() } }
}

if ($Unregister) {
    foreach ($type in $types) {
        $hive.DeleteSubKeyTree("$classes\$($type[1])", $false)
        Remove-Value "$classes\$($type[0])\OpenWithProgids" $type[1]
        foreach ($verb in $verbs) { $hive.DeleteSubKeyTree("$classes\SystemFileAssociations\$($type[0])\shell\$($verb[0])", $false) }
    }
    $hive.DeleteSubKeyTree($application, $false)
    $hive.DeleteSubKeyTree($capabilities, $false)
    Remove-Value "$Root\RegisteredApplications" $appName
} else {
    foreach ($type in $types) {
        $extension, $progId, $typeName = $type
        Set-Value "$classes\$progId" '' $typeName
        Set-Value "$classes\$progId\DefaultIcon" '' $icon
        Set-Value "$classes\$progId\shell\open" 'MultiSelectModel' 'Player'
        Set-Value "$classes\$progId\shell\open\command" '' $open
        Set-Value "$classes\$extension\OpenWithProgids" $progId ''
        Set-Value "$application\SupportedTypes" $extension ''
        Set-Value "$capabilities\FileAssociations" $extension $progId
        foreach ($verb in $verbs) {
            if ($extension -eq '.pdf' -and -not $verb[3]) { continue }
            $key = "$classes\SystemFileAssociations\$extension\shell\$($verb[0])"
            Set-Value $key 'MUIVerb' $verb[1]
            Set-Value $key 'MultiSelectModel' 'Player'
            Set-Value $key 'Icon' $icon
            Set-Value "$key\command" '' ('"' + $exe + '" ' + $verb[2] + ' "%1"')
        }
    }
    Set-Value $application 'FriendlyAppName' $appName
    Set-Value "$application\shell\open" 'MultiSelectModel' 'Player'
    Set-Value "$application\shell\open\command" '' $open
    Set-Value $capabilities 'ApplicationName' $appName
    Set-Value $capabilities 'ApplicationDescription' 'View and edit PDFs and images.'
    Set-Value "$Root\RegisteredApplications" $appName $capabilities
}

if ($Root -eq 'Software') {
    # SHCNE_ASSOCCHANGED with SHCNF_IDLIST tells Explorer to reload associations.
    Add-Type -Namespace PreviewForWindows -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int eventId, uint flags, System.IntPtr item1, System.IntPtr item2);'
    [PreviewForWindows.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
}
if ($Unregister) {
    'Preview is removed from Open with and Default apps for this Windows user.'
} else {
    'Preview is in Open with and Default apps for this Windows user. Your default apps did not change. To make Preview the default, open Settings > Apps > Default apps.'
}
