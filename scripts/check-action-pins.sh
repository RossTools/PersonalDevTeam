#!/usr/bin/env bash
# Every third-party action in .github is pinned to a full commit SHA, with its tag in a comment
# (`uses: owner/repo@<40 hex> # v1.2.3`), so a moved tag can't change what CI or a release runs. Dependabot bumps
# the SHA and the comment together (docs/SDLC.md "Dependabot"). GitHub's own `actions/*` and the local `./` actions
# may use tags. Bash 3.2 compatible (macOS).
set -euo pipefail

cd "$(dirname "$0")/.."

bad="$(grep -rnE '^[[:space:]-]*uses:[[:space:]]' .github --include='*.yml' --include='*.yaml' \
  | grep -vE 'uses:[[:space:]]+["'"'"']?(\./|actions/|docker://)' \
  | grep -vE 'uses:[[:space:]]+["'"'"']?[^@[:space:]]+@[0-9a-f]{40}["'"'"']?[[:space:]]+#[[:space:]]*v[0-9]' || true)"
if [ -n "$bad" ]; then
  printf 'Third-party actions must be pinned to a commit SHA with the tag in a comment (uses: owner/repo@<sha> # v1.2.3):\n%s\n' "$bad" >&2
  exit 1
fi
echo "action pins: ok"
