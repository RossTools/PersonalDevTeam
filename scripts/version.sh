#!/usr/bin/env bash
# PDT's version (docs/SDLC.md "Releases").
#
#   scripts/version.sh            print the version (the workspace version in Cargo.toml)
#   scripts/version.sh check      fail unless Cargo.toml and desktop/src-tauri/tauri.conf.json agree on a X.Y.Z version
#   scripts/version.sh next <patch|minor|major>   print the version after this one (major only from 1.0.0 on)
#   scripts/version.sh set <X.Y.Z>                write a newer version into both files (the release workflow does)
#   scripts/version.sh next-exp <X.Y.Z>           print the next experimental build of X.Y.Z, X.Y.Z-exp.N, given the
#                                                 existing tags on stdin (N: one more than the highest there)
#
# The workspace version in Cargo.toml is the source of truth; tauri.conf.json carries the same one (the bundles and
# installer take theirs from it). Both change only in a release PR. An experimental release (PDT-210) sets
# X.Y.Z-exp.N in its build's checkout and commits nothing. Options (before the command): `-C <dir>` works in
# <dir> instead of the repo root. Bash 3.2 compatible (macOS).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "-C" ]; then root="$2"; shift 2; fi

die() { printf 'version: %s\n' "$*" >&2; exit 1; }
usage() { sed -n '4,9p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
NUM='(0|[1-9][0-9]*)'
is_release() { printf '%s' "$1" | grep -qE "^$NUM\\.$NUM\\.$NUM\$"; }
# A release version, or an experimental build's (X.Y.Z-exp.N).
is_version() { printf '%s' "$1" | grep -qE "^$NUM\\.$NUM\\.$NUM(-exp\\.$NUM)?\$"; }

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
  is_version "$cargo" || die "Cargo.toml's version '$cargo' is not X.Y.Z (or X.Y.Z-exp.N in an experimental build)"
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
  is_release "$v" || die "$v is an experimental build's version: bump from a release's"
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

# A version as text that sorts as SemVer does for ours: X.Y.Z-exp.N before X.Y.Z.
sort_key() {
  printf '%s' "$1" | awk '{
    pre = 999999999
    if (match($0, /-exp\.[0-9]+$/)) { pre = substr($0, RSTART + 5) + 0; $0 = substr($0, 1, RSTART - 1) }
    split($0, p, ".")
    printf "%09d%09d%09d%09d", p[1], p[2], p[3], pre }'
}

# Is $1 newer than $2?
newer() {
  [ "$(sort_key "$1")" \> "$(sort_key "$2")" ]
}

# The next experimental build of release version $1: X.Y.Z-exp.N, N one more than the highest of the tags on stdin
# that are X.Y.Z-exp.<n> after any prefix (`v`, `test-v`), else 1.
next_exp() {
  local base="$1" pattern
  is_release "$base" || die "'$base' is not X.Y.Z"
  pattern="$(printf '%s' "$base" | sed 's/\./[.]/g')"
  # Passed through the environment: awk -v would read the backslashes in a pattern as escapes.
  PAT="(^|[^0-9.])${pattern}-exp[.][0-9]+\$" BASE="$base" awk '
    $0 ~ ENVIRON["PAT"] { n = $0; sub(/^.*-exp\./, "", n); if (n + 0 > max) max = n + 0 }
    END { printf "%s-exp.%d\n", ENVIRON["BASE"], max + 1 }'
}

# Writes the version into Cargo.toml's [workspace.package] and tauri.conf.json (Cargo.lock follows on the next cargo
# command). Only ever forward.
set_version() {
  local new="$1" old tmp
  is_version "$new" || die "'$new' is not X.Y.Z or X.Y.Z-exp.N"
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
  next-exp) [ $# -eq 2 ] || usage; next_exp "$2" ;;
  set) [ $# -eq 2 ] || usage; set_version "$2" ;;
  *) usage ;;
esac
