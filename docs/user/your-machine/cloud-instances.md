# Cloud instances

Run a remote instance on a virtual machine at AWS or Azure that PDT starts when work needs it and stops when it's
idle, so you pay only for the hours it works.

A cloud instance is a [remote instance](remote-projects.md) like any other: a PDT on another machine that works on one
of your projects. The difference is that the owner can start and stop its machine:

- **Start on demand.** When auto-start places a ticket on a cloud instance whose machine is stopped, the owner starts
  it. The ticket waits in To do and the board says **starting win-cloud…**. Once the instance's PDT answers (about a
  minute for Linux, up to three for Windows), the ticket starts there.
- **Idle stop.** Once nothing has run on it for a while (no sessions, runs or tickets waiting for it; 30 minutes unless
  you change it), the owner stops the machine. On Azure it is deallocated, so its compute isn't billed.
- **A cost meter**: the hours it ran this month and what they cost at the hourly rate you set.

## Costs

Rough prices for a Windows machine with 4 vCPUs and 16 GB, a 128 GB disk and its IP address, in US dollars (on-demand
list prices in late 2026; check your provider's current prices before you choose):

| Machine | About 65 hours a month (3 hours on weekdays) | Always on |
| --- | --- | --- |
| AWS m6i.xlarge, Windows Server | $35 | $288 |
| Azure D4as_v5, Windows Server, Standard SSD | $36 | $283 |
| AWS m6i.xlarge, Linux | $23 | $154 |
| Hetzner CCX23, Linux (always on only) | | €73–86 |

- The Windows licence is about $0.046 per vCPU-hour of that, which is why a Windows machine is cheapest stopped when
  idle.
- When it's stopped you still pay for its disk (about $0.08 a GB-month) and, on Azure, a static IP. On part-time use
  these are most of the bill.
- Coming into the machine (crates, npm, git fetches) is free; going out, past 100 GB a month, about $0.09 a GB.
- Spot machines aren't supported: they can be taken back with 30 seconds' to 2 minutes' notice, mid-session.

## What you need

- **PDT on the owner with remote access on**, and your Tailscale login as its owner. See
  [Remote access and your phone](remote-access.md).
- **A Tailscale auth key** for the machine: in the Tailscale admin console, **Settings › Keys › Generate auth key**,
  with **Reusable** off, **Ephemeral** off and **Pre-approved** on, and no tags. Then turn off **key expiry** for the
  machine once it has joined, so it stays on the tailnet. The instance has to sign in to Tailscale as you: the owner
  takes requests only from your Tailscale login.
- **The AWS or Azure plugin** on the owner, in **Settings › Plugins**, with a connection holding the provider's
  credentials (below). Give those credentials only the rights PDT uses.

## Set up a Windows cloud instance on AWS

1. **Make an access key for PDT.** In IAM, add a user with only this policy (your region, account and instance id;
   add the instance after step 2), then make an access key for it:

   ```json
   {
     "Version": "2012-10-17",
     "Statement": [
       {
         "Effect": "Allow",
         "Action": ["ec2:StartInstances", "ec2:StopInstances"],
         "Resource": "arn:aws:ec2:us-east-1:123456789012:instance/i-0123456789abcdef0"
       },
       { "Effect": "Allow", "Action": "ec2:DescribeInstances", "Resource": "*" }
     ]
   }
   ```

2. **Launch the machine.** In EC2, **Launch instance**: **Microsoft Windows Server 2022 Base** (with Desktop
   Experience), **m6i.xlarge**, a 128 GB **gp3** disk, and a security group with no inbound rules (you reach it over
   Tailscale). Under **Advanced details › User data**, paste this, with your values:

   ```powershell
   <powershell>
   $env:TS_AUTHKEY = 'tskey-auth-...'
   $env:PDT_OWNER_LOGIN = 'you@example.com'
   $env:PDT_INSTANCE_NAME = 'win-cloud'
   $env:GODOT_VERSION = '4.4.1'   # leave empty for no Godot
   # $env:PDT_MSVC = '1'          # the C++ Build Tools: check their licence first (below)
   Invoke-WebRequest https://pdt.dev/cloud/windows.ps1 -OutFile "$env:TEMP\pdt-bootstrap.ps1" -UseBasicParsing
   & "$env:TEMP\pdt-bootstrap.ps1"
   </powershell>
   ```

3. **Wait for it to set itself up**: 15 to 30 minutes, longer with the Build Tools. It installs git, Node.js,
   Tailscale, Godot and its export templates, the Build Tools if asked, rustup, Claude Code and Codex, and PDT; makes a
   local account `pdt` that PDT runs as; restarts; joins your tailnet as `win-cloud`; and turns on remote access with
   you as its owner. Its log is `C:\ProgramData\PdtBootstrap\bootstrap.log`.
4. **Join the project.** On the owner, open the project's settings, **Remote instances**, **Add instance**. Then open
   the new instance's PDT from your computer (`https://win-cloud.your-tailnet.ts.net`), click **Add project**, choose
   **Join a remote**, and enter the owner's address and the code. (The code lasts 10 minutes, so make it once the
   machine is ready. To join while it sets up, add `$env:PDT_OWNER_ADDRESS` and `$env:PDT_PAIRING_CODE` to the user
   data, if it will be ready in time.)
