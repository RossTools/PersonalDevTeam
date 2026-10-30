# Installs Personal Dev Team (PDT) on Windows, without admin rights (Windows PowerShell 5.1 or PowerShell 7):
#
#   irm https://pdt.dev/install.ps1 | iex
#
# `irm | iex` takes no arguments, so the options are environment variables, set first:
#
#   $env:PDT_CHANNEL = 'nightly'   # the newest nightly build instead of the newest stable release
#   $env:PDT_VERSION = '0.3.0'     # an exact release
#   $env:PDT_FORCE = '1'           # install again over an existing install
#   $env:GITHUB_TOKEN = '...'      # a GitHub token, for when the releases need one (a private repository)
#
# or, run as a script block, parameters: & ([scriptblock]::Create((irm https://pdt.dev/install.ps1))) -Channel nightly
#
# It downloads the installer (*_x64-setup.exe) from GitHub, checks it against the release's SHA256SUMS and runs it
# silently; the installer starts PDT and registers it to start at login. macOS and Linux have install.sh.
#
# Everything runs from Install-Pdt, called on the last line: a download cut short runs nothing. It never calls `exit`,
# which would close the PowerShell window `iex` runs in.
#
# For tests only: PDT_INSTALL_API (GitHub's API, https://api.github.com) and PDT_INSTALL_LOCAL (where a running PDT
# answers, http://127.0.0.1:7420).

param(
    [string]$Channel = $env:PDT_CHANNEL,
    [string]$Version = $env:PDT_VERSION,
    [switch]$Force
)

function Install-Pdt {
    param([string]$Channel, [string]$Version, [bool]$Force)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $repo = 'RossTools/PersonalDevTeam'
    $product = 'Personal Dev Team'
    $port = 7420
    $site = 'https://pdt.dev'
    $api = if ($env:PDT_INSTALL_API) { $env:PDT_INSTALL_API } else { 'https://api.github.com' }
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

    # Installed: the installer's folder (a per-user install), or a PDT answering on its port.
    $installDir = Join-Path $env:LOCALAPPDATA $product
    $existing = Test-Path (Join-Path $installDir 'pdt-host.exe')
    $running = $false
    try {
        $health = Invoke-RestMethod -Uri "$local/api/health" -TimeoutSec 3 -UseBasicParsing
        $running = $health.name -eq $product
    } catch { $running = $false }
    if (-not $Force -and ($existing -or $running)) {
        if ($existing) { Write-Host "$product is already installed in $installDir." } else { Write-Host "$product is already running at $local." }
        Write-Host 'It keeps itself up to date: Settings > This host > Updates shows its version and checks for a new one.'
        Write-Host "To install it again anyway, set `$env:PDT_FORCE = '1' and run this again."
        return
    }

    $headers = @{ Accept = 'application/vnd.github+json' }
    if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
    $releases = "https://github.com/$repo/releases"
    $noReleases = if ($env:GITHUB_TOKEN) {
        "could not read $product's releases from GitHub with GITHUB_TOKEN: check that it can read $repo."
    } else {
        "could not read $product's releases from GitHub. If the repository is private, set `$env:GITHUB_TOKEN to a token that can read $repo and run this again."
    }

    Write-Host 'Finding the release...'
    try {
        if ($Version) {
            try {
                $release = Invoke-RestMethod -Uri "$api/repos/$repo/releases/tags/v$Version" -Headers $headers -UseBasicParsing
            } catch { return Write-Failure "there is no $product release v$Version (the releases: $releases)." }
        } elseif ($Channel -eq 'nightly') {
            # The newest release a host on the nightly channel takes: a nightly build or a full release, listed newest first.
            $list = Invoke-RestMethod -Uri "$api/repos/$repo/releases?per_page=30" -Headers $headers -UseBasicParsing
            $release = @($list | Where-Object { $_.tag_name -match '^v\d+\.\d+\.\d+(-nightly\.[0-9.]+)?$' }) | Select-Object -First 1
            if (-not $release) { return Write-Failure "found no nightly build of $product (the releases: $releases)." }
        } else {
            $release = Invoke-RestMethod -Uri "$api/repos/$repo/releases/latest" -Headers $headers -UseBasicParsing
        }
    } catch { return Write-Failure $noReleases }
    $tag = $release.tag_name

    $asset = @($release.assets | Where-Object { $_.name -like '*-setup.exe' }) | Select-Object -First 1
    if (-not $asset) { return Write-Failure "release $tag has no installer for Windows." }
    $sums = @($release.assets | Where-Object { $_.name -eq 'SHA256SUMS' }) | Select-Object -First 1
    if (-not $sums) { return Write-Failure "release $tag has no SHA256SUMS, so the download can't be checked. Nothing was installed." }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("pdt-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $download = @{ Accept = 'application/octet-stream' }
        if ($env:GITHUB_TOKEN) { $download.Authorization = "Bearer $env:GITHUB_TOKEN" }
        $exe = Join-Path $tmp $asset.name
        $sumsFile = Join-Path $tmp 'SHA256SUMS'
        Write-Host "Downloading $product $tag ($($asset.name))..."
        try {
            Invoke-WebRequest -Uri "$api/repos/$repo/releases/assets/$($sums.id)" -Headers $download -OutFile $sumsFile -UseBasicParsing
            Invoke-WebRequest -Uri "$api/repos/$repo/releases/assets/$($asset.id)" -Headers $download -OutFile $exe -UseBasicParsing
        } catch { return Write-Failure "could not download $($asset.name): $($_.Exception.Message)" }

        $want = $null
        foreach ($line in Get-Content $sumsFile) {
            $hash, $name = $line -split '\s+\*?', 2
            if ($name -eq $asset.name) { $want = $hash.ToLowerInvariant(); break }
        }
        if (-not $want) { return Write-Failure "SHA256SUMS of $tag does not list $($asset.name), so it can't be checked. Nothing was installed." }
        $got = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            return Write-Failure "the checksum of $($asset.name) does not match SHA256SUMS (expected $want, got $got). Nothing was installed."
        }
        Write-Host 'Checksum verified.'

        Write-Host 'Installing...'
        # Silent (NSIS /S): a per-user install that starts PDT and registers it to start at login.
        $setup = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
        if ($setup.ExitCode -ne 0) { return Write-Failure "the installer failed (exit code $($setup.ExitCode))." }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host ''
    Write-Host "$product $tag is installed and starts when you log in."
    Write-Host "Open it at $local, then connect an agent: $site/docs/getting-started/connect/"
    if ($tag -like '*-nightly.*') {
        Write-Host 'To keep getting nightly builds, choose Settings > This host > Updates > Update channel > Nightly builds too.'
    }
}

Install-Pdt -Channel $Channel -Version $Version -Force ($Force.IsPresent -or $env:PDT_FORCE -eq '1')
