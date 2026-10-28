#!/usr/bin/env bash
# PDT's version (docs/SDLC.md "Releases").
#
#   scripts/version.sh            print the version (the workspace version in Cargo.toml)
#   scripts/version.sh check      fail unless Cargo.toml and desktop/src-tauri/tauri.conf.json agree on a X.Y.Z version
#
# The workspace version in Cargo.toml is the source of truth; tauri.conf.json carries the same one (the bundles and
# installer take theirs from it). Both change only in a release PR. Options (before the command): `-C <dir>` works in
# <dir> instead of the repo root. Bash 3.2 compatible (macOS).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "-C" ]; then root="$2"; shift 2; fi

die() { printf 'version: %s\n' "$*" >&2; exit 1; }

# `version = "…"` in Cargo.toml's [workspace.package] section.
cargo_version() {
  awk '/^\[/ { section = $0; next }
       section == "[workspace.package]" && /^version[[:space:]]*=/ {
         sub(/^version[[:space:]]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit }' "$root/Cargo.toml"
}

# The top-level "version" in tauri.conf.json (two spaces of indent, as the file is formatted).
tauri_version() {
  sed -nE 's/^  "version":[[:space:]]*"([^"]*)".*/\1/p' "$root/desktop/src-tauri/tauri.conf.json" | head -n 1
}

check() {
  local cargo tauri
  cargo="$(cargo_version)"
  tauri="$(tauri_version)"
  [ -n "$cargo" ] || die "no version in Cargo.toml's [workspace.package]"
  printf '%s' "$cargo" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' || die "Cargo.toml's version '$cargo' is not X.Y.Z"
  [ "$cargo" = "$tauri" ] ||
    die "Cargo.toml says $cargo but desktop/src-tauri/tauri.conf.json says ${tauri:-nothing}: change both (a release PR does)"
  echo "version $cargo"
}

case "${1:-}" in
  "") v="$(cargo_version)"; [ -n "$v" ] || die "no version in Cargo.toml"; echo "$v" ;;
  check) check ;;
  *) sed -n '3,4p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac
