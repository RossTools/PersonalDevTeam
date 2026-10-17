#!/usr/bin/env bash
# Notarizes a Developer ID-signed app bundle with Apple and staples the ticket, so Gatekeeper opens it without the
# "unidentified developer" warning. Run after scripts/sign-macos.sh.
#
# usage: scripts/notarize-macos.sh <path/to/App.app>
# Needs an App Store Connect API key (docs/superpowers/notes/developer-id.md):
#   MACOS_NOTARY_KEY (base64 of the AuthKey_<id>.p8 file), MACOS_NOTARY_KEY_ID, MACOS_NOTARY_ISSUER.
set -euo pipefail

app="${1:?usage: $0 <path/to/App.app>}"
[[ -d "$app/Contents/MacOS" ]] || { echo "not an app bundle: $app" >&2; exit 2; }
: "${MACOS_NOTARY_KEY:?MACOS_NOTARY_KEY is required}"
: "${MACOS_NOTARY_KEY_ID:?MACOS_NOTARY_KEY_ID is required}"
: "${MACOS_NOTARY_ISSUER:?MACOS_NOTARY_ISSUER is required}"

work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/pdt-notary.XXXXXX")"
trap 'rm -rf "$work"' EXIT
key="$work/AuthKey_$MACOS_NOTARY_KEY_ID.p8"
printf '%s' "$MACOS_NOTARY_KEY" | base64 --decode > "$key"
# notarytool takes a zip; ditto keeps the bundle's symlinks and metadata.
ditto -c -k --keepParent "$app" "$work/app.zip"
if ! xcrun notarytool submit "$work/app.zip" --key "$key" --key-id "$MACOS_NOTARY_KEY_ID" \
  --issuer "$MACOS_NOTARY_ISSUER" --wait --timeout 30m --output-format json > "$work/result.json"; then
  cat "$work/result.json" >&2 || true
  exit 1
fi
cat "$work/result.json"
id="$(/usr/bin/plutil -extract id raw "$work/result.json")"
status="$(/usr/bin/plutil -extract status raw "$work/result.json")"
if [[ "$status" != "Accepted" ]]; then
  xcrun notarytool log "$id" --key "$key" --key-id "$MACOS_NOTARY_KEY_ID" --issuer "$MACOS_NOTARY_ISSUER" >&2 || true
  echo "notarization $id: $status" >&2
  exit 1
fi
xcrun stapler staple "$app"
xcrun stapler validate "$app"
echo "Notarized $app"
