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
$preserved = "$Root\PreviewForWindows\AssociationPreservation"
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
$classesRoot = [Microsoft.Win32.Registry]::ClassesRoot
$thumbnailHandler = '{e357fccd-a995-4576-b01f-234630154e96}'
$previewHandler = '{8895b1c6-b41f-4c1c-a562-0d564250836f}'

function Set-Value([string]$Key, [string]$Name, [string]$Value) {
    $handle = $hive.CreateSubKey($Key)
    try { $handle.SetValue($Name, $Value, [Microsoft.Win32.RegistryValueKind]::String) } finally { $handle.Close() }
}
function Remove-Value([string]$Key, [string]$Name) {
    $handle = $hive.OpenSubKey($Key, $true)
    if ($handle) { try { $handle.DeleteValue($Name, $false) } finally { $handle.Close() } }
}
function Get-Value([Microsoft.Win32.RegistryKey]$Hive, [string]$Key, [string]$Name) {
    $handle = $Hive.OpenSubKey($Key)
    if (-not $handle) { return $null }
    try { return $handle.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } finally { $handle.Close() }
}
function Get-SourceValue([string]$RelativeKey, [string]$Name) {
    if ($Root -ieq 'Software') { return Get-Value $classesRoot $RelativeKey $Name }
    return Get-Value $hive "$Root\MachineClasses\$RelativeKey" $Name
}
function Get-PreviousProgId([string]$Extension) {
    if ($Root -ieq 'Software') {
        $value = Get-Value $hive "Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$Extension\UserChoice" 'ProgId'
    } else {
        $value = Get-Value $hive "$Root\UserChoice\$Extension" 'ProgId'
    }
    if ($value) { return $value }
    return Get-SourceValue $Extension ''
}
function Test-HandlerClsid([string]$Value) {
    return $Value -match '^\{[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\}$'
}
function Test-PerceivedType([string]$Value) {
    return $Value -match '^[0-9a-zA-Z_-]{1,64}$'
}
function Preserve-ShellMetadata([string]$Extension) {
    $extensionKey = "$classes\$Extension"
    $markerKey = "$preserved\$($Extension.TrimStart('.'))"
    # Read the merged association before creating the per-user extension key.
    $perceivedType = Get-SourceValue $Extension 'PerceivedType'
    $previousProgId = Get-PreviousProgId $Extension
    $handlers = foreach ($handler in @(
        @($thumbnailHandler, 'ThumbnailHandler'),
        @($previewHandler, 'PreviewHandler')
    )) {
        $value = Get-SourceValue "$Extension\ShellEx\$($handler[0])" ''
        if ($null -eq $value -and $previousProgId) {
            $value = Get-SourceValue "$previousProgId\ShellEx\$($handler[0])" ''
        }
        ,@($handler[0], $handler[1], $value)
    }
    if ($null -eq (Get-Value $hive $extensionKey 'PerceivedType')) {
        if ($null -ne $perceivedType -and (Test-PerceivedType $perceivedType)) {
            Set-Value $extensionKey 'PerceivedType' $perceivedType
            Set-Value $markerKey 'PerceivedType' $perceivedType
        }
    }
    foreach ($handler in $handlers) {
        $target = "$extensionKey\ShellEx\$($handler[0])"
        if ($null -ne (Get-Value $hive $target '')) { continue }
        $value = $handler[2]
        if ($null -ne $value -and (Test-HandlerClsid $value)) {
            Set-Value $target '' $value
            Set-Value $markerKey $handler[1] $value
        }
    }
}
if ($Unregister) {
    # Keep copied Explorer metadata. Removing a shared extension key can race
    # another installer, while leaving these original handler values prevents
    # an HKCU key from masking the machine-wide thumbnail and preview handlers.
    foreach ($type in $types) {
        $hive.DeleteSubKeyTree("$classes\$($type[1])", $false)
        Remove-Value "$classes\$($type[0])\OpenWithProgids" $type[1]
        foreach ($verb in $verbs) { $hive.DeleteSubKeyTree("$classes\SystemFileAssociations\$($type[0])\shell\$($verb[0])", $false) }
    }
    $hive.DeleteSubKeyTree($application, $false)
    $hive.DeleteSubKeyTree($capabilities, $false)
    $hive.DeleteSubKeyTree($preserved, $false)
    Remove-Value "$Root\RegisteredApplications" $appName
} else {
    foreach ($type in $types) {
        $extension, $progId, $typeName = $type
        Preserve-ShellMetadata $extension
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
