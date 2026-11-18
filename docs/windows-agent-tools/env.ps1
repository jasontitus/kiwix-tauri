# Dot-source: . E:\kiwix-win\env.ps1   — MSVC + Rust + Node, all temp/caches on E:
$lines = cmd.exe /c '"E:\BuildTools2022\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set'
foreach ($l in $lines) { if ($l -match '^([^=]+)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2] } }
$env:RUSTUP_HOME = 'E:\dev\rustup'
$env:CARGO_HOME  = 'E:\dev\cargo'
# User-private ACL, like %LOCALAPPDATA%\Temp; profile-privacy tests reject broadly-shared folders.
$env:TEMP = 'E:\kiwix-win\tmp-private'; $env:TMP = 'E:\kiwix-win\tmp-private'
$env:npm_config_cache = 'E:\dev\npm-cache'
Remove-Item Env:KIWIX_TYPECHECK_ONLY -ErrorAction SilentlyContinue
$node = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
if (-not $node) { $node = (Get-ChildItem E:\dev\node* -Filter node.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
if (-not $node) { $node = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Filter node.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
$env:Path = "E:\dev\cargo\bin;$(Split-Path $node);$env:Path"
