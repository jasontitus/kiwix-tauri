# Fill and accept a native Save dialog owned by kiwix-workbench using UI Automation.
# Usage: save-dialog.ps1 -Path E:\kiwix-win\exports\x.pdf [-TimeoutSec 20] [-Cancel]
param([string]$Path, [int]$TimeoutSec = 20, [switch]$Cancel)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$kiwix = @(Get-Process kiwix-workbench -ErrorAction SilentlyContinue).Id
if (-not $kiwix) { throw 'kiwix-workbench is not running' }
$deadline = (Get-Date).AddSeconds($TimeoutSec)
$dialog = $null
while (-not $dialog -and (Get-Date) -lt $deadline) {
    $wins = $AE::RootElement.FindAll($TS::Children, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($w in $wins) {
        if ($kiwix -contains $w.Current.ProcessId -and $w.Current.ClassName -eq '#32770') { $dialog = $w; break }
        # Common dialogs can also be children of the app window.
        if ($kiwix -contains $w.Current.ProcessId) {
            $inner = $w.FindFirst($TS::Children, (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, '#32770')))
            if ($inner) { $dialog = $inner; break }
        }
    }
    if (-not $dialog) { Start-Sleep -Milliseconds 300 }
}
if (-not $dialog) { "NO DIALOG within ${TimeoutSec}s"; exit 2 }
"dialog: '$($dialog.Current.Name)' class=$($dialog.Current.ClassName)"
function Find-ById($root, $id) {
    $root.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, $id)))
}
if ($Cancel) {
    $btn = Find-ById $dialog '2'
    if ($btn) { $btn.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
    else { $dialog.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close() }
    'cancelled'; exit 0
}
$edit = Find-ById $dialog '1001'
if (-not $edit) { 'NO FILENAME FIELD'; exit 3 }
"suggested name: $($edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value)"
$edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Path)
$save = Find-ById $dialog '1'
$save.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
"saved as $Path"
