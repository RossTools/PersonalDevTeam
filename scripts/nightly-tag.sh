#!/usr/bin/env bash
# Picks a nightly release's tag (docs/SDLC.md "Nightly releases"; release.yml's `nightly` job runs it).
#
#   scripts/nightly-tag.sh <sha> <X.Y.Z> <YYYYMMDD>   print skip=, tag=, version= and new= for main's head <sha>
#
# Reads the tags with TAG_PREFIX (default `v`) in GITHUB_REPOSITORY with `gh api`, as GH_TOKEN. By hand (SCHEDULED
# unset or not `true`) it is always the day's next build of X.Y.Z: X.Y.Z-nightly.YYYYMMDD, or .2, .3… after it
# (scripts/version.sh next-nightly), new=true. With SCHEDULED=true (the nightly a scheduler starts, PDT-307), a head
# that is already released (a published nightly or a stable release at <sha>) gives skip=true and nothing else; a
# nightly tag at <sha> whose release never got published (a failed night) is that tag again, new=false; otherwise the
# day's next build. It makes no tag: the workflow does, when new=true. A failed tag lookup fails.
set -euo pipefail

die() { printf 'nightly-tag: %s\n' "$*" >&2; exit 1; }
if [ "$#" -ne 3 ]; then sed -n '4p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; fi
sha="$1" base="$2" date="$3"
[ -n "${GITHUB_REPOSITORY:-}" ] || die "GITHUB_REPOSITORY is not set"
prefix="${TAG_PREFIX:-v}"
version_sh="$(cd "$(dirname "$0")" && pwd)/version.sh"

# What the run says, in its log and its summary.
note() {
  printf '%s\n' "$*" >&2
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then printf '%s\n' "$*" >> "$GITHUB_STEP_SUMMARY"; fi
}

# Every tag with the prefix and the commit it points at (ours are lightweight). Read on its own, so a failed lookup
# stops the script (in a pipe its status would be lost, and GitHub's error body read as tags). No match is an empty
# list, not a 404.
refs="$(gh api --paginate "repos/$GITHUB_REPOSITORY/git/matching-refs/tags/${prefix}" \
  --jq '.[] | "\(.ref | ltrimstr("refs/tags/")) \(.object.sha)"')" ||
  die "couldn't read the tags of $GITHUB_REPOSITORY"
tags="$(printf '%s\n' "$refs" | awk 'NF { print $1 }')"
at_head="$(printf '%s\n' "$refs" | awk -v sha="$sha" '$2 == sha { print $1 }')"

if [ "${SCHEDULED:-}" = true ] && [ -n "$at_head" ]; then
  # Main hasn't changed since a release: nothing to publish, unless it is a nightly whose release never got published
  # (that night failed), which is built again.
  tag=""
  for t in $at_head; do
    case "$t" in *-nightly.*) ;; *) tag=""; break ;; esac
    if gh release view "$t" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then tag=""; break; fi
    tag="$t"
  done
  if [ -z "$tag" ]; then
    note "Main's head $sha is released already ($(printf "%s" "$at_head" | paste -sd " " -)): no nightly tonight."
    echo "skip=true"
    exit 0
  fi
  note "Building $tag again: it points at main's head but has no release."
  printf 'skip=false\ntag=%s\nversion=%s\nnew=false\n' "$tag" "${tag#"$prefix"}"
  exit 0
fi

version="$(printf '%s\n' "$tags" | "$version_sh" next-nightly "$base" "$date")"
printf 'skip=false\ntag=%s\nversion=%s\nnew=true\n' "$prefix$version" "$version"
