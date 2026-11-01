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

# The version the build reports (pdt_core::BUILD_VERSION): PDT_BUILD_VERSION, else the workspace version + -local.
$workspaceVersion = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$buildVersion = if ($env:PDT_BUILD_VERSION) { $env:PDT_BUILD_VERSION } else { "$workspaceVersion-local" }

Push-Location $root
try {
    Invoke-Native 'pnpm install' { pnpm install --frozen-lockfile }
    Invoke-Native 'web build' { pnpm --dir web build }
    Invoke-Native 'pdtd and pdt-host build' { cargo build --release -p pdtd -p pdt-host }
    Invoke-Native 'stage sidecars' { node scripts/stage-sidecars.mjs }
    $bundleConf = Join-Path $root 'desktop\src-tauri\tauri.bundle.conf.json'
    Invoke-Native 'installer build' { pnpm --dir desktop exec tauri build --config $bundleConf --bundles nsis }
} finally { Pop-Location }

$setup = Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw 'no NSIS installer was produced' }
Write-Host "Built $($setup.FullName)"
if ($BuildOnly) { return }

$dataDir = Join-Path $env:LOCALAPPDATA 'pdt'
$status = $null
try { $status = Invoke-RestMethod -Uri "http://127.0.0.1:$port/api/host/update" -TimeoutSec 5 } catch {}
if ($status -and $status.state -ne 'disabled') {
    # pdt-host supervises the stable pdtd: hand the installer over like a GitHub release (with health check and rollback).
    $drop = Join-Path $dataDir 'update\downloads\local'
    New-Item -ItemType Directory -Force $drop | Out-Null
    Copy-Item $setup.FullName (Join-Path $drop $setup.Name) -Force
    # pdt-host's health check expects exactly the version the new build reports.
    $request = @{ version = $buildVersion; tag = 'local'; asset = (Join-Path $drop $setup.Name);
        fromVersion = $status.current; requestedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() } | ConvertTo-Json
    # Written whole, then renamed: pdt-host acts as soon as request.json appears.
    $requestPath = Join-Path $dataDir 'update\request.json'
    [IO.File]::WriteAllText("$requestPath.tmp", $request)
    Move-Item "$requestPath.tmp" $requestPath -Force
    # pdt-host also stops pdtd once it sees the request, so pdtd may already be gone.
    try { Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" -TimeoutSec 10 | Out-Null } catch {}
    Write-Host "Handed the installer to pdt-host; see $dataDir\logs\pdt-host.log and $dataDir\update\result.json."
    return
}

$installDir = Join-Path $env:LOCALAPPDATA $product
# First install (no pdt-host takes updates yet). The stable tray app, and a pdt-host that cannot take the update, hold
# no state: stop them so the installer can replace them and nothing starts pdtd again (pdtd is stopped gracefully
# below). The installer's hook registers and starts pdt-host.
$tray = Get-Process -ErrorAction SilentlyContinue | Where-Object {
    ($_.Path -like "$installDir\*" -and $_.ProcessName -ne 'pdtd') -or $_.Path -like "$dataDir\host\*" }
if ($tray) {
    Write-Host 'Stopping the stable tray app and pdt-host...'
    $tray | Stop-Process -Force
    # Wait-Process reports a timeout as an error that SilentlyContinue hides: check what is still running instead.
    $tray | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    $left = @($tray | Where-Object { -not $_.HasExited })
    if ($left) { throw "still running 10 seconds after being stopped: $(($left | ForEach-Object { "$($_.ProcessName) ($($_.Id))" }) -join ', ')" }
}

if (Test-Port $port) {
    Write-Host "Stopping the stable pdtd on port $port..."
    Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" | Out-Null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Test-Port $port) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Milliseconds 250 }
    if (Test-Port $port) { throw "pdtd on port $port did not stop within 20 seconds" }
}
$daemon = @(Get-Process pdtd -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" })
$daemon | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
if (@($daemon | Where-Object { -not $_.HasExited })) { throw "pdtd ($($daemon.Id -join ', ')) did not exit within 10 seconds" }

Write-Host 'Installing...'
# The installer's hook starts pdt-host with the installer's environment: no dev setup (e.g. PDT_PROFILE=dev) may leak in.
Get-ChildItem env: | Where-Object { $_.Name -like 'PDT_*' } | ForEach-Object { Remove-Item "env:$($_.Name)" }
$p = Start-Process $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "installer failed (exit $($p.ExitCode))" }

$app = Get-ChildItem $installDir -Filter '*.exe' | Where-Object { $_.BaseName -notin @('pdtd', 'pdt-host', 'uninstall') } | Select-Object -First 1
if (-not $app) { throw "installed, but no app exe was found in $installDir" }
Write-Host "Installed to $installDir; the installer started pdt-host (it starts pdtd). Starting $($app.Name)."
# Launch with no PDT_* variables so the stable app never inherits a dev setup (e.g. PDT_PROFILE=dev).
$psi = New-Object Diagnostics.ProcessStartInfo $app.FullName
$psi.UseShellExecute = $false
$psi.WorkingDirectory = $installDir
foreach ($k in @($psi.EnvironmentVariables.Keys)) { if ($k -like 'PDT_*') { $psi.EnvironmentVariables.Remove($k) } }
[Diagnostics.Process]::Start($psi) | Out-Null
