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

if [ "$failed" = 0 ]; then echo "version tests passed"; else exit 1; fi
