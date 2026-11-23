#!/usr/bin/env bash
# Puts the newest stable release's user docs in the working tree, for the website's build (pages.yml runs it before
# `pnpm --filter site build`; site/README.md "Which docs the site publishes").
#
#   scripts/site-release-docs.sh   docs/user and docs/agent-guide.md as of the newest vX.Y.Z tag
#
# The site's Download buttons and install scripts install the newest stable release, so its docs describe that release,
# not main. Pages added since the release are removed, changed ones go back to the release's text. The Contributing
# section stays at the checkout's, with the developer guide it sits beside: contributors work on main. Nightly and
# other pre-release tags never count. With no stable tag (a fork, a shallow clone) it changes nothing. Prints the tag.
set -euo pipefail

tag="$(git tag --list 'v*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -t. -k1.2,1n -k2,2n -k3,3n | tail -n 1 || true)"
if [ -z "$tag" ]; then
  echo "::warning title=No stable release::No vX.Y.Z tag here, so the site publishes the checkout's user docs." >&2
  exit 0
fi
git restore --source="$tag" --worktree -- docs/user docs/agent-guide.md ':(exclude)docs/user/contributing'
echo "$tag"
