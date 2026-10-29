# Tests for scripts/scratch-daemon.ps1's refusals, in a scratch TMPDIR. Run: pwsh scripts/scratch-daemon.test.ps1
# (CI runs it in the Windows build). scripts/scratch-daemon.test.sh also starts and stops a stand-in daemon.
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'scratch-daemon.ps1'
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("scratch-daemon-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $scratch | Out-Null
$failed = $false
function Fail-Test([string]$name) { [Console]::Error.WriteLine("FAIL: $name"); $script:failed = $true }

$savedTmp = $env:TMPDIR
try {
    $env:TMPDIR = Join-Path $scratch 'tmp'
    New-Item -ItemType Directory -Path $env:TMPDIR | Out-Null
    $parent = Join-Path $env:TMPDIR 'pdt-scratch'
    $outside = Join-Path $scratch 'outside'
    New-Item -ItemType Directory -Path $outside | Out-Null
    Set-Content -LiteralPath (Join-Path $outside 'file') -Value keep

    function Invoke-Stop([string]$arg) {
        & pwsh -NoProfile -File $script stop $arg 2>$null | Out-Null
        $LASTEXITCODE
    }
    foreach ($case in @(
            @{ name = 'a path'; arg = $outside },
            @{ name = 'a relative path'; arg = '..\outside' },
            @{ name = 'a parent escape'; arg = '..' },
            @{ name = 'a glob'; arg = '*' },
            @{ name = 'a port with no scratch folder'; arg = '7799' })) {
        if ((Invoke-Stop $case.arg) -eq 0) { Fail-Test "stop refuses $($case.name)" }
    }
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    New-Item -ItemType Junction -Path (Join-Path $parent '7798') -Target $outside | Out-Null
    if ((Invoke-Stop '7798') -eq 0) { Fail-Test 'stop refuses a link out of the parent' }
    if (-not (Test-Path -LiteralPath (Join-Path $outside 'file'))) { Fail-Test 'nothing outside the parent is deleted' }
    (Get-Item -LiteralPath (Join-Path $parent '7798')).Delete()

    New-Item -ItemType Directory -Path (Join-Path $parent '7791') | Out-Null
    if ((Invoke-Stop '7791') -ne 0) { Fail-Test 'stop deletes a scratch folder with no daemon' }
    if (Test-Path -LiteralPath (Join-Path $parent '7791')) { Fail-Test 'stop deletes the folder' }
    if (-not (Test-Path -LiteralPath (Join-Path $outside 'file'))) { Fail-Test 'nothing outside the parent is deleted' }
} finally {
    $env:TMPDIR = $savedTmp
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failed) { [Console]::Error.WriteLine('scratch-daemon tests failed'); exit 1 }
'scratch-daemon tests passed'
