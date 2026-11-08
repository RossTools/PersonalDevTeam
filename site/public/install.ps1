# Installs Personal Dev Team (PDT) on Windows, without admin rights (Windows PowerShell 5.1 or PowerShell 7):
#
#   irm https://pdt.dev/install.ps1 | iex
#
# `irm | iex` takes no arguments, so the options are environment variables, set first:
#
#   $env:PDT_CHANNEL = 'nightly'   # the newest nightly build instead of the newest stable release
#   $env:PDT_VERSION = '0.3.0'     # an exact release
#   $env:PDT_FORCE = '1'           # install again over an existing install
#   $env:GITHUB_TOKEN = '...'      # optional: only for a private fork, or when GitHub's rate limit is used up
#
# or, run as a script block, parameters: & ([scriptblock]::Create((irm https://pdt.dev/install.ps1))) -Channel nightly
#
# It downloads the installer (*-setup.exe) from GitHub (without GITHUB_TOKEN, a stable release takes no call to GitHub's
# API, a nightly one call), checks that the release's SHA256SUMS carries PDT's release signature (SHA256SUMS.sig,
# against the key in Install-Pdt) and the installer against SHA256SUMS, and runs it silently; the installer starts PDT
# and registers it to start at login. macOS and Linux have install.sh.
#
# Everything runs from Install-Pdt, called on the last line: a download cut short runs nothing. It never calls `exit`,
# which would close the PowerShell window `iex` runs in.
#
# For tests only: PDT_INSTALL_API (GitHub's API, https://api.github.com), PDT_INSTALL_WEB (where releases download
# from, https://github.com), PDT_INSTALL_LOCAL (where a running PDT answers, http://127.0.0.1:7420) and
# PDT_INSTALL_SIGNING_KEY (the release signing key).

param(
    [string]$Channel = $env:PDT_CHANNEL,
    [string]$Version = $env:PDT_VERSION,
    [switch]$Force
)

# An ECDSA signature as DER (SEQUENCE of INTEGER r, INTEGER s; what openssl writes) to r and s as 32 bytes each, the
# form .NET verifies. $null when it isn't one.
function ConvertFrom-DerSignature([byte[]]$der) {
    if ($der.Length -lt 8 -or $der[0] -ne 0x30 -or $der[1] -ne $der.Length - 2) { return $null }
    $out = New-Object byte[] 64
    $i = 2
    foreach ($half in 0, 1) {
        if ($i + 2 -gt $der.Length -or $der[$i] -ne 2) { return $null }
        $len = [int]$der[$i + 1]
        $start = $i + 2
        $i = $start + $len
        if ($i -gt $der.Length) { return $null }
        while ($len -gt 0 -and $der[$start] -eq 0) { $start++; $len-- }
        if ($len -gt 32) { return $null }
        [Array]::Copy($der, $start, $out, $half * 32 + 32 - $len, $len)
    }
    if ($i -ne $der.Length) { return $null }
    return , $out
}

# SHA256SUMS.sig is an ECDSA P-256 signature (DER) over the tag, a newline, then SHA256SUMS: it vouches for this
# release only. $key is base64 of the public key's DER (SubjectPublicKeyInfo). Works on .NET Framework 4.7+ (Windows
# PowerShell 5.1) and .NET.
function Test-ReleaseSignature([string]$key, [string]$tag, [byte[]]$sums, [byte[]]$signature) {
    try {
        $spki = [Convert]::FromBase64String($key)
        # A P-256 SubjectPublicKeyInfo is 91 bytes and ends with the uncompressed point: 0x04, X, Y.
        if ($spki.Length -ne 91 -or $spki[26] -ne 4) { return $false }
        $rs = ConvertFrom-DerSignature -der $signature
        if (-not $rs) { return $false }
        $point = New-Object System.Security.Cryptography.ECPoint
        $point.X = [byte[]]$spki[27..58]
        $point.Y = [byte[]]$spki[59..90]
        $params = New-Object System.Security.Cryptography.ECParameters
        $params.Curve = [System.Security.Cryptography.ECCurve+NamedCurves]::nistP256
        $params.Q = $point
        $ecdsa = [System.Security.Cryptography.ECDsa]::Create($params)
        $message = [byte[]]([Text.Encoding]::UTF8.GetBytes("$tag`n") + $sums)
        return $ecdsa.VerifyData($message, $rs, [System.Security.Cryptography.HashAlgorithmName]::SHA256)
    } catch {
        return $false
    }
}

