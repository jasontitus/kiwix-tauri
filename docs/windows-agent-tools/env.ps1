# Dot-source: . E:\kiwix-win\env.ps1   — MSVC + Rust + Node + self-built SDK, all temp/caches on E:
$lines = cmd.exe /c '"E:\BuildTools2022\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set'
foreach ($l in $lines) { if ($l -match '^([^=]+)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2] } }
$env:RUSTUP_HOME = 'E:\dev\rustup'
$env:CARGO_HOME  = 'E:\dev\cargo'
# User-private ACL, like %LOCALAPPDATA%\Temp; profile-privacy tests reject broadly-shared folders.
$env:TEMP = 'E:\kiwix-win\tmp-private'; $env:TMP = 'E:\kiwix-win\tmp-private'
$env:npm_config_cache = 'E:\dev\npm-cache'
$sdk = if ($env:KIWIX_SDK_OVERRIDE) { $env:KIWIX_SDK_OVERRIDE } else { 'E:\kiwix-win\sdk\x86_64-pc-windows-msvc' }
$env:KIWIX_DESKTOP_SDK_ROOT = $sdk
$env:LIBZIM_INCLUDE_DIR = Join-Path $sdk 'include'
$env:LIBZIM_LIB_DIR = Join-Path $sdk 'lib'
Remove-Item Env:KIWIX_TYPECHECK_ONLY -ErrorAction SilentlyContinue
# Workaround (no source change): desktop_sdk.rs canonicalize() yields \\?\E:\... paths that cl.exe
# cannot use for -I; expose the same verified SDK dirs through MSVC's INCLUDE/LIB search paths.
$env:INCLUDE = "$sdk\include;$env:INCLUDE"
$env:LIB = "$sdk\lib;$env:LIB"
$node = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
if (-not $node) { $node = (Get-ChildItem E:\dev\node* -Filter node.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
if (-not $node) { $node = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Filter node.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
$env:Path = "$sdk\bin;E:\dev\cargo\bin;$(Split-Path $node);$env:Path"
