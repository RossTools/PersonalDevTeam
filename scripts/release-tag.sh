#!/usr/bin/env bash
# Tags a release's commit (docs/SDLC.md "Releases"; release.yml's `tag` job runs it).
#
#   scripts/release-tag.sh <tag> <sha>   make refs/tags/<tag> at <sha> on GitHub, unless it is already there
#
# No such tag: it is made (with `gh api`, as GH_TOKEN, in GITHUB_REPOSITORY). The tag already at <sha>: a re-run, and
# nothing changes. The tag at another commit: it fails, as it does when the lookup itself fails (auth, network).
# Whether the tag exists comes from `git ls-remote`'s exit status against `origin` (0 found, 2 none, else an error),
# never from what a command prints: `gh api` prints GitHub's 404 body on stdout (PDT-219).
set -euo pipefail

die() { printf 'release-tag: %s\n' "$*" >&2; exit 1; }
if [ "$#" -ne 2 ]; then sed -n '4p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; fi
tag="$1" sha="$2"
[ -n "${GITHUB_REPOSITORY:-}" ] || die "GITHUB_REPOSITORY is not set"

status=0
out="$(git ls-remote --exit-code --tags origin "refs/tags/$tag")" || status=$?
case "$status" in
  0)
    existing="$(printf '%s\n' "$out" | awk -v ref="refs/tags/$tag" '$2 == ref { print $1; exit }')"
    if [ "$existing" = "$sha" ]; then
      echo "$tag already points at $sha (a re-run)"
      exit 0
    fi
    echo "::error title=The tag exists::$tag already points at ${existing:-another commit}, not $sha." >&2
    exit 1 ;;
  2) ;;
  *)
    echo "::error title=No tag lookup::Couldn't read the tags of origin (git ls-remote exited $status), so $tag was not made." >&2
    exit 1 ;;
esac

if ! gh api "repos/$GITHUB_REPOSITORY/git/refs" -f ref="refs/tags/$tag" -f sha="$sha" >/dev/null; then
  echo "::error title=Tagging failed::Couldn't make $tag at $sha." >&2
  exit 1
fi
echo "Tagged $tag at $sha"
