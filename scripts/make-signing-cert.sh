#!/usr/bin/env bash
# Makes PDT's self-signed macOS code-signing certificate, once. Every release is signed with it
# (scripts/sign-macos.sh), so the keychain keeps trusting pdtd across updates. It does not satisfy Gatekeeper.
#
# usage: scripts/make-signing-cert.sh <output dir>
# Writes <output dir>/pdt-signing.p12 and pdt-signing.password, and prints how to store them as the GitHub secrets
# MACOS_SIGNING_P12 and MACOS_SIGNING_P12_PASSWORD. Keep the .p12 safe (e.g. a password manager): a new certificate
# means every host must allow access to the update token once more. Delete the output dir when done.
set -euo pipefail

out="${1:?usage: $0 <output dir>}"
mkdir -p "$out"
chmod 700 "$out"
[[ -e "$out/pdt-signing.p12" ]] && { echo "$out/pdt-signing.p12 already exists" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/cert.cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = Personal Dev Team release signing
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
openssl req -x509 -newkey rsa:3072 -nodes -keyout "$work/key.pem" -out "$work/cert.pem" -days 7300 \
  -config "$work/cert.cnf" 2>/dev/null
password="$(openssl rand -base64 24)"
# -legacy: macOS's `security import` cannot read OpenSSL 3's default .p12 encryption. LibreSSL has no such flag.
legacy=()
openssl version | grep -q '^OpenSSL 3' && legacy=(-legacy)
openssl pkcs12 -export "${legacy[@]}" -inkey "$work/key.pem" -in "$work/cert.pem" \
  -name "Personal Dev Team release signing" -out "$out/pdt-signing.p12" -passout "pass:$password"
printf '%s' "$password" > "$out/pdt-signing.password"
chmod 600 "$out/pdt-signing.p12" "$out/pdt-signing.password"

echo "Certificate SHA-1: $(openssl x509 -in "$work/cert.pem" -noout -fingerprint -sha1 | cut -d= -f2 | tr -d :)"
echo "Store it in GitHub (repository secrets), then delete $out once the .p12 is backed up:"
echo "  base64 -i '$out/pdt-signing.p12' | gh secret set MACOS_SIGNING_P12"
echo "  gh secret set MACOS_SIGNING_P12_PASSWORD < '$out/pdt-signing.password'"
