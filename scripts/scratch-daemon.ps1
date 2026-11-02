# A throwaway dev pdtd for an agent to look at (screenshots, Playwright), started and stopped without commands an
# agent's approval prompt can't judge. The Windows twin of scripts/scratch-daemon.sh (read it for the details): each
# daemon gets a free port and a scratch folder <temp dir>\pdt-scratch\<port>; Stop deletes only folders directly under
# that parent and refuses anything else. Tests: scripts/scratch-daemon.test.ps1.
#
#   scripts\scratch-daemon.ps1 start [-Seed <seed>]   start one; prints port=, dir=, url=
#   scripts\scratch-daemon.ps1 pair <port>            a pairing link (good once, for 5 minutes) to open first in the
#                                                     test browser
#   scripts\scratch-daemon.ps1 stop <port>            stop it by port and delete its scratch folder
#   scripts\scratch-daemon.ps1 list                   the scratch folders and whether each daemon is up
#
# Set PDT_SCRATCH_BIN for another pdtd than target\debug\pdtd.exe. `start` also removes scratch folders older than a
# day whose daemon is gone.
param(
    [Parameter(Position = 0)][string]$Command = '',
    [Parameter(Position = 1)][string]$Port = '',
    [string]$Seed = 'demo'
)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$tmp = if ($env:TMPDIR) { $env:TMPDIR } else { [System.IO.Path]::GetTempPath() }
$parentPath = Join-Path $tmp 'pdt-scratch'
New-Item -ItemType Directory -Force -Path $parentPath | Out-Null
$parent = (Get-Item -LiteralPath $parentPath).FullName.TrimEnd('\', '/')
$firstPort = 7600
$lastPort = 7799

function Fail([string]$message) {
    [Console]::Error.WriteLine("scratch-daemon: $message")
    exit 1
}

function Get-Listeners([int]$p) {
    @(Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty OwningProcess -Unique | Where-Object { $_ -ne 0 })
}

# The scratch folder for a port, or a refusal: only <parent>\<digits>, a real folder (not a link) inside parent.
function Get-ScratchDir([string]$p) {
    if ($p -notmatch '^[0-9]+$') { Fail "not a port: '$p' (stop takes the port start printed)" }
    $dir = Join-Path $parent $p
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { Fail "no scratch folder for port $p under $parent" }
    $item = Get-Item -LiteralPath $dir -Force
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { Fail "refusing ${dir}: a link, not a scratch folder" }
    if ($item.FullName.TrimEnd('\', '/') -ne (Join-Path $parent $p)) { Fail "refusing ${dir}: outside $parent" }
    $dir
}

function Stop-Port([string]$p) {
    $dir = Get-ScratchDir $p
    foreach ($id in Get-Listeners ([int]$p)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    for ($i = 0; $i -lt 50 -and (Get-Listeners ([int]$p)).Count -gt 0; $i++) { Start-Sleep -Milliseconds 200 }
    # The process may hold its files for a moment after it exits.
    for ($i = 0; $i -lt 25; $i++) {
        try { Remove-Item -LiteralPath $dir -Recurse -Force; break } catch { Start-Sleep -Milliseconds 200 }
    }
    if (Test-Path -LiteralPath $dir) { Fail "could not delete $dir" }
    "stopped port $p, deleted $dir"
}

# Folders older than a day whose daemon is gone (a crashed agent's leftovers).
function Invoke-Prune {
    $cutoff = (Get-Date).AddDays(-1)
    foreach ($d in Get-ChildItem -LiteralPath $parent -Directory -ErrorAction SilentlyContinue) {
        if ($d.Name -notmatch '^[0-9]+$' -or $d.LastWriteTime -gt $cutoff) { continue }
        if ($d.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { continue }
        if ((Get-Listeners ([int]$d.Name)).Count -gt 0) { continue }
        try { Remove-Item -LiteralPath $d.FullName -Recurse -Force } catch {}
    }
}

function Start-Scratch {
    $bin = if ($env:PDT_SCRATCH_BIN) { $env:PDT_SCRATCH_BIN } else { Join-Path $root 'target\debug\pdtd.exe' }
    if (-not (Test-Path -LiteralPath $bin)) { Fail "no pdtd at ${bin}: build it first (cargo build -p pdtd)" }
    Invoke-Prune
    $port = $null
    for ($p = $firstPort; $p -le $lastPort; $p++) {
        if ((Get-Listeners $p).Count -gt 0) { continue }
        $dir = Join-Path $parent "$p"
        if (Test-Path -LiteralPath $dir) { continue }
        New-Item -ItemType Directory -Path $dir | Out-Null
        $port = $p
        break
    }
    if (-not $port) { Fail "no free port in $firstPort-$lastPort" }
    $vars = @{
        PDT_PROFILE = 'dev'; PDT_PORT = "$port"; PDT_DATA_DIR = (Join-Path $dir 'data')
        PDT_WEB_DIR = (Join-Path $root 'web\dist'); PDT_SECRET_STORE = 'memory'; PDT_SEED = $Seed
        PDT_TAILSCALE_BIN = 'C:\nonexistent'; PDT_GITHUB_OFFLINE = '1'; PDT_BROWSER = (Join-Path $root 'scripts\scratch-browser.cmd')
    }
    $saved = @{}
    foreach ($k in $vars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $vars[$k]) }
    try {
        Start-Process -FilePath $bin -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $dir 'pdtd.out') -RedirectStandardError (Join-Path $dir 'pdtd.err') | Out-Null
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    $url = "http://127.0.0.1:$port"
    for ($i = 0; $i -lt 150; $i++) {
        try {
            Invoke-WebRequest -Uri "$url/api/health" -UseBasicParsing -TimeoutSec 2 | Out-Null
            "port=$port"; "dir=$dir"; "url=$url"
            [Console]::Error.WriteLine("stop it with: scripts\scratch-daemon.ps1 stop $port")
            return
        } catch { Start-Sleep -Milliseconds 200 }
    }
    [Console]::Error.WriteLine("scratch-daemon: pdtd did not answer on $url")
    Get-Content -LiteralPath (Join-Path $dir 'pdtd.err') -Tail 20 -ErrorAction SilentlyContinue | ForEach-Object { [Console]::Error.WriteLine($_) }
    try { Stop-Port "$port" | Out-Null } catch {}
    exit 1
}

function Get-PairLink([string]$p) {
    $dir = Get-ScratchDir $p
    $file = Join-Path $dir 'pair-url'
    Remove-Item -LiteralPath $file -ErrorAction SilentlyContinue
    # pdtd opens one browser every few seconds at most: a second pair soon after the first waits its turn.
    for ($try = 1; ; $try++) {
        try { Invoke-WebRequest -Uri "http://127.0.0.1:$p/api/ui/open" -Method Post -UseBasicParsing -TimeoutSec 5 | Out-Null; break }
        catch { if ($try -ge 5) { Fail "pdtd on port $p opened no pairing link" }; Start-Sleep -Seconds 1 }
    }
    for ($i = 0; $i -lt 25; $i++) {
        if ((Test-Path -LiteralPath $file) -and (Get-Item -LiteralPath $file).Length -gt 0) {
            "pair=$((Get-Content -LiteralPath $file -Raw).Trim())"
            return
        }
        Start-Sleep -Milliseconds 200
    }
    Fail "no pairing link from pdtd on port $p"
}

function Get-ScratchList {
    foreach ($d in Get-ChildItem -LiteralPath $parent -Directory -ErrorAction SilentlyContinue) {
        $state = 'down'
        if ($d.Name -match '^[0-9]+$' -and (Get-Listeners ([int]$d.Name)).Count -gt 0) { $state = 'up' }
        "$($d.Name)`t$state`t$($d.FullName)"
    }
}

switch ($Command) {
    'start' { Start-Scratch }
    'pair' { if (-not $Port) { Fail 'usage: scratch-daemon.ps1 pair <port>' }; Get-PairLink $Port }
    'stop' { if (-not $Port) { Fail 'usage: scratch-daemon.ps1 stop <port>' }; Stop-Port $Port }
    'list' { Get-ScratchList }
    default { Fail 'usage: scratch-daemon.ps1 start [-Seed <seed>] | pair <port> | stop <port> | list' }
}
