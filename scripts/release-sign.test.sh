#!/usr/bin/env bash
# Tests for scripts/release-sign.sh with throwaway keys, in a scratch folder. Run: scripts/release-sign.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/release-sign.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }
has() { grep -qF -- "$2" <<<"$1"; }

openssl ecparam -name prime256v1 -genkey -noout -out "$scratch/a.pem"
openssl ecparam -name prime256v1 -genkey -noout -out "$scratch/b.pem"
openssl ec -in "$scratch/a.pem" -pubout -out "$scratch/a.pub" 2>/dev/null
pub_a="$(openssl ec -in "$scratch/a.pem" -pubout -outform DER 2>/dev/null | base64 | tr -d '\n')"
brand() { printf 'pub const RELEASE_SIGNING_KEY: Option<&str> = %s;\n' "$1" >"$scratch/brand.rs"; }
printf '%s  pdt-linux-x86_64.tar.gz\n' "$(printf x | openssl dgst -sha256 -r | cut -d' ' -f1)" >"$scratch/SHA256SUMS"
# Runs the script with key file $1 (or none) and sets $status and $out.
run() {
  rm -f "$scratch/SHA256SUMS.sig"
  local key=""
  if [ -n "$1" ]; then key="$(cat "$1")"; fi
  status=0
  out="$(env -i PATH="$PATH" PDT_BRAND_RS="$scratch/brand.rs" PDT_RELEASE_SIGNING_KEY="$key" "$script" v1.2.3 "$scratch/SHA256SUMS" 2>&1)" || status=$?
}
verifies() {
  { printf '%s\n' "$1"; cat "$scratch/SHA256SUMS"; } >"$scratch/message"
  openssl dgst -sha256 -verify "$scratch/a.pub" -signature "$scratch/SHA256SUMS.sig" "$scratch/message" >/dev/null 2>&1
}

# --- no key pinned, none given: a warning, no signature, the release still goes out
brand None
run ""
[ "$status" = 0 ] || fail "no key: exits 0 (got $status): $out"
has "$out" "::warning title=Release not signed::" || fail "no key: warns, got: $out"
[ ! -e "$scratch/SHA256SUMS.sig" ] || fail "no key: writes no signature"

# --- a key, none pinned: signs "<tag>\n" + SHA256SUMS
run "$scratch/a.pem"
[ "$status" = 0 ] || fail "key, no pin: exits 0 (got $status): $out"
verifies v1.2.3 || fail "key, no pin: the signature verifies over the tag and SHA256SUMS"
verifies v1.2.4 && fail "key, no pin: the signature is bound to the tag"

# --- the pinned key: signs and checks
brand "Some(\"$pub_a\")"
run "$scratch/a.pem"
[ "$status" = 0 ] || fail "pinned key: exits 0 (got $status): $out"
verifies v1.2.3 || fail "pinned key: the signature verifies"

# --- another key than the pinned one: fails, leaves no signature
run "$scratch/b.pem"
[ "$status" != 0 ] || fail "wrong key: fails"
has "$out" "is not the key brand.rs pins" || fail "wrong key: says why, got: $out"
[ ! -e "$scratch/SHA256SUMS.sig" ] || fail "wrong key: leaves no signature"

# --- a pinned key but none given: fails
run ""
[ "$status" != 0 ] || fail "pinned, no key: fails"
has "$out" "hosts would refuse an unsigned release" || fail "pinned, no key: says why, got: $out"

# --- the real brand.rs has the line the script reads
grep -Eq '^pub const RELEASE_SIGNING_KEY: Option<&str> = (None|Some\("[A-Za-z0-9+/=]+"\));$' \
  "$(dirname "$script")/../crates/pdt-core/src/brand.rs" || fail "brand.rs: RELEASE_SIGNING_KEY is None or Some(\"<base64>\") on one line"

if [ "$failed" = 0 ]; then echo "release-sign tests passed"; else exit 1; fi
