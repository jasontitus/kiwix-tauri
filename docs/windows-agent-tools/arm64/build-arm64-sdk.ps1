#Requires -Version 5.1
# Cross-build the libzim 9.8.2 SDK for aarch64-pc-windows-msvc on an x64 host (MSVC x64->ARM64).
# Same recipe as E:\kiwix-win\build-sdk.ps1 (x64): zlib, zstd, xapian (static) with meson,
# official ICU 74.2 WinARM64 binaries, and the patched libzim source in E:\kiwix-win\src\libzim.
# NOT release provenance. Stages: deps | zim | sdk | all. Log: E:\kiwix-win\arm64\build.log
param([ValidateSet('deps','zim','sdk','all')][string]$Stage = 'all')
$ErrorActionPreference = 'Stop'
$Root = 'E:\kiwix-win'; $A = "$Root\arm64"; $Venv = "$Root\venv\Scripts"
$Prefix = "$A\INSTALL"; $Build = "$A\build"; $Cross = "$A\arm64-cross.ini"
$ZimPrefix = "$A\libzim-install"; $ZimBuild = "$Build\libzim"
$Out = "$Root\sdk-next\aarch64-pc-windows-msvc"

$lines = cmd.exe /c '"E:\BuildTools2022\VC\Auxiliary\Build\vcvarsamd64_arm64.bat" >nul 2>&1 && set'
foreach ($line in $lines) { if ($line -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] } }
$env:Path = "$Venv;$env:Path;C:\Program Files\Git\cmd;C:\Program Files\Git\usr\bin"
# Some targets (xapian's getopt) include config.h, which includes zlib.h, without
# the dependency's include path; kiwix-build exposes its prefix the same way.
$env:INCLUDE = "$Prefix\include;$env:INCLUDE"
$env:LIB = "$Prefix\lib;$env:LIB"
$env:TEMP = "$Root\tmp-private"; $env:TMP = "$Root\tmp-private"
$cl = (Get-Command cl.exe).Source
if ($cl -notlike '*Hostx64\arm64\cl.exe') { throw "cl.exe is $cl, not the x64->ARM64 cross compiler" }
if ((Get-Command link.exe).Source -notlike 'E:\BuildTools2022\*') { throw 'link.exe is not MSVC' }
# Only ARM64 packages: never let pkg-config find the x64 dependency prefix.
$env:PKG_CONFIG = "$Venv\pkg-config.exe"
$env:PKG_CONFIG_LIBDIR = "$Prefix\lib\pkgconfig"
$env:PKG_CONFIG_PATH = "$Prefix\lib\pkgconfig"
Write-Host "cross compiler: $cl"

@"
[binaries]
c = 'cl'
cpp = 'cl'
ar = 'lib'
windres = 'rc'
pkg-config = '$($Venv -replace '\\','/')/pkg-config.exe'

[host_machine]
system = 'windows'
cpu_family = 'aarch64'
cpu = 'armv8'
endian = 'little'

[properties]
needs_exe_wrapper = true
"@ | Set-Content -Encoding ascii $Cross

