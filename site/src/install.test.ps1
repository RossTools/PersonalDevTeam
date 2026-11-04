# public/install.ps1's release signature check, on a signature openssl made (as the release workflow does) with a
# throwaway key. Loads only its functions (the script itself would install). CI runs it with pwsh in the `site` job:
#
#   pwsh site/src/install.test.ps1

$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '../public/install.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile($script, [ref]$null, [ref]$null)
foreach ($name in 'ConvertFrom-DerSignature', 'Test-ReleaseSignature') {
    $fn = $ast.Find({ param($a) $a -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $a.Name -eq $name }, $true)
    if (-not $fn) { throw "install.ps1 has no $name" }
    . ([scriptblock]::Create($fn.Extent.Text))
}

function FromHex([string]$hex) { [byte[]]($hex -split '(..)' -ne '' | ForEach-Object { [Convert]::ToByte($_, 16) }) }

$key = 'MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEZv3zoT15o6bXkEaJR39Kgk3t5Gawa5svOK89OHGRwZTgRkkH3/Y9igSbiKXQn6aeXX6m6nSzQ2Ifrvt3n/zJYg=='
$sig = FromHex ('3046022100c6b05e5948e87080e70255fe5f4d4eb4de68528119586cd435ff15febf755827022100b3b3170fa6016bd29c5e89960c4b' +
    'e6373a775dd6ad1093159a1287142083726e')
$sums = [Text.Encoding]::UTF8.GetBytes("abc  a.tar.gz`n")
$other = [Text.Encoding]::UTF8.GetBytes("abd  a.tar.gz`n")

$failed = 0
function Check([string]$what, [bool]$want, [bool]$got) {
    if ($want -ne $got) { Write-Host "FAIL: $what (expected $want, got $got)"; $script:failed++ }
}
Check 'openssl signature verifies' $true (Test-ReleaseSignature $key 'v1.2.3' $sums $sig)
Check 'bound to the tag' $false (Test-ReleaseSignature $key 'v1.2.4' $sums $sig)
Check 'bound to SHA256SUMS' $false (Test-ReleaseSignature $key 'v1.2.3' $other $sig)
Check 'junk signature' $false (Test-ReleaseSignature $key 'v1.2.3' $sums ([byte[]](1, 2, 3)))
Check 'truncated signature' $false (Test-ReleaseSignature $key 'v1.2.3' $sums ([byte[]]$sig[0..60]))
Check 'junk key' $false (Test-ReleaseSignature 'AAAA' 'v1.2.3' $sums $sig)
Check 'not base64' $false (Test-ReleaseSignature 'not base64!' 'v1.2.3' $sums $sig)

if ($failed) { exit 1 }
Write-Host 'install.ps1 signature tests passed'
