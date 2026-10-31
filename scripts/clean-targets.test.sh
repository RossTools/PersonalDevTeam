#!/usr/bin/env bash
# Tests for scripts/clean-targets.sh on a made-up target folder, in a scratch folder. Run: scripts/clean-targets.test.sh
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/clean-targets.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }
old() { touch -t 202001010000 "$@"; }
expect_gone() { [ ! -e "$t/$1" ] || fail "$1 should be removed"; }
expect_kept() { [ -e "$t/$1" ] || fail "$1 should be kept"; }

# A target folder with a stale unit (serde, built long ago), a fresh one (pdtd), an old and a new incremental session,
# and the copied-up binaries of each.
t="$scratch/target"
stale_hash=0123456789abcdef
fresh_hash=fedcba9876543210
setup() {
  rm -rf "$t"
  mkdir -p "$t/debug/.fingerprint/serde-$stale_hash" "$t/debug/.fingerprint/pdtd-$fresh_hash" \
    "$t/debug/deps/serde-$stale_hash.dSYM" "$t/debug/build/serde-$stale_hash/out" \
    "$t/debug/incremental/serde-1abc/s-old" "$t/debug/incremental/pdtd-2def/s-new"
  touch "$t/CACHEDIR.TAG" "$t/debug/.fingerprint/serde-$stale_hash/lib-serde" \
    "$t/debug/.fingerprint/pdtd-$fresh_hash/bin-pdtd" "$t/debug/deps/libserde-$stale_hash.rlib" \
    "$t/debug/deps/serde-$stale_hash.d" "$t/debug/deps/serde-$stale_hash" "$t/debug/deps/pdtd-$fresh_hash" \
    "$t/debug/build/serde-$stale_hash/out/x" "$t/debug/incremental/serde-1abc/s-old/a" \
    "$t/debug/incremental/pdtd-2def/s-new/a" "$t/debug/old-tool" "$t/debug/pdtd"
  old "$t/debug/.fingerprint/serde-$stale_hash/lib-serde" "$t/debug/.fingerprint/serde-$stale_hash" \
    "$t/debug/incremental/serde-1abc/s-old/a" "$t/debug/incremental/serde-1abc/s-old" "$t/debug/incremental/serde-1abc" \
    "$t/debug/old-tool"
}

# --- dry run: lists what it would remove, removes nothing
setup
out="$("$script" --dry-run "$t")"
grep -q "would remove $t/debug/deps/libserde-$stale_hash.rlib" <<<"$out" || fail "dry run lists the stale rlib"
grep -q "pdtd" <<<"$out" && fail "dry run lists nothing fresh"
expect_kept "debug/deps/libserde-$stale_hash.rlib"

# --- a run removes the stale unit's outputs, the old session and the old copied-up binary, and keeps the rest
setup
"$script" "$t" >/dev/null
for f in ".fingerprint/serde-$stale_hash" "deps/libserde-$stale_hash.rlib" "deps/serde-$stale_hash.d" \
  "deps/serde-$stale_hash" "deps/serde-$stale_hash.dSYM" "build/serde-$stale_hash" "incremental/serde-1abc" old-tool; do
  expect_gone "debug/$f"
done
for f in ".fingerprint/pdtd-$fresh_hash" "deps/pdtd-$fresh_hash" "incremental/pdtd-2def" pdtd; do
  expect_kept "debug/$f"
done

# --- --days 0 removes everything cargo built
setup
"$script" --days 0 "$t" >/dev/null
expect_gone "debug/.fingerprint/pdtd-$fresh_hash"
expect_gone "debug/deps/pdtd-$fresh_hash"
expect_kept CACHEDIR.TAG

# --- refuses a folder cargo didn't make
mkdir -p "$scratch/not-a-target"
touch "$scratch/not-a-target/keep"
if "$script" "$scratch/not-a-target" 2>/dev/null; then fail "a folder without CACHEDIR.TAG is refused"; fi
[ -e "$scratch/not-a-target/keep" ] || fail "nothing is removed from a folder it refuses"

[ "$failed" -eq 0 ] && echo "clean-targets: all tests passed"
exit "$failed"
