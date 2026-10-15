#!/usr/bin/env bash
# Builds the Personal Dev Team app bundle and installs it into ~/Applications as the *stable* app.
# Run this yourself. Agents must never run it without --build-only: it stops the stable pdtd that hosts them.
# Running sessions become 'interrupted' and can be restarted from the UI (agents resume).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
port=7420
product="Personal Dev Team"
build_only=false
[[ "${1:-}" == "--build-only" ]] && build_only=true

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
# Any process from the installed bundle other than pdtd means the window is still open.
if pgrep -fl "$dest/Contents/MacOS/" | grep -v "/Contents/MacOS/pdtd" | grep -q .; then
  echo "Close the stable desktop window first (its sessions keep running in pdtd)." >&2
  exit 1
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
open "$dest"
