# Full Windows gate for one checkout: frontend build, policy check, JS and Python tests,
# clippy with -D warnings, then the Rust test suites, on x86_64-pc-windows-msvc.
# Usage: run-gate.ps1 [-Source <checkout>] [-Logs <directory>]
# Each step writes <Logs>\<step>.log and <step>.log.exit; the script exits 1 if any step failed.
# The checkout must be LF (see .gitattributes); a CRLF working tree fails the byte-exact tests.
param(
    [string]$Source = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path,
    [string]$Logs = 'E:\kiwix-win\gate-logs'
)
# A caller's 'Stop' would turn a tool's first stderr line into a terminating error.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'env.ps1')
$target = 'x86_64-pc-windows-msvc'
New-Item -ItemType Directory -Force $Logs | Out-Null
# sherpa-onnx's build script copies its runtime DLLs into the profile directory;
# test executables run from deps\ and find them through PATH.
$env:PATH = "$Source\src-tauri\target\$target\debug;$env:PATH"
Set-Location $Source
$failed = @()

function Step([string]$Name, [string]$Exe, [string[]]$ArgList) {
    $log = Join-Path $Logs "$Name.log"
    Remove-Item $log, "$log.exit" -ErrorAction SilentlyContinue
    # A tool that cannot be started leaves the previous command's exit code
    # behind; clear it so a missing tool is a failure, never an inherited 0.
    $global:LASTEXITCODE = $null
    try {
        # Native tools write progress to stderr; use the exit code, never the stream.
        & $Exe @ArgList 2>&1 | ForEach-Object { "$_" } | Out-File -Encoding utf8 $log
        $code = $LASTEXITCODE
    } catch {
        "$_" | Out-File -Append -Encoding utf8 $log
        $code = $null
    }
    if ($null -eq $code) { $code = 'not started' }
    Set-Content "$log.exit" $code
    if ($code -ne 0) { $script:failed += $Name }
    "{0}: exit={1}" -f $Name, $code
}

# The frontend is built first and nothing rebuilds it while cargo runs:
# the app embeds dist/ at compile time, and a concurrent `vite build` empties it.
Step 'npm-ci' 'npm.cmd' @('ci')
Step 'npm-build' 'npm.cmd' @('run', 'build')
Step 'unsafe-census' 'node' @('scripts/check-unsafe-census.mjs')
Step 'js-tests' 'node' @('--test', 'scripts/test-*.mjs')
Step 'py-tests' 'python' @('-m', 'unittest', 'discover', '-s', 'scripts', '-p', 'test_*.py')
$app = @('--locked', '--manifest-path', 'src-tauri/Cargo.toml')
$feature = @('--features', 'custom-protocol', '--target', $target)
Step 'clippy-app' 'cargo' (@('clippy') + $app + @('--all-targets') + $feature + @('--', '-D', 'warnings'))
Step 'clippy-mcp' 'cargo' @('clippy', '--locked', '--manifest-path', 'kiwix-mcp/Cargo.toml', '--all-targets', '--target', $target, '--', '-D', 'warnings')
Step 'clippy-win-security' 'cargo' @('clippy', '--locked', '--manifest-path', 'win-security/Cargo.toml', '--all-targets', '--target', $target, '--', '-D', 'warnings')
Step 'test-app' 'cargo' (@('test') + $app + $feature)
Step 'test-mcp' 'cargo' @('test', '--locked', '--manifest-path', 'kiwix-mcp/Cargo.toml', '--target', $target)
Step 'test-win-security' 'cargo' @('test', '--locked', '--manifest-path', 'win-security/Cargo.toml', '--target', $target)

if ($failed.Count -gt 0) { "FAILED: $($failed -join ', ')"; exit 1 }
'Gate passed.'
