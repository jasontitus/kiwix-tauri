# Launch the installed Kiwix with the E: test profile and a localhost-only CDP port; wait until CDP answers.
param([string[]]$Extra = @())
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
$p = Start-Process E:\Apps\Kiwix\kiwix-workbench.exe -ArgumentList (@('--data-dir', 'E:\kiwix-win\appdata') + $Extra) -PassThru
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline) {
    try { Invoke-WebRequest -UseBasicParsing http://127.0.0.1:9222/json/version -TimeoutSec 2 | Out-Null; break }
    catch { Wait-Event -Timeout 1 | Out-Null }
}
"app pid $($p.Id); running=$(-not $p.HasExited)"
