#!/usr/bin/env bash
# Makes the release signing key, once (docs/SDLC.md "Release signing"). Run it on your own machine, not in CI:
#
#   scripts/release-signing-key.sh <backup file> [<owner/repo>]
#
# It generates an ECDSA P-256 key, writes the private key to <backup file> (keep it offline, in a password manager:
# GitHub never shows a secret again, and without the key no later release can be signed for hosts that pin it), stores
# it as the PDT_RELEASE_SIGNING_KEY secret of the `release-signing` environment of <owner/repo> (default: this
# checkout's GitHub repository) with `gh secret set`, and prints the public key to pin as RELEASE_SIGNING_KEY in
# crates/pdt-core/src/brand.rs and in site/public/install.sh and install.ps1. The private key is never printed.
set -euo pipefail

die() { printf 'release-signing-key: %s\n' "$*" >&2; exit 1; }
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then sed -n '4p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; fi
backup="$1"
[ ! -e "$backup" ] || die "$backup already exists; pick a new file"
repo_args=()
if [ "$#" = 2 ]; then repo_args=(--repo "$2"); fi
command -v openssl >/dev/null || die "openssl is needed"
command -v gh >/dev/null || die "the GitHub CLI (gh) is needed"

umask 077
openssl ecparam -name prime256v1 -genkey -noout -out "$backup"
gh secret set PDT_RELEASE_SIGNING_KEY --env release-signing ${repo_args[@]+"${repo_args[@]}"} <"$backup"
pub="$(openssl ec -in "$backup" -pubout -outform DER 2>/dev/null | base64 | tr -d '\n')"
cat <<EOF
The private key is in $backup (back it up offline, then you may delete it here) and in the
PDT_RELEASE_SIGNING_KEY secret of the release-signing environment.

The public key, to pin:
  crates/pdt-core/src/brand.rs:  pub const RELEASE_SIGNING_KEY: Option<&str> = Some("$pub");
  site/public/install.sh:        signing_key="$pub"
  site/public/install.ps1:       \$signingKey = '$pub'
EOF
