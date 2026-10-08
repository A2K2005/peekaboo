# Dumps the app's UI Automation tree and fails if any interactive element
# has no name. Windows PowerShell 5.1. Opens a window: run only in a GUI
# session that may be disturbed.
#
#   powershell -NoProfile -File tests/ui/a11y-check.ps1 -Executable target/release/peekaboo.exe -InputFile fixtures/20-pages.pdf
#
# Scenarios: the document window, the More menu, the Find sheet, and the
# empty window. Trees go to artifacts/a11y/<scenario>.txt.
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$InputFile = '',
    [string]$OutDir = (Join-Path $PSScriptRoot '../../artifacts/a11y'),
    [int]$TimeoutSeconds = 20
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
New-Item -ItemType Directory -Force $OutDir | Out-Null
$exe = (Resolve-Path -LiteralPath $Executable).Path
$interactive = @('Button', 'TabItem', 'MenuItem', 'Edit', 'CheckBox', 'ComboBox', 'Hyperlink', 'ListItem', 'RadioButton', 'SplitButton', 'Slider', 'Document')
$script:missing = @()

function Start-App([string]$file) {
    $marker = Join-Path $OutDir ('marker-' + [Guid]::NewGuid().ToString('N') + '.json')
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    if ($file) { $psi.Arguments = '"' + (Resolve-Path -LiteralPath $file).Path + '"' }
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables['PFW_BENCH_OUT'] = $marker
    $p = [Diagnostics.Process]::Start($psi)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $p.Refresh()
        $ready = $p.MainWindowHandle -ne [IntPtr]::Zero -and ((-not $file) -or (Test-Path -LiteralPath $marker))
        if ($ready) { break }
        Start-Sleep -Milliseconds 100
    }
    # Accessibility starts after first content; give it a moment.
    Start-Sleep -Milliseconds 800
    Remove-Item -LiteralPath $marker -ErrorAction SilentlyContinue
    return $p
}

function Write-Tree($element, [int]$depth, [System.Text.StringBuilder]$out, [string]$scenario) {
    $c = $element.Current
    $type = $c.ControlType.ProgrammaticName -replace '^ControlType\.', ''
    $r = $c.BoundingRectangle
    $line = ('  ' * $depth) + "$type ""$($c.Name)"" id=$($c.AutomationId) focusable=$($c.IsKeyboardFocusable) enabled=$($c.IsEnabled) rect=$([int]$r.X),$([int]$r.Y),$([int]$r.Width),$([int]$r.Height)"
    if ($c.AcceleratorKey) { $line += " shortcut=$($c.AcceleratorKey)" }
    if ($c.AccessKey) { $line += " accesskey=$($c.AccessKey)" }
    [void]$out.AppendLine($line)
    if ($interactive -contains $type -and [string]::IsNullOrWhiteSpace($c.Name)) {
        $script:missing += "${scenario}: $type at depth $depth has no name"
    }
    $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
    $child = $walker.GetFirstChild($element)
    while ($child -ne $null) {
        Write-Tree $child ($depth + 1) $out $scenario
        $child = $walker.GetNextSibling($child)
    }
}

function Save-Tree($element, [string]$scenario) {
    $sb = New-Object System.Text.StringBuilder
    Write-Tree $element 0 $sb $scenario
    $path = Join-Path $OutDir "$scenario.txt"
    [IO.File]::WriteAllText($path, $sb.ToString())
    Write-Output "Wrote $path"
}

function Find-ByName($root, [string]$name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    return $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}

function Invoke-Element($element) {
    $pattern = $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
    Start-Sleep -Milliseconds 600
}

function Stop-App($p) {
    if (-not $p.HasExited) { [void]$p.CloseMainWindow(); [void]$p.WaitForExit(3000) }
    if (-not $p.HasExited) { $p.Kill() }
}

$p = Start-App $InputFile
try {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
    Save-Tree $root 'document'
    $more = Find-ByName $root 'More'
    if ($more) {
        Invoke-Element $more
        # The menu is a separate popup window owned by the app process.
        $menus = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
            [System.Windows.Automation.TreeScope]::Children,
            (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p.Id)))
        $i = 0
        foreach ($m in $menus) { if ($m.Current.NativeWindowHandle -ne [int]$p.MainWindowHandle) { Save-Tree $m "menu-$i"; $i++ } }
    } else { $script:missing += 'document: no element named More' }
} finally { Stop-App $p }

if ($InputFile) {
    $p = Start-App $InputFile
    try {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
        $search = Find-ByName $root 'Search'
        if ($search -and $search.Current.IsEnabled) {
            Invoke-Element $search
            Save-Tree $root 'find-sheet'
        }
    } finally { Stop-App $p }
}

$p = Start-App ''
try { Save-Tree ([System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)) 'empty' }
finally { Stop-App $p }

if ($script:missing.Count -gt 0) {
    $script:missing | ForEach-Object { Write-Output "MISSING NAME: $_" }
    exit 1
}
Write-Output 'Every interactive element has a name.'
