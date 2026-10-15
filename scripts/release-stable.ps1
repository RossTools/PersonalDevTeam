#Requires -Version 5.1
<#
Builds the Personal Dev Team installer and installs it as the *stable* app (per user, no admin).
Run this yourself from a normal terminal. Agents must never run it without -BuildOnly: it stops the stable pdtd
that hosts them. Running sessions become 'interrupted' and can be restarted from the UI (agents resume).
#>
param([switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$port = 7420
$product = 'Personal Dev Team'

function Invoke-Native([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}
function Test-Port([int]$p) {
    try { $c = New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1', $p); $c.Close(); $true } catch { $false }
}

Push-Location $root
try {
    Invoke-Native 'pnpm install' { pnpm install --frozen-lockfile }
    Invoke-Native 'web build' { pnpm --dir web build }
    Invoke-Native 'pdtd build' { cargo build --release -p pdtd }
    Invoke-Native 'stage pdtd' { node scripts/stage-pdtd.mjs }
    $bundleConf = Join-Path $root 'desktop\src-tauri\tauri.bundle.conf.json'
    Invoke-Native 'installer build' { pnpm --dir desktop exec tauri build --config $bundleConf --bundles nsis }
} finally { Pop-Location }

$setup = Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw 'no NSIS installer was produced' }
Write-Host "Built $($setup.FullName)"
if ($BuildOnly) { return }

$installDir = Join-Path $env:LOCALAPPDATA $product
# The stable tray app holds no state: stop it so the installer can replace it (pdtd is stopped gracefully below).
$tray = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" -and $_.ProcessName -ne 'pdtd' }
if ($tray) {
    Write-Host 'Stopping the stable tray app...'
    $tray | Stop-Process -Force
    $tray | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

if (Test-Port $port) {
    Write-Host "Stopping the stable pdtd on port $port..."
    Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" | Out-Null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Test-Port $port) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Milliseconds 250 }
    if (Test-Port $port) { throw "pdtd on port $port did not stop within 20 seconds" }
}
Get-Process pdtd -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" } |
    Wait-Process -Timeout 10 -ErrorAction SilentlyContinue

Write-Host 'Installing...'
$p = Start-Process $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "installer failed (exit $($p.ExitCode))" }

$app = Get-ChildItem $installDir -Filter '*.exe' | Where-Object { $_.BaseName -notin @('pdtd', 'uninstall') } | Select-Object -First 1
if (-not $app) { throw "installed, but no app exe was found in $installDir" }
Write-Host "Installed to $installDir. Starting $($app.Name) (it starts pdtd)."
# Launch with no PDT_* variables so the stable app never inherits a dev setup (e.g. PDT_PROFILE=dev).
$psi = New-Object Diagnostics.ProcessStartInfo $app.FullName
$psi.UseShellExecute = $false
$psi.WorkingDirectory = $installDir
foreach ($k in @($psi.EnvironmentVariables.Keys)) { if ($k -like 'PDT_*') { $psi.EnvironmentVariables.Remove($k) } }
[Diagnostics.Process]::Start($psi) | Out-Null