5. **Log in to the agents.** Connect to `win-cloud` over Remote Desktop through Tailscale as `pdt`. Its password is in
   `C:\ProgramData\PdtBootstrap\pdt-password.txt`, which only administrators can read. Run `claude` and `codex` once
   each to sign in. See [Agent logins](#agent-logins).
6. **Let the owner start and stop it.** On the owner, in **Settings › Plugins**, turn on **AWS** and add a connection
   with the access key: its ID, the region and the secret access key. Then in **Remote instances**, click
   **Cloud VM…** on `win-cloud`, pick the connection, enter the instance id, and set:
   - **Stop after idle minutes**: 30 is a good start; 0 never stops it.
   - **Hourly rate (US$)**: what it costs an hour while it runs, for the meter (0.376 for m6i.xlarge with Windows in
     us-east-1).
7. **Send it work**: add a placement rule (label `windows` → `win-cloud`) in **Remote instances**. See
   [Choose where tickets run](remote-projects.md#choose-where-tickets-run).

The machine is stopped once it has been idle for its minutes. The next `windows` ticket starts it again.

## Set up a Windows cloud instance on Azure

1. **Make a service principal for PDT**: an app registration with a client secret. Give it a custom role on the
   machine's resource group with only these actions:

   ```json
   {
     "Name": "PDT cloud instance",
     "Actions": [
       "Microsoft.Compute/virtualMachines/read",
       "Microsoft.Compute/virtualMachines/instanceView/read",
       "Microsoft.Compute/virtualMachines/start/action",
       "Microsoft.Compute/virtualMachines/deallocate/action"
     ],
     "AssignableScopes": ["/subscriptions/<subscription id>/resourceGroups/pdt"]
   }
   ```

2. **Make the machine**: **Windows Server 2022 Datacenter** (not Server Core), **D4as_v5**, a 128 GB **Standard SSD**,
   no inbound ports. A new virtual network may have no outbound internet access by default: give the machine a way
   out (a Standard public IP address is the cheapest, about $3.65 a month), or setup, Tailscale and builds fail.
3. **Run the setup script** with the Custom Script Extension, with your values (the protected settings aren't shown
   afterwards):

   ```sh
   az vm extension set -g pdt --vm-name win-cloud --publisher Microsoft.Compute --name CustomScriptExtension \
     --protected-settings '{"fileUris": ["https://pdt.dev/cloud/windows.ps1"], "commandToExecute":
       "powershell -ExecutionPolicy Bypass -File windows.ps1 -TailscaleKey tskey-auth-... -OwnerLogin you@example.com -InstanceName win-cloud -GodotVersion 4.4.1"}'
   ```

   The machine restarts partway, so the extension may report a failure even when setup goes on: the log in
   `C:\ProgramData\PdtBootstrap\bootstrap.log` says how far it got.
4. Join the project and log in to the agents as in steps 4 and 5 for AWS.
5. On the owner, turn on **Azure** in **Settings › Plugins** and add a connection with the tenant, client and
   subscription IDs and the client secret. In **Remote instances**, click **Cloud VM…** on `win-cloud` and enter the
   machine as `resource-group/vm-name` (`pdt/win-cloud`), with its idle minutes and hourly rate (0.356 for D4as_v5).

## A Linux cloud instance

For Ubuntu 22.04 or later (or Debian 12), give the machine
[this cloud-init file](https://pdt.dev/cloud/cloud-init.yaml) as its user data, with your values filled in. It sets up
a `pdt` account running PDT from boot, with git, build tools, Node.js, Tailscale, rustup, Claude Code, Codex and
optionally Godot; its log is `/var/log/pdt-bootstrap.log`. Log in to the agents over SSH through Tailscale
(`sudo -iu pdt`, then `claude`).

Linux machines start quickly and cost less, so stopping them saves less. A Linux machine that is always on is cheapest
at Hetzner, which bills stopped machines anyway: don't set it up as a cloud instance, just as a remote instance.

## Cloud instances in Remote instances

Each cloud instance's row shows its machine's state (**VM running**, **VM starting**, **VM stopped**), the hours it
ran this month and what they cost, and when it stops. **Start** and **Stop** start or stop it now; one you start
stops again once it has been idle for its minutes. **Cloud VM…** changes its settings; **Not a cloud VM** there makes
it a plain remote instance again and leaves the machine as it is.

If the provider refuses (the credentials are wrong or lack a right, the machine was deleted), or a machine started 10
minutes ago still hasn't come online, the row says why, and tickets placed on it wait as for any instance that's
offline: "waiting for win-cloud (offline)".

The meter counts the time PDT saw the machine running, at the rate you set. It's an estimate: the disk, the IP address
and traffic aren't in it, and your provider's bill is the real figure.

## A backstop at the provider

If the owner is off when a machine goes idle, it can't stop it. Set a backstop at the provider too:

- **Azure**: a daily auto-shutdown, which deallocates the machine:
  `az vm auto-shutdown -g pdt -n win-cloud --time 2300` (the time is UTC).
- **AWS**: a CloudWatch alarm that stops the machine after an hour of low CPU:

  ```sh
  aws cloudwatch put-metric-alarm --alarm-name win-cloud-idle --namespace AWS/EC2 --metric-name CPUUtilization \
    --dimensions Name=InstanceId,Value=i-0123456789abcdef0 --statistic Average --period 300 --evaluation-periods 12 \
    --threshold 3 --comparison-operator LessThanThreshold --alarm-actions arn:aws:automate:us-east-1:ec2:stop
  ```

## Agent logins

An instance whose agents aren't signed in says so in **Remote instances**: "claude needs a login on win-cloud". Sign
in once on the machine, over Remote Desktop (Windows) or SSH (Linux) through Tailscale, as the account PDT runs as.
The login stays on the machine's disk across stops and starts.

Or give the project the agents' tokens as secrets for its remote sessions: see
[Secrets on a remote instance](remote-projects.md#secrets-on-a-remote-instance).

Never put credentials in a machine image you copy or share.

## Who can reach it

The machine has no open ports: you and the owner reach it over Tailscale only, and its PDT takes requests only from
your Tailscale login. On a tailnet only you use, that's all. If you share your tailnet, limit who reaches the machine
in its access controls (the address is the machine's Tailscale IP):

```json
{
  "hosts": { "win-cloud": "100.101.102.103" },
  "grants": [
    { "src": ["you@example.com"], "dst": ["win-cloud"], "ip": ["tcp:443", "tcp:22", "tcp:3389"] },
    { "src": ["win-cloud"], "dst": ["you@example.com"], "ip": ["tcp:443"] }
  ]
}
```

PDT is on port 443 (HTTPS on the tailnet); 22 and 3389 are for SSH and Remote Desktop.

## The Visual Studio Build Tools

The Windows script installs the C++ Build Tools only when you ask (`PDT_MSVC=1` or `-MsvcBuildTools`). They are free
to use under the Visual Studio Community licence if you qualify for it (individuals, open source, small teams);
otherwise you need a Visual Studio subscription. Check the licence before you turn them on.
