# Integration gate on lf-check with the sdk-next SDK: clippy (all targets, -D warnings), then the full lib test suite.
$env:KIWIX_SDK_OVERRIDE = 'E:\kiwix-win\sdk-next\x86_64-pc-windows-msvc'
$ps = 'E:\kiwix-win\run-cargo.ps1'
& powershell -NoProfile -ExecutionPolicy Bypass -File $ps -Name gate-clippy -CargoArgs 'clippy|--locked|--manifest-path|src-tauri/Cargo.toml|--all-targets|--features|custom-protocol|--target|x86_64-pc-windows-msvc|--|-D|warnings'
& powershell -NoProfile -ExecutionPolicy Bypass -File $ps -Name gate-test -CargoArgs 'test|--locked|--manifest-path|src-tauri/Cargo.toml|--lib|--features|custom-protocol|--target|x86_64-pc-windows-msvc'
Set-Content E:\kiwix-win\gate.done "clippy=$(Get-Content E:\kiwix-win\gate-clippy.log.exit) test=$(Get-Content E:\kiwix-win\gate-test.log.exit)"
