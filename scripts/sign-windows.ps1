# Signs Windows files with Azure Artifact Signing (Authenticode, timestamped), or with -Verify checks that each one
# carries a valid signature. Unsigned releases are what Defender's machine-learning detections quarantine (PDT-30);
# setup: docs/superpowers/notes/windows-signing.md.
#
#   sign-windows.ps1 <file>...          sign (Tauri's signCommand calls it once per file)
#   sign-windows.ps1 -Verify <file>...  fail unless every file is validly signed
#
# Signing needs ARTIFACT_SIGNING_ENDPOINT (e.g. https://wus2.codesigning.azure.net), ARTIFACT_SIGNING_ACCOUNT,
# ARTIFACT_SIGNING_PROFILE (the certificate profile) and an app registration with the "Artifact Signing Certificate
# Profile Signer" role: AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET.
param(
    [switch]$Verify,
    [Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Files
)
$ErrorActionPreference = 'Stop'

if ($Verify) {
    $bad = 0
    foreach ($f in $Files) {
        $sig = Get-AuthenticodeSignature -FilePath $f
        $signer = if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { '-' }
        Write-Host "$($sig.Status)  $f  ($signer)"
        if ($sig.Status -ne 'Valid' -or -not $sig.TimeStamperCertificate) { $bad++ }
    }
    if ($bad) { throw "$bad file(s) are not validly signed and timestamped" }
    exit 0
}

foreach ($v in 'ARTIFACT_SIGNING_ENDPOINT', 'ARTIFACT_SIGNING_ACCOUNT', 'ARTIFACT_SIGNING_PROFILE',
    'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET') {
    if (-not [Environment]::GetEnvironmentVariable($v)) { throw "$v is not set" }
}

# The signing client (a SignTool plug-in) from NuGet, fetched once per machine.
$ClientVersion = '1.0.128'
$temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$tools = Join-Path $temp "artifact-signing-client-$ClientVersion"
$dlib = Join-Path $tools 'bin\x64\Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path $dlib)) {
    $pkg = "$tools.zip"
    Invoke-WebRequest "https://www.nuget.org/api/v2/package/Microsoft.ArtifactSigning.Client/$ClientVersion" -OutFile $pkg
    Expand-Archive $pkg $tools -Force
    if (-not (Test-Path $dlib)) { throw "the signing client has no $dlib" }
}

# The newest x64 SignTool from the Windows SDK (the dlib needs 10.0.22621.755 or later).
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe was not found (install the Windows SDK)' }

$metadata = Join-Path $tools 'metadata.json'
@{
    Endpoint               = $env:ARTIFACT_SIGNING_ENDPOINT
    CodeSigningAccountName = $env:ARTIFACT_SIGNING_ACCOUNT
    CertificateProfileName = $env:ARTIFACT_SIGNING_PROFILE
} | ConvertTo-Json | Set-Content -Path $metadata -Encoding utf8

# Timestamped: the certificates last days, the timestamp keeps the signature valid after they expire.
& $signtool.FullName sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib $dlib /dmdf $metadata @Files
if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE) signing $($Files -join ', ')" }
