# Local unsigned NSIS build per docs/PACKAGING.md "For a local Windows build" (self-built SDK).
# Usage: build-nsis.ps1 [-Source <worktree>]  -> E:\kiwix-win\nsis-build.log(.exit)
param([string]$Source = 'E:\kiwix-win\lf-check')
. E:\kiwix-win\env.ps1
$env:CARGO_TARGET_DIR = 'E:\kiwix-win\target-release'
$target = 'x86_64-pc-windows-msvc'
$sdk = (Resolve-Path $env:KIWIX_DESKTOP_SDK_ROOT).Path
Set-Location $Source
Remove-Item E:\kiwix-win\nsis-build.log.exit -ErrorAction SilentlyContinue
$log = 'E:\kiwix-win\nsis-build.log'
# sherpa-onnx's shared Windows build copies its runtime DLLs into the release
# profile directory from its build script. Build it first so the DLLs exist
# before Tauri validates the bundle resources below.
& cargo build --release --locked --target $target --manifest-path src-tauri/Cargo.toml -p sherpa-onnx-sys 2>&1 |
    ForEach-Object { "$_" } | Out-File -Encoding utf8 $log
if ($LASTEXITCODE -ne 0) { Set-Content E:\kiwix-win\nsis-build.log.exit $LASTEXITCODE; exit }
$resources = @{}
Get-ChildItem (Join-Path $sdk 'bin') -Filter '*.dll' -File | ForEach-Object { $resources[$_.FullName] = $_.Name }
$profile = Join-Path $env:CARGO_TARGET_DIR "$target\release"
$sherpa = @{
    'onnxruntime.dll' = '7F66F939A881BAF4F46A2216496798EDF4A1429878B646D12674AA62F27D8A25'
    'onnxruntime_providers_shared.dll' = '551D0E1FE4C227D8542314BA718D52F4379E0C7BFE729A37C59833A884E27B4D'
    'sherpa-onnx-c-api.dll' = '0965A6E4C0254DA2BEFF489CF7A8AAFFAF7DC94F90B43B3E66448B8CAB4177EA'
    'sherpa-onnx-cxx-api.dll' = 'ADA772889F966426AD9CABA5B5125BFFB836163573791FDDA99541280D45DAFA'
}
foreach ($entry in $sherpa.GetEnumerator()) {
    $dll = Join-Path $profile $entry.Key
    if (-not (Test-Path $dll) -or (Get-FileHash -Algorithm SHA256 $dll).Hash -ne $entry.Value) {
        "missing or unverified sherpa-onnx runtime DLL: $dll" | Out-File -Append -Encoding utf8 $log
        Set-Content E:\kiwix-win\nsis-build.log.exit 3; exit
    }
    $resources[$dll] = $entry.Key
}
# The agent connector relay (kiwix-mcp) is a separate crate; ship it beside the app.
if (Test-Path (Join-Path $Source 'kiwix-mcp\Cargo.toml')) {
    & cargo build --release --locked --target $target --manifest-path kiwix-mcp/Cargo.toml 2>&1 |
        ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
    if ($LASTEXITCODE -ne 0) { Set-Content E:\kiwix-win\nsis-build.log.exit $LASTEXITCODE; exit }
    $resources[(Join-Path $profile 'kiwix-mcp.exe')] = 'kiwix-mcp.exe'
}
$config = 'E:\kiwix-win\tmp-private\kiwix-bundle.json'
@{ bundle = @{ resources = $resources } } | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 $config
# Native tools write progress to stderr; never treat that as failure. Use the exit code.
& npm.cmd run tauri -- build --target $target --bundles nsis --config $config 2>&1 |
    ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
Set-Content E:\kiwix-win\nsis-build.log.exit $LASTEXITCODE
