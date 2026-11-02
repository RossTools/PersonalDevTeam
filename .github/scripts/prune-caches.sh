#!/usr/bin/env bash
# Keeps the repo's Actions caches to what CI reads (docs/SDLC.md "Caching"): on main, the newest cache of each kind
# (a rust-cache shared key per OS and build, a bundle's per OS and CPU, a pnpm store per OS); everything else is
# deleted, including every cache saved on another branch (PRs and queue runs read only their own and main's).
#
#   .github/scripts/prune-caches.sh            delete (cache.yml and nightly releases run it after saving; needs actions: write)
#   .github/scripts/prune-caches.sh --dry-run  only print what it would delete
#
# Needs gh with GH_TOKEN, and GITHUB_REPOSITORY (owner/name) or a checkout gh can read it from.
set -euo pipefail

dry=false
[ "${1:-}" = --dry-run ] && dry=true
repo="${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

usage() {
  gh api "repos/$repo/actions/cache/usage" \
    --jq '"\(.active_caches_size_in_bytes / 1e9 * 100 | floor / 100) GB in \(.active_caches_count) caches"'
}
echo "Before: $(usage)"

# The kind a cache is, or nothing for one CI never reads:
#   v0-rust-<os>-<build|clippy|lint|minimal>-<OS>-<arch>-<hashes>  ->  rust-<os>-<kind>
#   v0-rust-bundle-<bundles>[-<target>]-<OS>-<arch>-<hashes>        ->  rust-bundle-…-<OS>-<arch> (.github/actions/bundle)
#   pnpm-<OS>-<arch>-<lockfile hash>                                ->  pnpm-<OS>-<arch>
kind() {
  printf '%s\n' "$1" | sed -nE \
    -e 's/^v0-rust-((linux|macos|windows)-(build|clippy|lint|minimal))-.*/rust-\1/p' \
    -e 's/^v0-rust-(bundle-[a-z0-9_-]+-[A-Za-z_]+-[A-Za-z0-9_]+)-[0-9a-f]+-[0-9a-f]+$/rust-\1/p' \
    -e 's/^(pnpm-[A-Za-z]+-[A-Za-z0-9]+)-[0-9a-f]+$/\1/p'
}

seen=" "
deleted=0
# Newest first, so the first of each kind is the one kept.
while IFS=$'\t' read -r id ref key; do
  k=""
  [ "$ref" = refs/heads/main ] && k="$(kind "$key")"
  if [ -n "$k" ] && [ "${seen#* "$k" }" = "$seen" ]; then
    seen="$seen$k "
    echo "keep   $key"
    continue
  fi
  echo "delete $key ($ref)"
  deleted=$((deleted + 1))
  if [ "$dry" = false ]; then gh cache delete "$id" --repo "$repo" >/dev/null; fi
done < <(gh cache list --repo "$repo" --limit 1000 --sort created_at --order desc --json id,ref,key \
  --jq '.[] | [.id, .ref, .key] | @tsv')

if [ "$dry" = true ]; then echo "Would delete $deleted caches."; else echo "Deleted $deleted. After: $(usage)"; fi
