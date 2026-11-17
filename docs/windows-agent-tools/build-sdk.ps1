#Requires -Version 5.1
# Local, self-built Windows MSVC libzim 9.8.2 SDK for kiwix-tauri testing.
# NOT release provenance. Stages: env | deps | zim | test
param([ValidateSet('env','deps','zim','test','all')][string]$Stage = 'all')
$ErrorActionPreference = 'Stop'
$Root   = 'E:\kiwix-win'
$Venv   = "$Root\venv\Scripts"
$KbWork = "$Root\kb"
$DepsPrefix = "$KbWork\BUILD_native_dyn\INSTALL"
$ZimSrc = "$Root\src\libzim"
$ZimBuild = "$Root\build\libzim"
$ZimPrefix = "$Root\libzim-install"

function Import-VsEnv {
    $vcvars = 'E:\BuildTools2022\VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $vcvars)) { throw "Missing $vcvars" }
    $lines = cmd.exe /c "`"$vcvars`" >nul 2>&1 && set"
    foreach ($line in $lines) {
        if ($line -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
    }
    # venv tools first; Git's usr\bin (patch.exe) LAST so its link.exe never shadows MSVC link.exe.
    $env:Path = "$Venv;$env:Path;C:\Program Files\Git\cmd;C:\Program Files\Git\usr\bin"
    $env:PKG_CONFIG = "$Venv\pkg-config.exe"
    $link = (Get-Command link.exe).Source
    if ($link -notlike 'E:\BuildTools2022\*') { throw "link.exe resolves to $link, not MSVC" }
    Write-Host "MSVC env loaded; cl=$((Get-Command cl.exe).Source)"
}

function Invoke-Checked([string]$Exe, [string[]]$ArgList) {
    & $Exe @ArgList
    if ($LASTEXITCODE -ne 0) { throw "$Exe failed ($LASTEXITCODE)" }
}

Import-VsEnv
if ($Stage -eq 'env') { return }

if ($Stage -in 'deps','all') {
    New-Item -ItemType Directory -Force $KbWork | Out-Null
    Push-Location $KbWork
    try {
        Invoke-Checked "$Venv\kiwix-build.exe" @('--working-dir', $KbWork, '--config', 'native_dyn',
            '--build-deps-only', '--hide-progress', '--assume-packages-installed', '--fast-clone', 'libzim')
    } finally { Pop-Location }
}

if ($Stage -in 'zim','all') {
    $zts = Get-ChildItem "$KbWork\SOURCE" -Directory -Filter 'zim-testing-suite*' | Select-Object -First 1
    $env:PKG_CONFIG_PATH = "$DepsPrefix\lib\pkgconfig"
    $env:Path = "$DepsPrefix\bin;$env:Path"
    $opts = @('setup', $ZimBuild, $ZimSrc, "--prefix=$ZimPrefix", '--libdir=lib', '--buildtype=release',
        '--default-library=shared', '-Dwith_xapian=true', '-Dwerror=false', '-Dexamples=false', '-Ddoc=false',
        "-Dtest_data_dir=$($zts.FullName)")
    if (Test-Path "$ZimBuild\meson-private\coredata.dat") { $opts += '--reconfigure' }
    Invoke-Checked "$Venv\meson.exe" $opts
    Invoke-Checked "$Venv\meson.exe" @('compile', '-C', $ZimBuild)
    Invoke-Checked "$Venv\meson.exe" @('install', '-C', $ZimBuild)
}

if ($Stage -in 'test','all') {
    $env:Path = "$DepsPrefix\bin;$ZimBuild\src;$env:Path"
    $env:SKIP_BIG_MEMORY_TEST = '1'
    & "$Venv\meson.exe" test -C $ZimBuild --print-errorlogs -t 8
    Write-Host "libzim meson test exit=$LASTEXITCODE"
}
