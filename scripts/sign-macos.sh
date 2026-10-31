#!/usr/bin/env bash
# Signs a built macOS app bundle with PDT's release certificate: an Apple Developer ID Application certificate, or
# (until one is set up) a self-signed code-signing certificate.
#
# Why: the macOS keychain lets a program read an item without asking only when it passes both the item's access rule
# (the program's designated requirement) and its partition list. A Developer ID signature carries a Team ID, and the
# partition list trusts the team (`teamid:<id>`), so every release reads the update token without a prompt once it
# has been allowed. A self-signed certificate has no Team ID: the partition list then names each build's exact code
# hash (`cdhash:<hash>`), so every update asks again (docs/SDLC.md "Signing"). An ad-hoc signature
# fails the access rule as well.
#
# usage: scripts/sign-macos.sh <path/to/App.app>
# The certificate comes from either
#   MACOS_SIGNING_P12 (base64 .p12) + MACOS_SIGNING_P12_PASSWORD: imported into a temporary keychain (CI), or
#   PDT_SIGNING_IDENTITY: the SHA-1 hash of a code-signing identity already in your keychain (local builds).
# A Developer ID signature gets the hardened runtime and a secure timestamp (both required for notarization;
# scripts/notarize-macos.sh). Make a self-signed certificate with scripts/make-signing-cert.sh.
set -euo pipefail

app="${1:?usage: $0 <path/to/App.app>}"
[[ -d "$app/Contents/MacOS" ]] || { echo "not an app bundle: $app" >&2; exit 2; }

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")"
main_exe="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist")"

keychain=""
saved_keychains=()
cleanup() {
  if [[ -n "$keychain" ]]; then
    if ((${#saved_keychains[@]})); then security list-keychains -d user -s "${saved_keychains[@]}"; fi
    security delete-keychain "$keychain" || true
  fi
}
trap cleanup EXIT

if [[ -n "${MACOS_SIGNING_P12:-}" ]]; then
  : "${MACOS_SIGNING_P12_PASSWORD:?MACOS_SIGNING_P12_PASSWORD is required with MACOS_SIGNING_P12}"
  tmp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
  keychain="$tmp/pdt-signing-$$.keychain-db"
  kc_password="$(uuidgen)"
  p12="$tmp/pdt-signing-$$.p12"
  printf '%s' "$MACOS_SIGNING_P12" | base64 --decode > "$p12"
  security create-keychain -p "$kc_password" "$keychain"
  security set-keychain-settings -lut 3600 "$keychain"
  security unlock-keychain -p "$kc_password" "$keychain"
  security import "$p12" -k "$keychain" -P "$MACOS_SIGNING_P12_PASSWORD" -T /usr/bin/codesign >/dev/null
  rm -f "$p12"
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$kc_password" "$keychain" >/dev/null
  # codesign finds identities only in keychains on the search list.
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"; line="${line#\"}"; line="${line%\"}"
    [[ -n "$line" ]] && saved_keychains+=("$line")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s "$keychain" "${saved_keychains[@]}"
  # A self-signed certificate is "not trusted", so codesign takes it only by hash, never by name.
  identity="$(security find-identity -p codesigning "$keychain" | awk '/^ *1\)/ { print $2; exit }')"
  [[ -n "$identity" ]] || { echo "the .p12 holds no code-signing identity" >&2; exit 1; }
  identity_name="$(security find-identity -p codesigning "$keychain" | awk -F'"' '/^ *1\)/ { print $2; exit }')"
elif [[ -n "${PDT_SIGNING_IDENTITY:-}" ]]; then
  identity="$PDT_SIGNING_IDENTITY"
  identity_name="$(security find-identity -p codesigning | awk -F'"' -v h="$identity" '$0 ~ h { print $2; exit }')"
else
  echo "no signing certificate: set MACOS_SIGNING_P12 (+ _PASSWORD) or PDT_SIGNING_IDENTITY" >&2
  exit 3
fi

if [[ "${identity_name:-}" == "Developer ID Application:"* ]]; then
  echo "Signing with $identity_name (hardened runtime, secure timestamp)"
  sign() { codesign --force --options runtime --timestamp --sign "$identity" "$@"; }
else
  echo "Signing with ${identity_name:-$identity} (self-signed: the keychain asks again after every update)"
  sign() { codesign --force --timestamp=none --sign "$identity" "$@"; }
fi
# Inner executables first (the bundle's seal covers them), each with an identifier that never changes between
# builds; the linker's default (`<name>-<hash>`) changes with every build.
for exe in "$app/Contents/MacOS/"*; do
  name="$(basename "$exe")"
  [[ "$name" == "$main_exe" ]] && continue
  sign --identifier "$bundle_id.$name" "$exe"
done
sign --identifier "$bundle_id" "$app"

codesign --verify --strict --deep --verbose=2 "$app"
for exe in "$app/Contents/MacOS/"*; do
  codesign -d -r- "$exe" 2>&1 | sed -n 's/^designated => /  /p' | sed "s|^|$(basename "$exe"):|"
done
echo "Signed $app"
