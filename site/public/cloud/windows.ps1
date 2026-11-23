# Sets up a Windows Server cloud VM (2022 or 2025, Desktop Experience) as a remote instance of Personal Dev Team (PDT)
# and joins it to a project. Run it once as an administrator (EC2 user data, Azure's Custom Script Extension, or by
# hand in an elevated PowerShell), with the values as parameters or environment variables:
#
#   -TailscaleKey  / TS_AUTHKEY         a Tailscale auth key: pre-approved, not ephemeral, used once (required)
#   -OwnerLogin    / PDT_OWNER_LOGIN    your Tailscale login, the only one allowed to use this PDT (required)
#   -OwnerAddress  / PDT_OWNER_ADDRESS  the owner's address (my-mac.your-tailnet.ts.net) and
#   -PairingCode   / PDT_PAIRING_CODE   the code under the owner's Project Settings > Remote instances > Add instance:
#                                       with both, the instance joins that project at the end
#   -InstanceName  / PDT_INSTANCE_NAME  this instance's name in PDT and on the tailnet (default: the computer name)
#   -Agents        / PDT_AGENTS         the agent CLIs to install: claude, codex (default: both)
#   -GodotVersion  / GODOT_VERSION      a Godot release to install with its export templates (4.4.1)
#   -MsvcBuildTools / PDT_MSVC=1        install the Visual Studio Build Tools' C++ workload. Their licence is Visual
#                                       Studio's: free under the Community terms when you qualify, otherwise it needs a
#                                       Visual Studio subscription. Check before you turn it on.
#
# Two phases. As the administrator it installs git, Node.js, Tailscale, Godot and the Build Tools for every user, makes
# a local account `pdt` that signs in on its own at boot (so PDT runs without anyone at the desktop; the screen is locked
# once PDT is up), and restarts. At that account's first sign-in it joins the tailnet, installs rustup, the agent CLIs
# and PDT, turns on remote access for you and joins the project. Its log is C:\ProgramData\PdtBootstrap\bootstrap.log.
#
# It never stores credentials for the agents: sign in to them once over RDP as `pdt` (the account's password is in
# C:\ProgramData\PdtBootstrap\pdt-password.txt, readable by administrators only), or give the project token secrets for
# remote sessions.

param(
    [string]$TailscaleKey = $env:TS_AUTHKEY,
    [string]$OwnerLogin = $env:PDT_OWNER_LOGIN,
    [string]$OwnerAddress = $env:PDT_OWNER_ADDRESS,
    [string]$PairingCode = $env:PDT_PAIRING_CODE,
    [string]$InstanceName = $env:PDT_INSTANCE_NAME,
    [string[]]$Agents = @(),
    [string]$GodotVersion = $env:GODOT_VERSION,
    [switch]$MsvcBuildTools,
    [ValidateSet('system', 'user')]
    [string]$Phase = 'system'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$site = 'https://pdt.dev'
$port = 7420
$account = 'pdt'
$stateDir = Join-Path $env:ProgramData 'PdtBootstrap'
$settingsFile = Join-Path $stateDir 'settings.json'
$tools = 'C:\Tools'

function Write-Step([string]$text) {
    $line = '{0:u} {1}' -f (Get-Date), $text
    Write-Output $line
    Add-Content -Path (Join-Path $stateDir 'bootstrap.log') -Value $line
}

# Downloads $url to a file in the temp folder and returns its path.
function Get-Download([string]$url, [string]$name) {
    $path = Join-Path $env:TEMP $name
    Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
    return $path
}

# Runs an installer and waits; a nonzero exit code (but 3010, restart needed) fails.
function Invoke-Installer([string]$file, [string[]]$arguments) {
    $p = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru
    if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) { throw "$file exited with $($p.ExitCode)" }
}

function Add-MachinePath([string]$dir) {
    $path = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    if (($path -split ';') -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', "$path;$dir", 'Machine')
    }
}

