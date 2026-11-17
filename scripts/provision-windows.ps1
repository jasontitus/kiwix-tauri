#Requires -Version 5.1
# Provisions the Windows build tools that can be installed non-interactively.
# The verified libzim 9.8.2 MSVC SDK remains an external prerequisite.
param([switch]$Install)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This script must run on Windows.'
}

function Refresh-Path {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
        [Environment]::GetEnvironmentVariable('Path', 'User')
}

if ($Install) {
    $admin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) { throw 'Run -Install from an elevated PowerShell.' }
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw 'winget is unavailable; install App Installer from Microsoft Store first.'
    }
    winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --accept-package-agreements --accept-source-agreements --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    winget install --id Rustlang.Rustup --exact --accept-package-agreements --accept-source-agreements
    winget install --id OpenJS.NodeJS.LTS --exact --accept-package-agreements --accept-source-agreements
    winget install --id Git.Git --exact --accept-package-agreements --accept-source-agreements
    Refresh-Path
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        rustup default stable
        rustup target add x86_64-pc-windows-msvc
    } else {
        Write-Host 'Rustup was installed but is not on PATH yet. Open a new shell and rerun without -Install.' -ForegroundColor Yellow
    }
}

& (Join-Path $PSScriptRoot 'check-windows-build.ps1')
