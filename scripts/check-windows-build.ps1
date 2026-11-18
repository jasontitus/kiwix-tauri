#Requires -Version 5.1
# Read-only Windows build prerequisite checker. Run from the repository root.
$ErrorActionPreference = 'Stop'
$missing = New-Object System.Collections.Generic.List[string]

function Require-Command([string]$Name, [string]$Hint) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        $missing.Add("$Name ($Hint)")
    }
}
Require-Command 'git' 'Git for Windows'
Require-Command 'node' 'Node 22 LTS'
Require-Command 'npm' 'Node 22 LTS'
Require-Command 'rustc' 'Rust stable via rustup'
Require-Command 'cargo' 'Rust stable via rustup'
Require-Command 'cl' 'Visual Studio 2022 C++ build tools (Developer PowerShell)'

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { $missing.Add('Visual Studio C++ workload (vswhere found no VC tools)') }
} else {
    $missing.Add('Visual Studio Installer/vswhere (install Build Tools 2022)')
}

$webview = @(
    "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application",
    "${env:ProgramFiles}\Microsoft\EdgeWebView\Application"
) | Where-Object { Test-Path $_ }
if (-not $webview) { $missing.Add('Microsoft Edge WebView2 runtime') }

if ($missing.Count -gt 0) {
    Write-Host 'Windows build is NOT ready. Missing:' -ForegroundColor Yellow
    $missing | ForEach-Object { Write-Host "  - $_" }
    exit 1
}
Write-Host 'Windows build prerequisites are present. Run the Phase 1 commands in docs/WINDOWS-TEST-PLAN.md.' -ForegroundColor Green
