#!/usr/bin/env bash
# Builds the Personal Dev Team app bundle and installs it into ~/Applications as the *stable* app.
# Run this yourself. Agents must never run it without --build-only: it stops the stable pdtd that hosts them.
# Running sessions become 'interrupted' and can be restarted from the UI (agents resume).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
port=7420
product="Personal Dev Team"
usage() { echo "usage: $0 [--build-only]" >&2; exit 2; }
build_only=false
case $# in
  0) ;;
  1) [[ "$1" == "--build-only" ]] || usage; build_only=true ;;
  *) usage ;;
esac

up() { curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$port/api/health"; }

# The version the build reports (pdt_core::BUILD_VERSION): PDT_BUILD_VERSION, else the workspace version + -local.
workspace_version="$(awk -F'"' '/^version = /{print $2; exit}' "$root/Cargo.toml")"
build_version="${PDT_BUILD_VERSION:-$workspace_version-local}"

cd "$root"
pnpm install --frozen-lockfile
pnpm --dir web build
cargo build --release -p pdtd -p pdt-host
node scripts/stage-sidecars.mjs
pnpm --dir desktop exec tauri build --config "$root/desktop/src-tauri/tauri.bundle.conf.json" --bundles app

bundle="$root/target/release/bundle/macos/$product.app"
[[ -d "$bundle" ]] || { echo "no app bundle was produced at $bundle" >&2; exit 1; }
# Signed like a release when PDT_SIGNING_IDENTITY (or MACOS_SIGNING_P12) is set; otherwise the keychain asks once
# more for access to the update token after this build is installed (scripts/sign-macos.sh).
if [[ -n "${PDT_SIGNING_IDENTITY:-}${MACOS_SIGNING_P12:-}" ]]; then
  scripts/sign-macos.sh "$bundle"
else
  echo "warning: not signed with the release certificate; allow keychain access when asked" >&2
fi
echo "Built $bundle"
$build_only && exit 0

data_dir="$HOME/Library/Application Support/pdt"
# -f: an older pdtd without the update API answers 404, which is not a hand-over.
status="$(curl -fs --max-time 5 "http://127.0.0.1:$port/api/host/update" || true)"
if [[ -n "$status" && "$status" != *'"state":"disabled"'* ]]; then
  # pdt-host supervises the stable pdtd: hand the bundle over like a GitHub release (health check, rollback).
  drop="$data_dir/update/downloads/local"
  mkdir -p "$drop"
  (cd "$(dirname "$bundle")" && tar -czf "$drop/$product.app.tar.gz" "$product.app")
  now="$(($(date +%s) * 1000))"
  from_version="$(printf '%s' "$status" | sed -n 's/.*"current":"\([^"]*\)".*/\1/p')"
  # Written whole, then renamed: pdt-host acts as soon as request.json appears.
  # pdt-host's health check expects exactly the version the new build reports.
  printf '{"version":"%s","tag":"local","asset":"%s","fromVersion":"%s","requestedAt":%s}\n' \
    "$build_version" "$drop/$product.app.tar.gz" "${from_version:-unknown}" "$now" > "$data_dir/update/request.json.tmp"
  mv -f "$data_dir/update/request.json.tmp" "$data_dir/update/request.json"
  # pdt-host also stops pdtd once it sees the request, so pdtd may already be gone.
  curl -fsS --max-time 10 -X POST "http://127.0.0.1:$port/api/shutdown" >/dev/null || true
  echo "Handed the bundle to pdt-host; see $data_dir/logs/pdt-host.log and $data_dir/update/result.json."
  exit 0
fi

dest="$HOME/Applications/$product.app"
# A registered pdt-host that cannot take the update (KeepAlive) would start pdtd again: stop and unregister it. The
# new app registers it again when it starts, copying the new pdt-host.
agent="$HOME/Library/LaunchAgents/dev.personaldevteam.host.plist"
launchctl bootout "gui/$(id -u)/dev.personaldevteam.host" 2>/dev/null || true
rm -f "$agent"
# The stable tray app (any process from the installed bundle other than pdtd) holds no state: stop it so the bundle
# can be replaced. pdtd is stopped gracefully below. No pipelines here: under pipefail a SIGPIPE would skip this.
procs="$(pgrep -fl "$dest/Contents/MacOS/" || true)"
tray_pids=""
while read -r pid cmd; do
  if [[ -n "$pid" && "$cmd" != *"/Contents/MacOS/pdtd"* ]]; then tray_pids="$tray_pids $pid"; fi
done <<<"$procs"
if [[ -n "$tray_pids" ]]; then
  echo "Stopping the stable tray app..."
  # shellcheck disable=SC2086 # word splitting is intended: one argument per pid
  kill $tray_pids 2>/dev/null || true
  for _ in $(seq 1 40); do
    # shellcheck disable=SC2086
    kill -0 $tray_pids 2>/dev/null || break
    sleep 0.25
  done
fi

if up; then
  echo "Stopping the stable pdtd on port $port..."
  curl -fsS -X POST "http://127.0.0.1:$port/api/shutdown" >/dev/null
  for _ in $(seq 1 80); do up || break; sleep 0.25; done
  if up; then echo "pdtd did not stop within 20 seconds" >&2; exit 1; fi
fi

mkdir -p "$HOME/Applications"
rm -rf "$dest"
cp -R "$bundle" "$dest"
echo "Installed to $dest. Starting it (it registers and starts pdt-host, which starts pdtd)."
# The stable app must not inherit a dev setup (PDT_PROFILE=dev etc.); open passes our env to the app.
for v in $(compgen -e); do
  if [[ "$v" == PDT_* ]]; then unset "$v"; fi
done
open "$dest"
