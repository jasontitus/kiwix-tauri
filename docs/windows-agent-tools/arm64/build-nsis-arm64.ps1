# Local unsigned NSIS build for Windows ARM64 (aarch64-pc-windows-msvc), cross-compiled on x64.
# Mirrors E:\kiwix-win\build-nsis.ps1. Log: E:\kiwix-win\arm64\nsis-arm64.log(.exit)
param([string]$Source = 'E:\kiwix-win\lf-check')
$ErrorActionPreference = 'Continue'
$target = 'aarch64-pc-windows-msvc'
$log = 'E:\kiwix-win\arm64\nsis-arm64.log'
Remove-Item "$log.exit" -ErrorAction SilentlyContinue
"starting $(Get-Date -Format o)" | Out-File -Encoding utf8 $log

# x64-hosted MSVC targeting ARM64; Rust, Node and caches on E:.
$lines = cmd.exe /c '"E:\BuildTools2022\VC\Auxiliary\Build\vcvarsamd64_arm64.bat" >nul 2>&1 && set'
foreach ($l in $lines) { if ($l -match '^([^=]+)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2] } }
$env:RUSTUP_HOME = 'E:\dev\rustup'; $env:CARGO_HOME = 'E:\dev\cargo'
$env:TEMP = 'E:\kiwix-win\tmp-private'; $env:TMP = 'E:\kiwix-win\tmp-private'
$env:npm_config_cache = 'E:\dev\npm-cache'
$node = (Get-ChildItem E:\dev\node* -Filter node.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1).FullName
if (-not $node) { $node = (Get-Command node.exe -ErrorAction SilentlyContinue).Source }
# ring and aws-lc-sys compile their ARM64 Windows sources with clang (VS "C++ Clang tools").
$llvm = 'E:\BuildTools2022\VC\Tools\Llvm\x64\bin'
if (-not (Test-Path "$llvm\clang.exe")) {
    "clang.exe not found in $llvm; add the VS component Microsoft.VisualStudio.Component.VC.Llvm.Clang" | Out-File -Append -Encoding utf8 $log
    Set-Content "$log.exit" 2; exit 2
}
$env:Path = "E:\dev\cargo\bin;$(Split-Path $node);$llvm;$env:Path"
$env:CARGO_TARGET_DIR = 'E:\kiwix-win\target-release'
Set-Location $Source

function Machine([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path); $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    [BitConverter]::ToUInt16($bytes, $pe + 4)
}
function Fail([int]$Code, [string]$Message) {
    $Message | Out-File -Append -Encoding utf8 $log; Set-Content "$log.exit" $Code; exit $Code
}

"rustc target $target" | Out-File -Append -Encoding utf8 $log
& cargo build --release --locked --target $target --manifest-path src-tauri/Cargo.toml -p sherpa-onnx-sys 2>&1 |
    ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
if ($LASTEXITCODE -ne 0) { Fail $LASTEXITCODE 'sherpa-onnx-sys build failed' }
$profile = Join-Path $env:CARGO_TARGET_DIR "$target\release"
$resources = @{}
# sherpa-onnx 2.0.1 win-arm64-shared-MT-Release, pinned like the x64 build: the
# crate downloads its prebuilt archive without a checksum.
$sherpa = @{
    'onnxruntime.dll' = '968D081836CAD9AF537105F59E34D4997BCA2CE6D78FF39D3E47660620E8A642'
    'onnxruntime_providers_shared.dll' = '05FB0CC694B0671DB544E77C55C3736D13BE850B8B4045145E982E072E6F938F'
    'sherpa-onnx-c-api.dll' = '041BC4D2E1DEA05A75656B126CA58BD603594E1BE14F827293486F64567BB5EB'
    'sherpa-onnx-cxx-api.dll' = '88E92FFB92827A198208C8DFF21C9D76F3E51803C28D5D496176A7A1F14036C5'
}
foreach ($entry in $sherpa.GetEnumerator()) {
    $dll = Join-Path $profile $entry.Key
    if (-not (Test-Path $dll) -or (Get-FileHash -Algorithm SHA256 $dll).Hash -ne $entry.Value) { Fail 3 "missing or unverified sherpa-onnx runtime DLL: $dll" }
    if ((Machine $dll) -ne 0xAA64) { Fail 3 "$($entry.Key) is not ARM64" }
    $resources[$dll] = $entry.Key
}
# The Visual C++ runtime, app-local: clean Windows ARM64 PCs often lack the
# ARM64 VC++ redistributable that x64 PCs almost always have.
$crt = Get-ChildItem 'E:\BuildTools2022\VC\Redist\MSVC' -Directory | Where-Object Name -match '^\d' |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
foreach ($dll in Get-ChildItem (Join-Path $crt.FullName 'arm64\Microsoft.VC143.CRT') -Filter '*.dll') {
    # The ARM64 redist also carries vcruntime140_1.dll for emulated x64 code;
    # native ARM64 code does not use it, so ship ARM64 binaries only.
    if ((Machine $dll.FullName) -ne 0xAA64) { "skipping non-ARM64 runtime $($dll.Name)" | Out-File -Append -Encoding utf8 $log; continue }
    $resources[$dll.FullName] = $dll.Name
}
& cargo build --release --locked --target $target --manifest-path kiwix-mcp/Cargo.toml 2>&1 |
    ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
if ($LASTEXITCODE -ne 0) { Fail $LASTEXITCODE 'kiwix-mcp build failed' }
$resources[(Join-Path $profile 'kiwix-mcp.exe')] = 'kiwix-mcp.exe'
$config = 'E:\kiwix-win\tmp-private\kiwix-bundle-arm64.json'
@{ bundle = @{ resources = $resources } } | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $config
& npm.cmd run tauri -- build --target $target --bundles nsis --config $config 2>&1 |
    ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
$code = $LASTEXITCODE
if ($code -eq 0) {
    # Every shipped executable and DLL must be ARM64 (0xAA64). The NSIS installer
    # and uninstaller stubs are x86 by design and are not checked here.
    foreach ($file in @((Join-Path $profile 'kiwix-workbench.exe')) + @($resources.Keys)) {
        $machine = Machine $file
        "{0} machine 0x{1:X4}" -f (Split-Path $file -Leaf), $machine | Out-File -Append -Encoding utf8 $log
        if ($machine -ne 0xAA64) { Fail 4 "$file is not ARM64" }
    }
}
Set-Content "$log.exit" $code
