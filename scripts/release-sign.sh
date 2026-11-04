#!/usr/bin/env bash
# Signs a release's SHA256SUMS (docs/SDLC.md "Release signing"; release.yml's `publish` job runs it).
#
#   scripts/release-sign.sh <tag> <SHA256SUMS>   write <SHA256SUMS>.sig, the signature hosts and the install scripts check
#
# The key is PDT_RELEASE_SIGNING_KEY: a PEM ECDSA P-256 private key (scripts/release-signing-key.sh makes it). The
# signature is `openssl dgst -sha256 -sign` (DER) over "<tag>\n" followed by SHA256SUMS, so it vouches for this
# release only. With a key pinned in crates/pdt-core/src/brand.rs (RELEASE_SIGNING_KEY), the new signature is checked
# against it, and a missing key fails: hosts would refuse the release. With none pinned, a missing key only warns and
# the release goes out with SHA256SUMS alone. For tests: PDT_BRAND_RS (the brand.rs to read).
set -euo pipefail

die() { printf 'release-sign: %s\n' "$*" >&2; exit 1; }
if [ "$#" -ne 2 ]; then sed -n '4p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; fi
tag="$1" sums="$2"
[ -f "$sums" ] || die "$sums does not exist"
brand="${PDT_BRAND_RS:-$(cd "$(dirname "$0")/.." && pwd)/crates/pdt-core/src/brand.rs}"
pinned="$(sed -n 's/^pub const RELEASE_SIGNING_KEY: Option<&str> = Some("\([A-Za-z0-9+/=]*\)");.*/\1/p' "$brand")"
grep -q '^pub const RELEASE_SIGNING_KEY: Option<&str> = ' "$brand" || die "$brand has no RELEASE_SIGNING_KEY"

if [ -z "${PDT_RELEASE_SIGNING_KEY:-}" ]; then
  [ -z "$pinned" ] || die "PDT_RELEASE_SIGNING_KEY is empty, but brand.rs pins a release key: hosts would refuse an unsigned release"
  echo "::warning title=Release not signed::PDT_RELEASE_SIGNING_KEY is empty: $tag ships with SHA256SUMS alone"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "**Not signed**: \`PDT_RELEASE_SIGNING_KEY\` is empty, so $tag has no \`SHA256SUMS.sig\`." >>"$GITHUB_STEP_SUMMARY"
  fi
  exit 0
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
umask 077
printf '%s\n' "$PDT_RELEASE_SIGNING_KEY" >"$scratch/key.pem"
{ printf '%s\n' "$tag"; cat "$sums"; } >"$scratch/message"
openssl dgst -sha256 -sign "$scratch/key.pem" -out "$sums.sig" "$scratch/message" || die "openssl could not sign with PDT_RELEASE_SIGNING_KEY"

if [ -n "$pinned" ]; then
  { echo '-----BEGIN PUBLIC KEY-----'; printf '%s\n' "$pinned" | fold -w 64; echo '-----END PUBLIC KEY-----'; } >"$scratch/pinned.pem"
  if ! openssl dgst -sha256 -verify "$scratch/pinned.pem" -signature "$sums.sig" "$scratch/message" >/dev/null; then
    rm -f "$sums.sig"
    die "PDT_RELEASE_SIGNING_KEY is not the key brand.rs pins (RELEASE_SIGNING_KEY): hosts would refuse the release"
  fi
fi
echo "Signed SHA256SUMS of $tag"
