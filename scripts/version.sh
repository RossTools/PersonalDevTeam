#!/usr/bin/env bash
# PDT's version (docs/SDLC.md "Releases").
#
#   scripts/version.sh            print the version (the workspace version in Cargo.toml)
#   scripts/version.sh check      fail unless Cargo.toml and desktop/src-tauri/tauri.conf.json agree on a X.Y.Z version
#   scripts/version.sh next <patch|minor|major>   print the version after this one (major only from 1.0.0 on)
#   scripts/version.sh set <X.Y.Z>                write a newer version into both files (the release workflow does)
#   scripts/version.sh next-nightly <X.Y.Z> <YYYYMMDD>   print the day's next nightly build of X.Y.Z given the
#                                                 existing tags on stdin: X.Y.Z-nightly.YYYYMMDD, or .2, .3… after it
#   scripts/version.sh prune-nightlies <N>        print the nightly tags on stdin beyond the newest N (oldest first)
#
# The workspace version in Cargo.toml is the source of truth; tauri.conf.json carries the same one (the bundles and
# installer take theirs from it). Both change only in a release PR. A nightly release (PDT-283) sets
# X.Y.Z-nightly.YYYYMMDD in its build's checkout and commits nothing (as the experimental X.Y.Z-exp.N builds before
# it did, PDT-210: still read and ordered, no longer made). Options (before the command): `-C <dir>` works in
# <dir> instead of the repo root. Bash 3.2 compatible (macOS).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
if [ "${1:-}" = "-C" ]; then root="$2"; shift 2; fi

die() { printf 'version: %s\n' "$*" >&2; exit 1; }
usage() { sed -n '4,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
NUM='(0|[1-9][0-9]*)'
is_release() { printf '%s' "$1" | grep -qE "^$NUM\\.$NUM\\.$NUM\$"; }
# A nightly's date and its day's build number (2 and up; the first has none).
DATE='[1-9][0-9]{7}'
BUILD='([2-9]|[1-9][0-9]+)'
NIGHTLY="-nightly\\.$DATE(\\.$BUILD)?"
# A release version, or a nightly build's (X.Y.Z-nightly.YYYYMMDD[.N]), or an experimental build's (X.Y.Z-exp.N).
is_version() { printf '%s' "$1" | grep -qE "^$NUM\\.$NUM\\.$NUM(-exp\\.$NUM|$NIGHTLY)?\$"; }

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
  is_version "$cargo" || die "Cargo.toml's version '$cargo' is not X.Y.Z (or X.Y.Z-nightly.YYYYMMDD in a nightly build)"
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
  is_release "$v" || die "$v is a nightly build's version: bump from a release's"
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

# A version (after any tag prefix) as text that sorts as SemVer does for ours: X.Y.Z-exp.N, then
# X.Y.Z-nightly.YYYYMMDD, .2, .3…, then X.Y.Z.
sort_key() {
  printf '%s' "$1" | sed 's/^[^0-9]*//' | awk '{
    rank = 2; a = 0; b = 0
    if (match($0, /-exp\.[0-9]+$/)) { rank = 0; a = substr($0, RSTART + 5) + 0; $0 = substr($0, 1, RSTART - 1) }
    else if (match($0, /-nightly\.[0-9.]+$/)) {
      rank = 1; n = split(substr($0, RSTART + 9), d, "."); a = d[1] + 0; b = (n > 1 ? d[2] + 0 : 1)
      $0 = substr($0, 1, RSTART - 1) }
    split($0, p, ".")
    printf "%09d%09d%09d%d%09d%09d\n", p[1], p[2], p[3], rank, a, b }'
}

# Is $1 newer than $2?
newer() {
  [ "$(sort_key "$1")" \> "$(sort_key "$2")" ]
}

# The day's next nightly build of release version $1 on date $2 (YYYYMMDD): X.Y.Z-nightly.YYYYMMDD when the tags on
# stdin (after any prefix: `v`, `test-v`) have none of that day yet, else .N with N one more than the highest there
# (the first, with no suffix, counts as 1).
next_nightly() {
  local base="$1" date="$2" pattern
  is_release "$base" || die "'$base' is not X.Y.Z"
  printf '%s' "$date" | grep -qE "^$DATE\$" || die "'$date' is not YYYYMMDD"
  pattern="$(printf '%s' "$base" | sed 's/\./[.]/g')-nightly[.]$date"
  # Passed through the environment: awk -v would read the backslashes in a pattern as escapes.
  PAT="(^|[^0-9.])${pattern}([.][0-9]+)?\$" NAME="$base-nightly.$date" awk '
    $0 ~ ENVIRON["PAT"] { n = 1; if (match($0, /-nightly\.[0-9]+\.[0-9]+$/)) { n = $0; sub(/^.*\./, "", n) }
                         if (n + 0 > max) max = n + 0 }
    END { if (max == 0) print ENVIRON["NAME"]; else printf "%s.%d\n", ENVIRON["NAME"], max + 1 }'
}

# The nightly tags on stdin (any prefix; other lines are ignored) beyond the newest $1 by version, oldest first: the
# ones a nightly release deletes.
prune_nightlies() {
  local keep="$1" sorted count
  printf '%s' "$keep" | grep -qE '^[0-9]+$' || die "'$keep' is not a number"
  sorted="$(while IFS= read -r line; do
      printf '%s' "$line" | sed 's/^[^0-9]*//' | grep -qE "^$NUM\\.$NUM\\.$NUM$NIGHTLY\$" || continue
      printf '%s %s\n' "$(sort_key "$line")" "$line"
    done | sort | awk '{ print $2 }')"
  [ -n "$sorted" ] || return 0
  count="$(printf '%s\n' "$sorted" | wc -l | tr -d ' ')"
  if [ "$count" -gt "$keep" ]; then printf '%s\n' "$sorted" | head -n "$((count - keep))"; fi
}

# Writes the version into Cargo.toml's [workspace.package] and tauri.conf.json (Cargo.lock follows on the next cargo
# command). Only ever forward.
set_version() {
  local new="$1" old tmp
  is_version "$new" || die "'$new' is not X.Y.Z or X.Y.Z-nightly.YYYYMMDD[.N]"
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
  next-nightly) [ $# -eq 3 ] || usage; next_nightly "$2" "$3" ;;
  prune-nightlies) [ $# -eq 2 ] || usage; prune_nightlies "$2" ;;
  set) [ $# -eq 2 ] || usage; set_version "$2" ;;
  *) usage ;;
esac
