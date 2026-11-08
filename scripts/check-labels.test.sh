#!/usr/bin/env bash
# Tests for scripts/check-labels.sh on sample .github folders, in a scratch folder. Run: scripts/check-labels.test.sh
# (CI runs it in the `changelog` job.)
# shellcheck disable=SC2016 # the expected errors quote labels in backticks
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/check-labels.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

# A repo-like folder with the given .github files (path under .github, then content, repeated).
setup() {
  dir="$scratch/$1"; shift
  mkdir -p "$dir/.github/workflows" "$dir/.github/ISSUE_TEMPLATE"
  while [ $# -gt 0 ]; do printf '%s' "$2" > "$dir/.github/$1"; shift 2; done
}

labels=$'- name: windows\n  color: 0078D4\n  description: Run the Windows checks\n- name: no-changelog\n  color: ededed\n  description: No fragment\n- name: bug\n  color: d73a4a\n  description: Broken\n'
workflow=$'      - run: |\n          # Windows: on a PR only with the label, otherwise it has to wait.\n          if has windows || false; then windows=true; fi\n          if printf %s "$LABELS" | grep -q \'"no-changelog"\'; then exit 0; fi\n'

expect_ok() { # name
  if ! out="$("$script" -C "$dir" 2>&1)"; then fail "$1: expected ok"; printf '%s\n' "$out" >&2; fi
}
expect_error() { # name, text the error contains
  if out="$("$script" -C "$dir" 2>&1)"; then fail "$1: expected an error"; return; fi
  case "$out" in *"$2"*) ;; *) fail "$1: expected '$2' in"; printf '%s\n' "$out" >&2 ;; esac
}

# --- every label used is listed (prose like "it has to wait" isn't a label)
setup ok labels.yml "$labels" workflows/ci.yml "$workflow" \
  ISSUE_TEMPLATE/bug.yml $'name: Bug\nlabels: ["bug"]\n' dependabot.yml $'    labels: [no-changelog]\n'
expect_ok "all listed"

# --- a label an issue form or Dependabot uses, missing from the list
setup form labels.yml "$labels" ISSUE_TEMPLATE/feature.yml $'labels: ["enhancement"]\n'
expect_error "issue form" '`enhancement`'
setup dependabot labels.yml "$labels" dependabot.yml $'    labels: [dependencies, no-changelog]\n'
expect_error "dependabot" '`dependencies`'

# --- a label CI tests for, missing from the list
setup ci labels.yml "$labels" workflows/ci.yml $'          if has macos || false; then macos=true; fi\n'
expect_error "ci has" '`macos`'

# --- the list itself: duplicates, a bad color, no description, no file
setup dup labels.yml "$labels"$'- name: bug\n  color: d73a4a\n  description: Again\n'
expect_error "duplicate" 'more than once: bug'
setup color labels.yml $'- name: bug\n  color: "#d73a4a"\n  description: Broken\n'
expect_error "color" 'bug needs a color'
setup desc labels.yml $'- name: bug\n  color: d73a4a\n'
expect_error "description" 'bug needs a description'
setup none workflows/ci.yml "$workflow"
expect_error "no list" 'labels.yml is missing'

[ "$failed" = 0 ] || exit 1
echo "check-labels tests: ok"
