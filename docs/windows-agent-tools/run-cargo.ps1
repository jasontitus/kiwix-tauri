# Run cargo in the LF worktree with the MSVC + SDK environment; logs to E:\kiwix-win\<Name>.log(.exit)
# -CargoArgs is a single '|'-separated string (powershell -File passes strings, not arrays).
param([string]$Name, [string]$CargoArgs)
. E:\kiwix-win\env.ps1
Set-Location E:\kiwix-win\lf-check
# sherpa-onnx's shared Windows build copies its runtime DLLs into the profile
# directory; test executables run from deps\ and find them through PATH.
$env:PATH = "E:\kiwix-win\lf-check\src-tauri\target\x86_64-pc-windows-msvc\debug;$env:PATH"
$log = "E:\kiwix-win\$Name.log"
Remove-Item "$log.exit" -ErrorAction SilentlyContinue
$list = $CargoArgs -split '\|'
& cargo @list 2>&1 | ForEach-Object { "$_" } | Out-File -Encoding utf8 $log
Set-Content "$log.exit" $LASTEXITCODE
