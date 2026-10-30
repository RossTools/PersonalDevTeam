#!/usr/bin/env bash
# Tests for scripts/version.sh on sample files, in a scratch folder. Run: scripts/version.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/version.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }
expect_eq() { # name, expected, actual
  if [ "$2" != "$3" ]; then fail "$1"; printf -- '--- expected\n%s\n--- actual\n%s\n' "$2" "$3" >&2; fi
}

# A repo-like folder at the given version (Cargo.toml and tauri.conf.json, with versions elsewhere to leave alone).
setup() {
  dir="$scratch/$1"
  mkdir -p "$dir/desktop/src-tauri"
  printf '[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1.0.0" }\n' \
    "$2" > "$dir/Cargo.toml"
  printf '{\n  "productName": "PDT",\n  "version": "%s",\n  "plugins": {\n    "x": { "version": "9.9.9" }\n  }\n}\n' \
    "$2" > "$dir/desktop/src-tauri/tauri.conf.json"
}

setup next 0.2.3
expect_eq "next patch" 0.2.4 "$("$script" -C "$dir" next patch)"
expect_eq "next minor" 0.3.0 "$("$script" -C "$dir" next minor)"
if "$script" -C "$dir" next major >/dev/null 2>&1; then fail "no major bump before 1.0.0"; fi
if "$script" -C "$dir" next huge >/dev/null 2>&1; then fail "next refuses an unknown kind"; fi
setup one 1.4.2
expect_eq "next major past 1.0" 2.0.0 "$("$script" -C "$dir" next major)"

setup set 0.2.3
"$script" -C "$dir" set 0.10.0 >/dev/null || fail "set a newer version"
expect_eq "set writes Cargo.toml" 0.10.0 "$("$script" -C "$dir")"
"$script" -C "$dir" check >/dev/null || fail "set keeps the two files in step"
grep -q 'serde = { version = "1.0.0" }' "$dir/Cargo.toml" || fail "set leaves dependency versions alone"
grep -q '"x": { "version": "9.9.9" }' "$dir/desktop/src-tauri/tauri.conf.json" || fail "set leaves nested versions alone"
for v in 0.9.9 0.10.0 1.0 v1.0.0 01.0.0; do
  if "$script" -C "$dir" set "$v" >/dev/null 2>&1; then fail "set should refuse $v"; fi
done
expect_eq "a refused set changes nothing" 0.10.0 "$("$script" -C "$dir")"

# --- nightly builds (PDT-283): the day's next -nightly.YYYYMMDD[.N], and setting it in a build's checkout
tags='v0.2.1-nightly.20261029
v0.2.1-nightly.20261030
v0.2.1-nightly.20261030.2
v0.2.1-nightly.20261030.10
v0.2.10-nightly.20261031
v10.2.1-nightly.20261031.40
v0.2.1
v0.2.0-nightly.20261031.70
v0.2.1-exp.4'
expect_eq "next-nightly after the day's highest" 0.2.1-nightly.20261030.11 \
  "$(printf '%s\n' "$tags" | "$script" next-nightly 0.2.1 20261030)"
expect_eq "next-nightly after the day's first" 0.2.1-nightly.20261029.2 \
  "$(printf '%s\n' "$tags" | "$script" next-nightly 0.2.1 20261029)"
expect_eq "next-nightly with none that day" 0.2.1-nightly.20261031 \
  "$(printf '%s\n' "$tags" | "$script" next-nightly 0.2.1 20261031)"
expect_eq "next-nightly with no tags" 0.3.0-nightly.20261031 "$(printf '' | "$script" next-nightly 0.3.0 20261031)"
expect_eq "next-nightly after a test prefix" 0.2.1-nightly.20261031.2 \
  "$(printf 'test-v0.2.1-nightly.20261031\n' | "$script" next-nightly 0.2.1 20261031)"
if printf '' | "$script" next-nightly 0.2.1-exp.1 20261031 >/dev/null 2>&1; then fail "next-nightly takes a release version"; fi
if printf '' | "$script" next-nightly 0.2.1 2026-10-31 >/dev/null 2>&1; then fail "next-nightly takes YYYYMMDD"; fi

setup nightly 0.2.0
"$script" -C "$dir" set 0.2.1-nightly.20261030 >/dev/null || fail "set a nightly version"
expect_eq "set writes the nightly version" 0.2.1-nightly.20261030 "$("$script" -C "$dir")"
"$script" -C "$dir" check >/dev/null || fail "check takes a nightly version"
if "$script" -C "$dir" next patch >/dev/null 2>&1; then fail "no bump from a nightly build"; fi
"$script" -C "$dir" set 0.2.1-nightly.20261030.2 >/dev/null || fail "set the day's second nightly over its first"
"$script" -C "$dir" set 0.2.1-nightly.20261031 >/dev/null || fail "set the next day's over the day's second"
setup nightly-old 0.2.1
for v in 0.2.1-nightly.20261030 0.2.0-nightly.20261031 0.2.2-nightly 0.2.2-nightly.2026103 0.2.2-nightly.20261030.1 \
  0.2.2-nightly.20261030.02 0.2.2-nightly.020261030 0.2.2-rc.1; do
  if "$script" -C "$dir" set "$v" >/dev/null 2>&1; then fail "set should refuse $v over 0.2.1"; fi
done
"$script" -C "$dir" set 0.2.2-nightly.20261030 >/dev/null || fail "set the next version's nightly build"

# An experimental build (PDT-210, before nightlies) is still a version, older than the same version's nightlies.
setup exp 0.2.0
"$script" -C "$dir" set 0.2.1-exp.3 >/dev/null || fail "set an experimental version"
"$script" -C "$dir" check >/dev/null || fail "check takes an experimental version"
"$script" -C "$dir" set 0.2.1-nightly.20261030 >/dev/null || fail "a nightly is newer than an experimental build"
if "$script" -C "$dir" set 0.2.1-exp.9 >/dev/null 2>&1; then fail "an experimental build is older than a nightly"; fi

# --- pruning: the nightly tags beyond the newest N, by version, oldest first; others are never listed
tags='v0.2.1-nightly.20261030.2
v0.2.1
v0.2.2-nightly.20261101
v0.2.1-nightly.20261029
v0.2.1-exp.3
v0.2.1-nightly.20261030
v0.2.1-nightly.20261030.10
v0.2.0'
expect_eq "prune keeps the newest" 'v0.2.1-nightly.20261029
v0.2.1-nightly.20261030' "$(printf '%s\n' "$tags" | "$script" prune-nightlies 3)"
expect_eq "prune with room for all" "" "$(printf '%s\n' "$tags" | "$script" prune-nightlies 14)"
expect_eq "prune with no tags" "" "$(printf '' | "$script" prune-nightlies 14)"
expect_eq "prune to none" 5 "$(printf '%s\n' "$tags" | "$script" prune-nightlies 0 | wc -l | tr -d ' ')"
if printf '' | "$script" prune-nightlies x >/dev/null 2>&1; then fail "prune-nightlies takes a number"; fi

if [ "$failed" = 0 ]; then echo "version tests passed"; else exit 1; fi
