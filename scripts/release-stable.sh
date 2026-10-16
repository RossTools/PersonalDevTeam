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

cd "$root"
pnpm install --frozen-lockfile
pnpm --dir web build
cargo build --release -p pdtd
node scripts/stage-pdtd.mjs
pnpm --dir desktop exec tauri build --config "$root/desktop/src-tauri/tauri.bundle.conf.json" --bundles app

bundle="$root/target/release/bundle/macos/$product.app"
[[ -d "$bundle" ]] || { echo "no app bundle was produced at $bundle" >&2; exit 1; }
echo "Built $bundle"
$build_only && exit 0

dest="$HOME/Applications/$product.app"
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
echo "Installed to $dest. Starting it (it starts pdtd)."
# The stable app must not inherit a dev setup (PDT_PROFILE=dev etc.); open passes our env to the app.
for v in $(compgen -e); do
  if [[ "$v" == PDT_* ]]; then unset "$v"; fi
done
open "$dest"
