#!/usr/bin/env bash
# Tests for scripts/changelog.sh on sample fragments, in a scratch folder. Run: scripts/changelog.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/changelog.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

# A fresh repo-like folder with a CHANGELOG.md and the given fragments (name, then content, repeated).
setup() {
  dir="$scratch/$1"; shift
  mkdir -p "$dir/.changes/unreleased"
  printf '# Changelog\n\nIntro.\n\n## 0.1.x: continuous builds\n\nOld.\n' > "$dir/CHANGELOG.md"
  while [ $# -gt 0 ]; do printf '%s' "$2" > "$dir/.changes/unreleased/$1"; shift 2; done
}

expect_eq() { # name, expected, actual
  if [ "$2" != "$3" ]; then fail "$1"; printf -- '--- expected\n%s\n--- actual\n%s\n' "$2" "$3" >&2; fi
}

# --- compile: groups in order, internal left out, lists kept, fragments moved
setup compile \
  PDT-1.md $'---\nkind: fixed\n---\nThe board no longer flickers.\n' \
  PDT-2.md $'---\nkind: added\n---\n\nTickets can be starred.\nStars show on the board.\n\n' \
  PDT-2-api.md $'---\nkind: added\n---\n- `GET /api/stars` lists them.\n- Stars sync between hosts.\n' \
  PDT-3.md $'---\nkind: internal\n---\nRefactored the store.\n' \
  PDT-4.md $'---\nkind: breaking\n---\nThe `pdt` CLI is gone; use the UI.\n' \
  PDT-5.md $'---\nkind: changed\n---\nPaths like C:\\new\\table stay as written.\n'
notes="$(CHANGELOG_DATE=2026-10-27 "$script" -C "$dir" 0.2.0)"
expected_notes='### Breaking

- The `pdt` CLI is gone; use the UI.

### Added

- Tickets can be starred.
  Stars show on the board.
- `GET /api/stars` lists them.
- Stars sync between hosts.

### Changed

- Paths like C:\new\table stay as written.

### Fixed

- The board no longer flickers.'
expect_eq "compile prints the notes" "$expected_notes" "$notes"
expect_eq "compile writes CHANGELOG.md" "# Changelog

Intro.

## 0.2.0 (2026-10-27)

$expected_notes

## 0.1.x: continuous builds

Old." "$(cat "$dir/CHANGELOG.md")"
expect_eq "fragments move to .changes/<version>" "PDT-1.md PDT-2-api.md PDT-2.md PDT-3.md PDT-4.md PDT-5.md" \
  "$(cd "$dir/.changes/0.2.0" && echo *)"
expect_eq "unreleased is left empty" "" "$(ls "$dir/.changes/unreleased")"

# --- compile: only internal fragments
setup internal PDT-3.md $'---\nkind: internal\n---\nRefactored the store.\n'
notes="$(CHANGELOG_DATE=2026-10-27 "$script" -C "$dir" 0.2.1)"
expect_eq "internal only" "Internal changes only." "$notes"

# --- compile: ticket order is numeric
setup order PDT-10.md $'---\nkind: fixed\n---\nTen.\n' PDT-9.md $'---\nkind: fixed\n---\nNine.\n'
notes="$(CHANGELOG_DATE=2026-10-27 "$script" -C "$dir" 0.2.2)"
expect_eq "numeric order" $'### Fixed\n\n- Nine.\n- Ten.' "$notes"

# --- compile refuses: no fragments, an existing version folder, a bad fragment (and changes nothing)
setup empty
if "$script" -C "$dir" 0.3.0 >/dev/null 2>&1; then fail "compile with no fragments should fail"; fi
setup exists PDT-1.md $'---\nkind: fixed\n---\nA fix.\n'
mkdir -p "$dir/.changes/0.3.0"
if "$script" -C "$dir" 0.3.0 >/dev/null 2>&1; then fail "compile into an existing version should fail"; fi
setup invalid PDT-1.md $'---\nkind: fixed\n---\nA fix.\n' PDT-2.md $'---\nkind: nope\n---\nText.\n'
before="$(cat "$dir/CHANGELOG.md")"
if "$script" -C "$dir" 0.3.0 >/dev/null 2>&1; then fail "compile with a bad fragment should fail"; fi
expect_eq "a refused compile leaves CHANGELOG.md alone" "$before" "$(cat "$dir/CHANGELOG.md")"
[ -f "$dir/.changes/unreleased/PDT-1.md" ] || fail "a refused compile leaves the fragments in place"

# --- check
setup check \
  PDT-1.md $'---\nkind: fixed\n---\nA fix.\n' \
  PDT-2.md $'---\nkind: nope\n---\nText.\n' \
  PDT-3.md $'kind: fixed\nNo front matter.\n' \
  PDT-4.md $'---\nkind: added\n---\n\n' \
  notes.md $'---\nkind: added\n---\nText.\n' \
  PDT-5.txt $'---\nkind: added\n---\nText.\n'
u="$dir/.changes/unreleased"
"$script" check "$u/PDT-1.md" 2>/dev/null || fail "check accepts a good fragment"
for f in PDT-2.md PDT-3.md PDT-4.md notes.md PDT-5.txt missing.md; do
  if "$script" check "$u/$f" 2>/dev/null; then fail "check should refuse $f"; fi
done
err="$("$script" check "$u/PDT-2.md" 2>&1 || true)"
case "$err" in *"kind 'nope' is not one of"*) ;; *) fail "check names the bad kind: $err" ;; esac
if "$script" -C "$dir" check 2>/dev/null; then fail "check with no arguments checks every unreleased fragment"; fi
setup good PDT-1.md $'---\nkind: fixed\n---\nA fix.\n'
"$script" -C "$dir" check || fail "check with no arguments passes good fragments"

if [ "$failed" = 0 ]; then echo "changelog tests passed"; else exit 1; fi
