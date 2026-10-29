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

# --- experimental builds (PDT-210): the next -exp.N, and setting it in a build's checkout
tags='v0.2.1-exp.1
v0.2.1-exp.9
v0.2.1-exp.10
v10.2.1-exp.40
v0.2.10-exp.50
v0.2.1
v0.2.0-exp.70'
expect_eq "next-exp after the highest" 0.2.1-exp.11 "$(printf '%s\n' "$tags" | "$script" next-exp 0.2.1)"
expect_eq "next-exp with none yet" 0.3.0-exp.1 "$(printf '%s\n' "$tags" | "$script" next-exp 0.3.0)"
expect_eq "next-exp with no tags" 0.2.1-exp.1 "$(printf '' | "$script" next-exp 0.2.1)"
if printf '' | "$script" next-exp 0.2.1-exp.1 >/dev/null 2>&1; then fail "next-exp takes a release version"; fi

setup exp 0.2.0
"$script" -C "$dir" set 0.2.1-exp.3 >/dev/null || fail "set an experimental version"
expect_eq "set writes the experimental version" 0.2.1-exp.3 "$("$script" -C "$dir")"
"$script" -C "$dir" check >/dev/null || fail "check takes an experimental version"
if "$script" -C "$dir" next patch >/dev/null 2>&1; then fail "no bump from an experimental build"; fi
setup exp-old 0.2.1
for v in 0.2.1-exp.1 0.2.0-exp.9 0.2.1-exp 0.2.1-exp.01 0.2.1-rc.1; do
  if "$script" -C "$dir" set "$v" >/dev/null 2>&1; then fail "set should refuse $v over 0.2.1"; fi
done
"$script" -C "$dir" set 0.2.2-exp.1 >/dev/null || fail "set the next version's experimental build"

if [ "$failed" = 0 ]; then echo "version tests passed"; else exit 1; fi
