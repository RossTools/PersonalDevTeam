#!/usr/bin/env bash
# The repository's labels are listed in .github/labels.yml (docs/SDLC.md "Labels"). This checks the list (each label
# once, with a 6-digit hex `color` and a `description`) and that every label .github names is in it: Dependabot's
# and the issue forms' `labels:`, and the labels CI reads (`if has <label>`, `|| has <label>` and `grep -q '"<label>"'` in workflows).
#   scripts/check-labels.sh [-C <dir>]          offline, what CI runs
#   scripts/check-labels.sh [-C <dir>] --live   also compares the list with GitHub's labels (`gh label list`)
# Bash 3.2 compatible (macOS).
set -euo pipefail

dir="$(cd "$(dirname "$0")/.." && pwd)"
live=false
while [ $# -gt 0 ]; do
  case "$1" in
    -C) dir="$2"; shift 2 ;;
    --live) live=true; shift ;;
    *) echo "usage: $0 [-C <dir>] [--live]" >&2; exit 2 ;;
  esac
done
cd "$dir"

list=.github/labels.yml
[ -f "$list" ] || { echo "$list is missing" >&2; exit 1; }

# The list as tab-separated `name color description` lines.
listed="$(awk '
  function out() { if (name != "") printf "%s\t%s\t%s\n", name, color, desc }
  function val(s) { sub(/^[^:]*:[[:space:]]*/, "", s); sub(/[[:space:]]+$/, "", s); return s }
  /^- name:/ { out(); name = val($0); color = ""; desc = ""; next }
  /^[[:space:]]+color:/ { color = val($0); next }
  /^[[:space:]]+description:/ { desc = val($0); next }
  END { out() }
' "$list")"

failed=0
fail() { printf '%s\n' "$*" >&2; failed=1; }

dups="$(printf '%s\n' "$listed" | cut -f1 | sort | uniq -d)"
[ -z "$dups" ] || fail "$list lists these more than once: $(printf '%s' "$dups" | tr '\n' ' ')"
while IFS="$(printf '\t')" read -r name color desc; do
  [ -n "$name" ] || continue
  printf '%s' "$color" | grep -qE '^[0-9A-Fa-f]{6}$' || fail "$list: $name needs a color (6 hex digits, no #)"
  [ -n "$desc" ] || fail "$list: $name needs a description"
done <<< "$listed"

# Labels named in .github, one per line: `labels: [a, b]` or `labels: ["a"]` (Dependabot, issue forms), then the
# labels CI tests for. (A function, not comments inside `$(…)`, which bash 3.2 misreads.)
used_labels() {
  {
    grep -rhE '^[[:space:]]*labels:[[:space:]]*\[' .github --include='*.yml' --include='*.yaml' 2>/dev/null \
      | sed -E 's/^[^[]*\[//; s/\].*$//' | tr ',' '\n' || true
    grep -rhoE '(if|&&|\|\||!) has [a-z][a-z0-9-]*' .github/workflows 2>/dev/null | sed -E 's/.* has //' || true
    grep -rhoE "grep -q '\"[^\"]+\"'" .github/workflows 2>/dev/null | sed -E "s/^grep -q '\"//; s/\"'\$//" || true
  } | sed -E 's/^[[:space:]"'"'"']*//; s/[[:space:]"'"'"']*$//' | { grep -v '^$' || true; } | sort -u
}
used="$(used_labels)"
names="$(printf '%s\n' "$listed" | cut -f1)"
while IFS= read -r label; do
  [ -n "$label" ] || continue
  printf '%s\n' "$names" | grep -qxF -- "$label" || fail ".github uses the label \`$label\`, which $list doesn't list"
done <<< "$used"

if [ "$live" = true ]; then
  remote="$(gh label list --limit 1000 --json name,color,description \
    --jq '.[] | [.name, .color, .description] | @tsv')"
  norm() { awk -F'\t' '{ printf "%s\t%s\t%s\n", $1, tolower($2), $3 }' | sort; }
  diff="$(diff <(printf '%s\n' "$listed" | norm) <(printf '%s\n' "$remote" | norm) || true)"
  if [ -n "$diff" ]; then
    fail "GitHub's labels differ from $list (< the list, > GitHub):"
    printf '%s\n' "$diff" | grep -E '^[<>]' >&2
  fi
fi

[ "$failed" = 0 ] || exit 1
echo "labels: ok"
