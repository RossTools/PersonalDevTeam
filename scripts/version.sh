#!/usr/bin/env bash
# PDT's version (docs/SDLC.md "Releases").
#
#   scripts/version.sh            print the version (the workspace version in Cargo.toml)
#   scripts/version.sh check      fail unless Cargo.toml and desktop/src-tauri/tauri.conf.json agree on a X.Y.Z version
#   scripts/version.sh next <patch|minor|major>   print the version after this one (major only from 1.0.0 on)
#   scripts/version.sh set <X.Y.Z>                write a newer version into both files (the release workflow does)
#
# The workspace version in Cargo.toml is the source of truth; tauri.conf.json carries the same one (the bundles and
# installer take theirs from it). Both change only in a release PR. Options (before the command): `-C <dir>` works in
# <dir> instead of the repo root. Bash 3.2 compatible (macOS).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "-C" ]; then root="$2"; shift 2; fi

die() { printf 'version: %s\n' "$*" >&2; exit 1; }
usage() { sed -n '4,7p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
is_version() { printf '%s' "$1" | grep -qE '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; }

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

# The version after the checked-in one. Before 1.0.0 a major bump is refused: 0.x releases bump minor for breaking
# changes, and 1.0.0 is set as an exact version.
next() {
  local v major minor patch
  check >/dev/null
  v="$(cargo_version)"
  major="${v%%.*}"; minor="${v#*.}"; minor="${minor%%.*}"; patch="${v##*.}"
  case "$1" in
    patch) echo "$major.$minor.$((patch + 1))" ;;
    minor) echo "$major.$((minor + 1)).0" ;;
    major)
      [ "$major" -ge 1 ] || die "no major bump before 1.0.0: bump minor, or give 1.0.0 as the exact version"
      echo "$((major + 1)).0.0" ;;
    *) die "'$1' is not patch, minor or major" ;;
  esac
}

# Is $1 newer than $2 (both X.Y.Z)?
newer() {
  local a b
  a="$(printf '%s' "$1" | awk -F. '{ printf "%09d%09d%09d", $1, $2, $3 }')"
  b="$(printf '%s' "$2" | awk -F. '{ printf "%09d%09d%09d", $1, $2, $3 }')"
  [ "$a" \> "$b" ]
}

# Writes the version into Cargo.toml's [workspace.package] and tauri.conf.json (Cargo.lock follows on the next cargo
# command). Only ever forward.
set_version() {
  local new="$1" old tmp
  is_version "$new" || die "'$new' is not X.Y.Z"
  check >/dev/null
  old="$(cargo_version)"
  newer "$new" "$old" || die "$new is not newer than $old"
  tmp="$root/Cargo.toml.tmp"
  NEW="$new" awk '/^\[/ { section = $0 }
       section == "[workspace.package]" && !done && /^version[[:space:]]*=/ {
         print "version = \"" ENVIRON["NEW"] "\""; done = 1; next }
       { print }' "$root/Cargo.toml" > "$tmp"
  mv "$tmp" "$root/Cargo.toml"
  tmp="$root/desktop/src-tauri/tauri.conf.json.tmp"
  NEW="$new" awk '!done && /^  "version":/ { sub(/"version":[[:space:]]*"[^"]*"/, "\"version\": \"" ENVIRON["NEW"] "\""); done = 1 }
       { print }' "$root/desktop/src-tauri/tauri.conf.json" > "$tmp"
  mv "$tmp" "$root/desktop/src-tauri/tauri.conf.json"
  check
}

case "${1:-}" in
  "") v="$(cargo_version)"; [ -n "$v" ] || die "no version in Cargo.toml"; echo "$v" ;;
  check) check ;;
  next) [ $# -eq 2 ] || usage; next "$2" ;;
  set) [ $# -eq 2 ] || usage; set_version "$2" ;;
  *) usage ;;
esac