# A random password of 24 letters and digits, as a SecureString, and its text (kept for the administrator).
function Get-RandomPassword {
    $chars = 'abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
    $bytes = New-Object byte[] 24
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $text = -join ($bytes | ForEach-Object { $chars[$_ % $chars.Length] })
    # Letters and digits of each kind, for Windows' complexity rules.
    $text = "Aa1$text"
    $secure = New-Object Security.SecureString
    foreach ($c in $text.ToCharArray()) { $secure.AppendChar($c) }
    return @{ Text = $text; Secure = $secure }
}

# $given: the script's parameters, passed in (a function reading them from the script's scope hides their use).
function Install-SystemPart([hashtable]$given) {
    $TailscaleKey = $given.TailscaleKey
    $OwnerLogin = $given.OwnerLogin
    $OwnerAddress = $given.OwnerAddress
    $PairingCode = $given.PairingCode
    $InstanceName = $given.InstanceName
    $Agents = $given.Agents
    $GodotVersion = $given.GodotVersion
    $MsvcBuildTools = $given.MsvcBuildTools
    if (-not $TailscaleKey) { throw 'Set TS_AUTHKEY (or -TailscaleKey) to a Tailscale auth key.' }
    if (-not $OwnerLogin) { throw 'Set PDT_OWNER_LOGIN (or -OwnerLogin) to your Tailscale login.' }
    $agentList = $Agents
    if (-not $agentList) { $agentList = if ($env:PDT_AGENTS) { $env:PDT_AGENTS -split '[ ,]+' } else { @('claude', 'codex') } }
    $name = if ($InstanceName) { $InstanceName } else { $env:COMPUTERNAME.ToLower() }

    Write-Step 'Git'
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/git-for-windows/git/releases/latest' -UseBasicParsing
        $asset = $release.assets | Where-Object { $_.name -match '^Git-[\d.]+-64-bit\.exe$' } | Select-Object -First 1
        $file = Get-Download $asset.browser_download_url 'git-setup.exe'
        Invoke-Installer $file @('/VERYSILENT', '/NORESTART', '/NOCANCEL', '/SP-')
    }

    Write-Step 'Node.js'
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
        $lts = (Invoke-RestMethod -Uri 'https://nodejs.org/dist/index.json' -UseBasicParsing | Where-Object { $_.lts } | Select-Object -First 1).version
        $file = Get-Download "https://nodejs.org/dist/$lts/node-$lts-x64.msi" 'node.msi'
        Invoke-Installer 'msiexec.exe' @('/i', $file, '/qn', '/norestart')
    }

    Write-Step 'Tailscale'
    if (-not (Test-Path 'C:\Program Files\Tailscale\tailscale.exe')) {
        $file = Get-Download 'https://pkgs.tailscale.com/stable/tailscale-setup-latest-amd64.msi' 'tailscale.msi'
        Invoke-Installer 'msiexec.exe' @('/i', $file, '/qn', '/norestart', 'TS_UNATTENDEDMODE=always')
    }

    if ($GodotVersion) {
        Write-Step "Godot $GodotVersion"
        $dir = Join-Path $tools "Godot-$GodotVersion"
        if (-not (Test-Path (Join-Path $dir 'godot.exe'))) {
            $base = "https://github.com/godotengine/godot/releases/download/$GodotVersion-stable"
            $zip = Get-Download "$base/Godot_v$GodotVersion-stable_win64.exe.zip" 'godot.zip'
            Expand-Archive -Path $zip -DestinationPath $dir -Force
            Copy-Item (Join-Path $dir "Godot_v$GodotVersion-stable_win64.exe") (Join-Path $dir 'godot.exe')
            Copy-Item (Join-Path $dir "Godot_v$GodotVersion-stable_win64_console.exe") (Join-Path $dir 'godot_console.exe') -ErrorAction SilentlyContinue
        }
        Add-MachinePath $dir
    }

    if ($MsvcBuildTools -or $env:PDT_MSVC -eq '1') {
        Write-Step 'Visual Studio Build Tools (C++)'
        $file = Get-Download 'https://aka.ms/vs/17/release/vs_BuildTools.exe' 'vs_BuildTools.exe'
        Invoke-Installer $file @('--quiet', '--wait', '--norestart', '--nocache', '--add', 'Microsoft.VisualStudio.Workload.VCTools', '--includeRecommended')
    }

    Write-Step "The $account account"
    if (-not (Get-LocalUser -Name $account -ErrorAction SilentlyContinue)) {
        $password = Get-RandomPassword
        $null = New-LocalUser -Name $account -Password $password.Secure -PasswordNeverExpires -AccountNeverExpires -Description 'Runs Personal Dev Team'
        Add-LocalGroupMember -Group 'Remote Desktop Users' -Member $account
        # Its phase writes the log here.
        $null = icacls $stateDir /grant "${account}:(OI)(CI)M"
        $passwordFile = Join-Path $stateDir 'pdt-password.txt'
        Set-Content -Path $passwordFile -Value $password.Text
        # Administrators and SYSTEM only.
        $null = icacls $passwordFile /inheritance:r /grant:r 'Administrators:F' 'SYSTEM:F'
        # It signs in at boot, so PDT (which starts at its sign-in) runs with nobody at the desktop. Winlogon keeps the
        # password in plain text (local users and images of the VM can read it); the guide says so.
        $winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
        Set-ItemProperty -Path $winlogon -Name 'AutoAdminLogon' -Value '1'
        Set-ItemProperty -Path $winlogon -Name 'DefaultUserName' -Value $account
        Set-ItemProperty -Path $winlogon -Name 'DefaultDomainName' -Value $env:COMPUTERNAME
        Set-ItemProperty -Path $winlogon -Name 'DefaultPassword' -Value $password.Text
    }

    # What the account's phase needs, readable by it and administrators only; it is deleted once used.
    $settings = @{ TailscaleKey = $TailscaleKey; OwnerLogin = $OwnerLogin; OwnerAddress = $OwnerAddress; PairingCode = $PairingCode; Name = $name; Agents = $agentList; GodotVersion = $GodotVersion }
    $settings | ConvertTo-Json | Set-Content -Path $settingsFile
    $null = icacls $settingsFile /inheritance:r /grant:r 'Administrators:F' 'SYSTEM:F' "${account}:M"

    Write-Step "The account's part, at its sign-in"
    $script = Join-Path $stateDir 'windows.ps1'
    Copy-Item -Path $PSCommandPath -Destination $script -Force
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$script`" -Phase user"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $account
    $principal = New-ScheduledTaskPrincipal -UserId $account -LogonType Interactive -RunLevel Limited
    $null = Register-ScheduledTask -TaskName 'PDT bootstrap' -Action $action -Trigger $trigger -Principal $principal -Force

    Write-Step 'Restarting: the account signs in and finishes'
    Restart-Computer -Force
}

function Install-UserPart {
    if (-not (Test-Path $settingsFile)) { return }
    $s = Get-Content -Path $settingsFile -Raw | ConvertFrom-Json
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')

    Write-Step 'Joining the tailnet'
    & 'C:\Program Files\Tailscale\tailscale.exe' up --authkey $s.TailscaleKey --hostname $s.Name --unattended
    if ($LASTEXITCODE -ne 0) { throw "tailscale up exited with $LASTEXITCODE" }

    Write-Step 'Rust'
    if (-not (Test-Path (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'))) {
        $file = Get-Download 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' 'rustup-init.exe'
        Invoke-Installer $file @('-y', '--profile', 'minimal', '--default-host', 'x86_64-pc-windows-msvc')
    }

    Write-Step "Agent CLIs: $($s.Agents -join ', ')"
    $npmDir = Join-Path $env:APPDATA 'npm'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $npmDir) { [Environment]::SetEnvironmentVariable('Path', "$userPath;$npmDir", 'User') }
    foreach ($agent in $s.Agents) {
        $package = switch ($agent) { 'claude' { '@anthropic-ai/claude-code' } 'codex' { '@openai/codex' } default { throw "unknown agent $agent (claude or codex)" } }
        & npm.cmd install -g --silent $package
        if ($LASTEXITCODE -ne 0) { throw "npm couldn't install $package" }
    }

    if ($s.GodotVersion) {
        Write-Step 'Godot export templates'
        $templates = Join-Path $env:APPDATA "Godot\export_templates\$($s.GodotVersion).stable"
        if (-not (Test-Path $templates)) {
            $base = "https://github.com/godotengine/godot/releases/download/$($s.GodotVersion)-stable"
            $tpz = Get-Download "$base/Godot_v$($s.GodotVersion)-stable_export_templates.tpz" 'templates.zip'
            $unpacked = Join-Path $env:TEMP 'godot-templates'
            Expand-Archive -Path $tpz -DestinationPath $unpacked -Force
            $null = New-Item -ItemType Directory -Path $templates -Force
            Move-Item -Path (Join-Path $unpacked 'templates\*') -Destination $templates -Force
        }
    }

    Write-Step 'PDT'
    $installer = Get-Download "$site/install.ps1" 'pdt-install.ps1'
    & $installer
    $local = "http://127.0.0.1:$port"
    $deadline = (Get-Date).AddMinutes(3)
    while ($true) {
        try { $null = Invoke-RestMethod -Uri "$local/api/health" -TimeoutSec 3 -UseBasicParsing; break } catch {
            if ((Get-Date) -gt $deadline) { throw "PDT didn't answer on $local" }
            Start-Sleep -Seconds 2
        }
    }

    Write-Step "Remote access for $($s.OwnerLogin)"
    $body = @{ instanceName = $s.Name; ownerLogin = $s.OwnerLogin; remoteAccess = $true } | ConvertTo-Json
    $null = Invoke-RestMethod -Uri "$local/api/host/settings" -Method Put -ContentType 'application/json' -Body $body -UseBasicParsing
    $deadline = (Get-Date).AddMinutes(2)
    while (-not (Invoke-RestMethod -Uri "$local/api/host/remote/refresh" -Method Post -UseBasicParsing).active) {
        if ((Get-Date) -gt $deadline) { throw "remote access didn't start: is HTTPS on for your tailnet (Tailscale admin console, DNS)?" }
        Start-Sleep -Seconds 2
    }

    if ($s.OwnerAddress -and $s.PairingCode) {
        Write-Step "Joining the project on $($s.OwnerAddress)"
        $body = @{ ownerAddress = $s.OwnerAddress; code = $s.PairingCode } | ConvertTo-Json
        $null = Invoke-RestMethod -Uri "$local/api/remote-projects" -Method Post -ContentType 'application/json' -Body $body -UseBasicParsing
    }

    # Done once: the auth key and pairing code go, and PDT starts at each sign-in on its own from now on.
    Remove-Item -Path $settingsFile -Force
    Unregister-ScheduledTask -TaskName 'PDT bootstrap' -Confirm:$false -ErrorAction SilentlyContinue
    Write-Step 'Done. Sign in to the agents over RDP as pdt, then check Project Settings > Remote instances on the owner.'
    rundll32.exe 'user32.dll,LockWorkStation'
}

$null = New-Item -ItemType Directory -Path $stateDir -Force
try {
    if ($Phase -eq 'user') {
        Install-UserPart
    } else {
        Install-SystemPart @{
            TailscaleKey = $TailscaleKey; OwnerLogin = $OwnerLogin; OwnerAddress = $OwnerAddress; PairingCode = $PairingCode
            InstanceName = $InstanceName; Agents = $Agents; GodotVersion = $GodotVersion; MsvcBuildTools = [bool]$MsvcBuildTools
        }
    }
} catch {
    Write-Step "Failed: $($_.Exception.Message)"
    throw
}