# The newest stable release's tag, from where github.com's releases/latest ($url) redirects (…/releases/tag/<tag>).
# $null when it doesn't redirect to a tag (no release yet, or no answer).
function Get-LatestTag([string]$url) {
    try {
        $request = [Net.HttpWebRequest]::Create($url)
        $request.AllowAutoRedirect = $false
        $request.UserAgent = 'pdt-install'
        # .NET answers a redirect it isn't following as the response, or (some versions) as a WebException holding it.
        try { $response = $request.GetResponse() } catch [Net.WebException] { $response = $_.Exception.Response }
        if (-not $response) { return $null }
        $location = $response.Headers['Location']
        $response.Close()
    } catch { return $null }
    if ($location -match '/releases/tag/([^/?#]+)$') { return [Uri]::UnescapeDataString($Matches[1]) }
    return $null
}

function Install-Pdt {
    param([string]$Channel, [string]$Version, [bool]$Force)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $repo = 'RossTools/PersonalDevTeam'
    $product = 'Personal Dev Team'
    $port = 7420
    $site = 'https://pdt.dev'
    # The release signing key (base64 of its public key's DER; the same as RELEASE_SIGNING_KEY in PDT's source). Empty
    # until releases are signed: then SHA256SUMS alone is checked.
    $signingKey = ''
    if ($env:PDT_INSTALL_SIGNING_KEY) { $signingKey = $env:PDT_INSTALL_SIGNING_KEY }
    $api = if ($env:PDT_INSTALL_API) { $env:PDT_INSTALL_API } else { 'https://api.github.com' }
    $web = if ($env:PDT_INSTALL_WEB) { $env:PDT_INSTALL_WEB } else { 'https://github.com' }
    $local = if ($env:PDT_INSTALL_LOCAL) { $env:PDT_INSTALL_LOCAL } else { "http://127.0.0.1:$port" }

    function Write-Failure([string]$message) {
        Write-Host "pdt install: $message" -ForegroundColor Red
    }

    if (-not $Channel) { $Channel = 'stable' }
    if ($Channel -ne 'stable' -and $Channel -ne 'nightly') { return Write-Failure "PDT_CHANNEL is stable or nightly, not '$Channel'." }
    $Version = "$Version".TrimStart('v')
    if ($Version -and $Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') {
        return Write-Failure "PDT_VERSION is a release version like 0.3.0, not '$Version'."
    }

    if ([Environment]::OSVersion.Platform -ne 'Win32NT') {
        return Write-Failure "this installs $product on Windows. On macOS and Linux: curl -fsSL $site/install.sh | sh"
    }
    # The installer is x64; Windows on Arm runs it emulated.
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    if ($arch -ne 'AMD64' -and $arch -ne 'ARM64') { return Write-Failure "$product for Windows needs a 64-bit Windows (this one is $arch)." }

    # GitHub needs TLS 1.2, which Windows PowerShell 5.1 may not offer by default.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    # Installed: the installer's folder (a per-user install), or a PDT answering on its port. An install counts as one
    # only when it starts at login (pdt-host's Run value) or runs: files left by an install cut short are installed again.
    $installDir = Join-Path $env:LOCALAPPDATA $product
    $existing = Test-Path (Join-Path $installDir 'pdt-host.exe')
    $registered = $false
    if ($existing) {
        try {
            $null = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Personal Dev Team host' -ErrorAction Stop
            $registered = $true
        } catch { $registered = $false }
    }
    $running = $false
    try {
        $health = Invoke-RestMethod -Uri "$local/api/health" -TimeoutSec 3 -UseBasicParsing
        $running = $health.name -eq $product
    } catch { $running = $false }
    # Stops this install's pdt-host and pdtd (`pdt-host stop`, until the next login) and any left running from the
    # install folder, then waits until PDT no longer answers on its port and pdtd.exe can be replaced. Returns why it
    # could not, or nothing once it is stopped.
    function Close-RunningPdt {
        $daemon = Join-Path $installDir 'pdtd.exe'
        $hostExe = Join-Path $installDir 'pdt-host.exe'
        if (Test-Path $hostExe) {
            Write-Host "Stopping the running $product..."
            # Its exit code and output don't matter: the wait below checks what it did.
            $stop = Start-Process -FilePath $hostExe -ArgumentList 'stop' -WindowStyle Hidden -PassThru
            $null = $stop.Handle
            $null = $stop.WaitForExit(30000)
        }
        # A stray the stop missed (a second pdt-host's pdtd, say) still keeps pdtd.exe open.
        Get-Process -Name 'pdtd', 'pdt-host' -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and $_.Path.StartsWith("$installDir\", [StringComparison]::OrdinalIgnoreCase) } |
            Stop-Process -Force -ErrorAction SilentlyContinue
        $deadline = (Get-Date).AddSeconds(30)
        while ($true) {
            $answers = $false
            try {
                $answers = (Invoke-RestMethod -Uri "$local/api/health" -TimeoutSec 2 -UseBasicParsing).name -eq $product
            } catch { $answers = $false }
            $inUse = $false
            if (Test-Path $daemon) {
                try { [IO.File]::Open($daemon, 'Open', 'ReadWrite', 'None').Close() } catch { $inUse = $true }
            }
            if (-not $answers -and -not $inUse) { return }
            if ((Get-Date) -gt $deadline) {
                $what = if ($inUse) { "$daemon is still in use" } else { "it still answers at $local" }
                return "could not stop the running $product ($what). Quit it from its tray icon (Stop PDT and quit) and run this again. Nothing was installed."
            }
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $Force) {
        if ($running) {
            if ($existing) { Write-Host "$product is already installed in $installDir, and running." } else { Write-Host "$product is already running at $local." }
            Write-Host 'It keeps itself up to date: Settings > This host > Updates shows its version and checks for a new one.'
            Write-Host "To install it again anyway, set `$env:PDT_FORCE = '1' and run this again."
            return
        }
        if ($registered) {
            Write-Host "$product is installed in $installDir, but it isn't running."
            Write-Host "Start it by opening $product from the Start menu."
            Write-Host "To install it again instead, set `$env:PDT_FORCE = '1' and run this again."
            return
        }
        if ($existing) { Write-Host "$product is in $installDir but isn't set up to start at login: installing it again." }
    }

    $headers = @{ Accept = 'application/vnd.github+json' }
    if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
    $releases = "https://github.com/$repo/releases"
    $noReleases = if ($env:GITHUB_TOKEN) {
        "could not read $product's releases from GitHub with GITHUB_TOKEN: check that it can read $repo."
    } else {
        "could not read $product's releases from GitHub. Try again later; if GitHub's rate limit is used up or $repo is a private fork, set `$env:GITHUB_TOKEN to a token that can read it and run this again."
    }
    $noNightly = "found no nightly build of $product (the releases: $releases)."
    # The newest release a host on the nightly channel takes: a nightly build or a full release, listed newest first.
    $nightlyTag = '^v\d+\.\d+\.\d+(-nightly\.[0-9.]+)?$'

    # Without a token, GitHub's API is used only to list the nightly builds (an IP has 60 anonymous API calls an hour):
    # the newest stable release's tag comes from github.com's releases/latest redirect, and the files download by name
    # from github.com, which has no such limit. With one, everything goes through the API (a private fork needs that).
    Write-Host 'Finding the release...'
    $release = $null
    try {
        if (-not $env:GITHUB_TOKEN) {
            if ($Version) {
                $tag = "v$Version"
            } elseif ($Channel -eq 'nightly') {
                $list = Invoke-RestMethod -Uri "$api/repos/$repo/releases?per_page=30" -Headers $headers -UseBasicParsing
                $tag = @($list | Where-Object { $_.tag_name -match $nightlyTag }) | Select-Object -First 1 -ExpandProperty tag_name
                if (-not $tag) { return Write-Failure $noNightly }
            } else {
                $tag = Get-LatestTag -url "$web/$repo/releases/latest"
                if (-not $tag) { return Write-Failure $noReleases }
            }
        } elseif ($Version) {
            try {
                $release = Invoke-RestMethod -Uri "$api/repos/$repo/releases/tags/v$Version" -Headers $headers -UseBasicParsing
            } catch { return Write-Failure "there is no $product release v$Version (the releases: $releases)." }
        } elseif ($Channel -eq 'nightly') {
            $list = Invoke-RestMethod -Uri "$api/repos/$repo/releases?per_page=30" -Headers $headers -UseBasicParsing
            $release = @($list | Where-Object { $_.tag_name -match $nightlyTag }) | Select-Object -First 1
            if (-not $release) { return Write-Failure $noNightly }
        } else {
            $release = Invoke-RestMethod -Uri "$api/repos/$repo/releases/latest" -Headers $headers -UseBasicParsing
        }
    } catch { return Write-Failure $noReleases }
    if ($release) { $tag = $release.tag_name }

    # Downloads the release file named $name to $out: from github.com, or through the API with a token. Returns
    # nothing once it is there, else GitHub's HTTP status (0: no answer; 404 also for a file the release doesn't list).
    function Get-ReleaseFile([string]$name, [string]$out) {
        if ($release) {
            $asset = @($release.assets | Where-Object { $_.name -eq $name }) | Select-Object -First 1
            if (-not $asset) { return 404 }
            $uri = "$api/repos/$repo/releases/assets/$($asset.id)"
            $download = @{ Accept = 'application/octet-stream'; Authorization = "Bearer $env:GITHUB_TOKEN" }
        } else {
            $uri = "$web/$repo/releases/download/$tag/$([Uri]::EscapeDataString($name))"
            $download = @{}
        }
        try {
            Invoke-WebRequest -Uri $uri -Headers $download -OutFile $out -UseBasicParsing
        } catch {
            $status = 0
            if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
            return $status
        }
    }
    # Why a download of $name failed with $status other than a 404, when GitHub's answer tells.
    function Get-DownloadFailure([string]$name, [int]$status) {
        $why = switch ($status) {
            0 { 'GitHub did not answer: check the connection and try again.' }
            401 { "GitHub refused GITHUB_TOKEN (HTTP 401): check that it is current and can read $repo." }
            { $_ -eq 403 -or $_ -eq 429 } {
                if ($env:GITHUB_TOKEN) {
                    "GitHub refused it (HTTP $status): check that GITHUB_TOKEN can read $repo, or wait for its rate limit to reset."
                } else {
                    "GitHub refused it (HTTP $status): its rate limit may be used up. Try again later, or set `$env:GITHUB_TOKEN to a token that can read $repo and run this again."
                }
            }
            default { "GitHub answered HTTP $status. Try again later." }
        }
        return "could not download $name of $($tag). $why"
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("pdt-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $sumsFile = Join-Path $tmp 'SHA256SUMS'
        $sigFile = Join-Path $tmp 'SHA256SUMS.sig'
        # SHA256SUMS lists the release's files: the installer is picked from it.
        $failed = Get-ReleaseFile -name 'SHA256SUMS' -out $sumsFile
        if ($failed -eq 404 -and $Version) { return Write-Failure "there is no $product release v$Version (the releases: $releases)." }
        if ($failed -eq 404) { return Write-Failure "release $tag has no SHA256SUMS, so the download can't be checked. Nothing was installed." }
        if ($null -ne $failed) { return Write-Failure (Get-DownloadFailure -name 'SHA256SUMS' -status $failed) }
        if ($signingKey) {
            $failed = Get-ReleaseFile -name 'SHA256SUMS.sig' -out $sigFile
            if ($failed -eq 404) {
                return Write-Failure "release $tag is not signed (it has no SHA256SUMS.sig), so its SHA256SUMS can't be trusted. Nothing was installed."
            }
            if ($null -ne $failed) { return Write-Failure (Get-DownloadFailure -name 'SHA256SUMS.sig' -status $failed) }
            if (-not (Test-ReleaseSignature -key $signingKey -tag $tag -sums ([IO.File]::ReadAllBytes($sumsFile)) -signature ([IO.File]::ReadAllBytes($sigFile)))) {
                return Write-Failure "the signature of $tag's SHA256SUMS does not match $product's release key. Nothing was installed."
            }
            Write-Host 'Signature verified.'
        }
        $sums = @{}
        $assetName = $null
        foreach ($line in Get-Content $sumsFile) {
            $hash, $name = $line -split '\s+\*?', 2
            if (-not $name) { continue }
            $sums[$name] = $hash.ToLowerInvariant()
            if (-not $assetName -and $name -like '*-setup.exe') { $assetName = $name }
        }
        if (-not $assetName) { return Write-Failure "release $tag has no installer for Windows." }

        $exe = Join-Path $tmp $assetName
        Write-Host "Downloading $product $tag ($assetName)..."
        $failed = Get-ReleaseFile -name $assetName -out $exe
        if ($failed -eq 404) { return Write-Failure "release $tag has no installer for Windows ($assetName)." }
        if ($null -ne $failed) { return Write-Failure (Get-DownloadFailure -name $assetName -status $failed) }

        $want = $sums[$assetName]
        $got = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            return Write-Failure "the checksum of $assetName does not match SHA256SUMS (expected $want, got $got). Nothing was installed."
        }
        Write-Host 'Checksum verified.'

        # The silent installer reports success without replacing a pdtd.exe a running PDT keeps open, so a forced
        # install over a running one stops it first, as an update does.
        if ($Force -and ($existing -or $running)) {
            $stopped = Close-RunningPdt
            if ($stopped) { return Write-Failure $stopped }
        }

        Write-Host 'Installing...'
        # Silent (NSIS /S): a per-user install that starts PDT and registers it to start at login.
        # Waits for the installer only: its hook starts pdt-host, which runs on, and `Start-Process -Wait` would wait for
        # that too (it waits for the whole process tree). Reading Handle first keeps ExitCode readable after the exit.
        $setup = Start-Process -FilePath $exe -ArgumentList '/S' -PassThru
        $null = $setup.Handle
        $setup.WaitForExit()
        if ($setup.ExitCode -ne 0) { return Write-Failure "the installer failed (exit code $($setup.ExitCode))." }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    # The installer starts PDT: check that the one answering is the version just installed.
    $wantVersion = $tag.TrimStart('v')
    $answered = $null
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        try {
            $answered = (Invoke-RestMethod -Uri "$local/api/health" -TimeoutSec 2 -UseBasicParsing).version
            if ($answered -eq $wantVersion) { break }
        } catch { $answered = $null }
        Start-Sleep -Seconds 1
    }
    if ($answered -and $answered -ne $wantVersion) {
        return Write-Failure "the installer ran, but the $product answering at $local is still $answered, not $wantVersion. Quit it from its tray icon (Stop PDT and quit), then set `$env:PDT_FORCE = '1' and run this again."
    }

    Write-Host ''
    Write-Host "$product $tag is installed and starts when you log in."
    Write-Host "Open it at $local, then connect an agent: $site/docs/getting-started/connect/"
    if ($tag -like '*-nightly.*') {
        Write-Host 'To keep getting nightly builds, choose Settings > This host > Updates > Update channel > Nightly builds too.'
    }
}

Install-Pdt -Channel $Channel -Version $Version -Force ($Force.IsPresent -or $env:PDT_FORCE -eq '1')
