#!/bin/sh
# Sets up a Linux cloud VM (Ubuntu 22.04 or later, Debian 12) as a remote instance of Personal Dev Team (PDT) and joins
# it to a project. Run once as root, from cloud-init (https://pdt.dev/cloud/cloud-init.yaml) or by hand:
#
#   curl -fsSL https://pdt.dev/cloud/linux.sh | sudo -E sh
#
# with these set in the environment:
#
#   TS_AUTHKEY         a Tailscale auth key: pre-approved, not ephemeral, used once (required)
#   PDT_OWNER_LOGIN    your Tailscale login, the only one allowed to use this PDT (required)
#   PDT_OWNER_ADDRESS  the owner's address (my-mac.your-tailnet.ts.net) and
#   PDT_PAIRING_CODE   the code under the owner's Project Settings > Remote instances > Add instance: with both, the
#                      instance joins that project at the end
#   PDT_INSTANCE_NAME  this instance's name in PDT and on the tailnet (default: the host name)
#   PDT_USER           the account PDT runs as (default: pdt)
#   PDT_AGENTS         the agent CLIs to install, space-separated: claude codex (default: both)
#   GODOT_VERSION      a Godot release to install with its export templates (4.4.1), or empty for none
#
# It installs git, build tools, Node.js, Tailscale, rustup, the agent CLIs and PDT (a systemd user service that runs
# from boot), and never stores credentials: log in to the agents afterwards over SSH (`sudo -iu pdt`, then `claude`),
# or give the project token secrets for remote sessions. It can run again: each step skips what is already done.

set -eu

site="https://pdt.dev"
port=7420
user="${PDT_USER:-pdt}"
name="${PDT_INSTANCE_NAME:-$(hostname)}"
agents="${PDT_AGENTS:-claude codex}"
godot="${GODOT_VERSION:-}"

say() { printf '==> %s\n' "$*"; }
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

[ "$(id -u)" -eq 0 ] || fail "run it as root (sudo -E sh)"
[ -n "${TS_AUTHKEY:-}" ] || fail "set TS_AUTHKEY to a Tailscale auth key"
[ -n "${PDT_OWNER_LOGIN:-}" ] || fail "set PDT_OWNER_LOGIN to your Tailscale login"
command -v apt-get >/dev/null 2>&1 || fail "this script needs apt (Ubuntu or Debian)"

say "Packages"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q ca-certificates curl git build-essential pkg-config unzip jq
if ! command -v node >/dev/null 2>&1; then
    curl -fsSL https://deb.nodesource.com/setup_lts.x | bash -
    apt-get install -y -q nodejs
fi

say "Tailscale"
if ! command -v tailscale >/dev/null 2>&1; then
    curl -fsSL https://tailscale.com/install.sh | sh
fi
tailscale up --authkey "$TS_AUTHKEY" --hostname "$name"

say "The $user account"
if ! id "$user" >/dev/null 2>&1; then
    useradd --create-home --shell /bin/bash "$user"
fi
# PDT serves itself on the tailnet with `tailscale serve`, which needs the operator role.
tailscale set --operator="$user"
uid="$(id -u "$user")"
home="$(getent passwd "$user" | cut -d: -f6)"
# Its user services run from boot, without a login.
loginctl enable-linger "$user"
systemctl start "user@$uid.service"

# Runs a command as the account, in its systemd user session.
as_user() {
    runuser -u "$user" -- env HOME="$home" XDG_RUNTIME_DIR="/run/user/$uid" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" PATH="$home/.cargo/bin:$home/.local/bin:$PATH" \
        sh -c "$1"
}

say "Agent CLIs: $agents"
for agent in $agents; do
    case "$agent" in
        claude) npm install -g --silent @anthropic-ai/claude-code ;;
        codex) npm install -g --silent @openai/codex ;;
        *) fail "unknown agent $agent (claude or codex)" ;;
    esac
done

say "Rust"
as_user 'command -v rustup >/dev/null 2>&1 || curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal'

if [ -n "$godot" ]; then
    say "Godot $godot"
    dir="/opt/godot-$godot"
    base="https://github.com/godotengine/godot/releases/download/$godot-stable"
    if [ ! -x "$dir/godot" ]; then
        mkdir -p "$dir"
        curl -fsSL -o /tmp/godot.zip "$base/Godot_v$godot-stable_linux.x86_64.zip"
        unzip -oq /tmp/godot.zip -d "$dir"
        mv "$dir/Godot_v$godot-stable_linux.x86_64" "$dir/godot"
        rm -f /tmp/godot.zip
    fi
    ln -sf "$dir/godot" /usr/local/bin/godot
    templates="$home/.local/share/godot/export_templates/$godot.stable"
    if [ ! -d "$templates" ]; then
        curl -fsSL -o /tmp/templates.tpz "$base/Godot_v$godot-stable_export_templates.tpz"
        mkdir -p "$templates"
        unzip -oq /tmp/templates.tpz -d /tmp/godot-templates
        mv /tmp/godot-templates/templates/* "$templates/"
        rm -rf /tmp/godot-templates /tmp/templates.tpz
        chown -R "$user:$user" "$home/.local"
    fi
fi

say "PDT"
as_user "curl -fsSL $site/install.sh | sh -s -- --yes"

local_url="http://127.0.0.1:$port"
tries=0
until curl -fsS "$local_url/api/health" >/dev/null 2>&1; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || fail "PDT didn't answer on $local_url"
    sleep 2
done

say "Remote access for $PDT_OWNER_LOGIN"
body="$(jq -n --arg n "$name" --arg o "$PDT_OWNER_LOGIN" '{instanceName: $n, ownerLogin: $o, remoteAccess: true}')"
curl -fsS -X PUT -H 'content-type: application/json' -d "$body" "$local_url/api/host/settings" >/dev/null
tries=0
until [ "$(curl -fsS -X POST "$local_url/api/host/remote/refresh" | jq -r '.active')" = true ]; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || fail "remote access didn't start: is HTTPS on for your tailnet (Tailscale admin console, DNS)?"
    sleep 2
done

if [ -n "${PDT_OWNER_ADDRESS:-}" ] && [ -n "${PDT_PAIRING_CODE:-}" ]; then
    say "Joining the project on $PDT_OWNER_ADDRESS"
    body="$(jq -n --arg a "$PDT_OWNER_ADDRESS" --arg c "$PDT_PAIRING_CODE" '{ownerAddress: $a, code: $c}')"
    curl -fsS -X POST -H 'content-type: application/json' -d "$body" "$local_url/api/remote-projects" >/dev/null
fi

say "Done. Log in to the agents over SSH (sudo -iu $user), then check Project Settings > Remote instances on the owner."