# Xapian compiles and runs one configure probe with the build machine's compiler.
# Give meson the x64 compiler with x64 libraries for that probe only; every
# shipped object is still built by the ARM64 cross compiler.
$Native = "$A\x64-native.ini"
$msvc = (Split-Path (Split-Path (Split-Path (Split-Path $cl)))) -replace '\\','/'
$kits = 'C:/Program Files (x86)/Windows Kits/10/Lib/' + $env:WindowsSDKLibVersion.TrimEnd('\')
$x64libs = "['/LIBPATH:$msvc/lib/x64', '/LIBPATH:$kits/ucrt/x64', '/LIBPATH:$kits/um/x64']"
@"
[binaries]
c = '$msvc/bin/Hostx64/x64/cl.exe'
cpp = '$msvc/bin/Hostx64/x64/cl.exe'

[built-in options]
c_link_args = $x64libs
cpp_link_args = $x64libs
"@ | Set-Content -Encoding ascii $Native

function Invoke-Checked([string]$Exe, [string[]]$ArgList) {
    & $Exe @ArgList
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($ArgList[0]) failed ($LASTEXITCODE)" }
}
function Build-Meson([string]$Name, [string]$Source, [string[]]$Options) {
    $dir = "$Build\$Name"
    $setup = @('setup', $dir, $Source, "--cross-file=$Cross", "--native-file=$Native", '--buildtype=release', "--prefix=$Prefix", '--libdir=lib') + $Options
    if (Test-Path "$dir\meson-private\coredata.dat") { $setup += '--reconfigure' }
    Invoke-Checked "$Venv\meson.exe" $setup
    Invoke-Checked "$Venv\meson.exe" @('compile', '-C', $dir)
    Invoke-Checked "$Venv\meson.exe" @('install', '-C', $dir)
}

if ($Stage -in 'deps','all') {
    New-Item -ItemType Directory -Force "$Prefix\bin", "$Prefix\lib\pkgconfig", "$Prefix\include" | Out-Null
    # ICU 74.2 official WinARM64 binaries, staged like kiwix-build stages the x64 zip.
    $icu = "$A\icu"
    Copy-Item "$icu\include\*" "$Prefix\include" -Recurse -Force
    Copy-Item "$icu\libARM64\*.lib" "$Prefix\lib" -Force
    Copy-Item "$icu\binARM64\icu*74.dll" "$Prefix\bin" -Force
    $pfx = $Prefix -replace '\\','/'
    @"
prefix=$pfx
libdir=`${prefix}/lib
includedir=`${prefix}/include
Name: icu-uc
Description: International Components for Unicode: Common and Data libraries
Version: 74.2
Cflags: -I`${includedir}
Libs: -L`${libdir} -licuuc -licudt
"@ | Set-Content -Encoding ascii "$Prefix\lib\pkgconfig\icu-uc.pc"
    @"
prefix=$pfx
libdir=`${prefix}/lib
includedir=`${prefix}/include
Name: icu-i18n
Description: International Components for Unicode: Stream and I/O Library
Version: 74.2
Requires: icu-uc
Cflags: -I`${includedir}
Libs: -L`${libdir} -licuin
"@ | Set-Content -Encoding ascii "$Prefix\lib\pkgconfig\icu-i18n.pc"
    Build-Meson 'zlib' "$Root\kb\SOURCE\zlib-1.3.1" @('--default-library=shared')
    Build-Meson 'zstd' "$Root\kb\SOURCE\zstd-1.5.7\build\meson" @('--default-library=shared', '-Dbin_programs=false', '-Dbin_contrib=false')
    Build-Meson 'xapian' "$Root\kb\SOURCE\xapian-core\xapian-core" @('--default-library=static', '-Denable-sse=false', '-Denable-backend-chert=false', '-Denable-backend-remote=false')
}

if ($Stage -in 'zim','all') {
    $env:Path = "$Prefix\bin;$env:Path"
    $opts = @('setup', $ZimBuild, "$Root\src\libzim", "--cross-file=$Cross", "--prefix=$ZimPrefix", '--libdir=lib',
        '--buildtype=release', '--default-library=shared', '-Dwith_xapian=true', '-Dwerror=false',
        '-Dexamples=false', '-Ddoc=false', '-Dtests=false')
    if (Test-Path "$ZimBuild\meson-private\coredata.dat") { $opts += '--reconfigure' }
    Invoke-Checked "$Venv\meson.exe" $opts
    Invoke-Checked "$Venv\meson.exe" @('compile', '-C', $ZimBuild)
    Invoke-Checked "$Venv\meson.exe" @('install', '-C', $ZimBuild)
}

if ($Stage -in 'sdk','all') {
    $script = "$A\assemble-sdk-arm64.py"
    (Get-Content "$Root\sdk-next\x86_64-pc-windows-msvc\share\provenance\assemble-sdk.py" -Raw) -replace
        '"target": "x86_64-pc-windows-msvc"', '"target": "aarch64-pc-windows-msvc"' | Set-Content -Encoding utf8 $script
    # The assembler records the build script beside it as provenance.
    Copy-Item $PSCommandPath "$A\build-sdk.ps1" -Force
    Invoke-Checked 'python' @('-I', $script, "$Root\lf-check", $ZimPrefix, $Prefix, "$Root\src\libzim", $Out, $ZimBuild)
    # Every shipped binary must be ARM64 (IMAGE_FILE_MACHINE_ARM64 = 0xAA64).
    foreach ($dll in Get-ChildItem "$Out\bin" -Filter *.dll) {
        $bytes = [IO.File]::ReadAllBytes($dll.FullName); $pe = [BitConverter]::ToInt32($bytes, 0x3C)
        $machine = [BitConverter]::ToUInt16($bytes, $pe + 4)
        if ($machine -ne 0xAA64) { throw ("{0} is machine 0x{1:X4}, not ARM64" -f $dll.Name, $machine) }
        "{0} ARM64 ok" -f $dll.Name
    }
}
