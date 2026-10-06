#requires -Version 5.1
[CmdletBinding()]
param([string]$Executable = (Join-Path $PSScriptRoot 'preview-for-windows.exe'))
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
if ([IO.Path]::GetExtension($exe) -ine '.exe') { throw 'Select the Preview executable.' }
$application = 'HKCU:\Software\Classes\Applications\preview-for-windows.exe'
$command = '"' + $exe + '" "%1"'
New-Item -Path "$application\shell\open\command" -Force | Out-Null
Set-Item -LiteralPath "$application\shell\open\command" -Value $command
New-ItemProperty -LiteralPath $application -Name FriendlyAppName -Value 'Preview for Windows' -PropertyType String -Force | Out-Null
New-Item -Path "$application\SupportedTypes" -Force | Out-Null
foreach ($extension in @('.pdf','.jpg','.jpeg','.png','.gif','.tif','.tiff','.bmp','.webp')) {
    New-ItemProperty -LiteralPath "$application\SupportedTypes" -Name $extension -Value '' -PropertyType String -Force | Out-Null
}
'Preview is registered in Open with for this Windows user. Your default apps were not changed.'
