#!/usr/bin/env bash
# Tests for scripts/nightly-tag.sh with a stand-in `gh`, in a scratch folder. Run: scripts/nightly-tag.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/nightly-tag.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

# Stand-in: `gh api` prints $REFS (the "<tag> <sha>" lines its --jq would make) and exits $API_STATUS; `gh release
# view <tag>` exits 0 for the tags in $PUBLISHED, else 1. Every call is logged to $scratch/gh.log.
mkdir -p "$scratch/bin"
cat > "$scratch/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SCRATCH/gh.log"
case "$1 $2" in
  "api "*)
    if [ "${API_STATUS:-0}" != 0 ]; then echo '{"message":"Bad credentials","status":"401"}'; exit "$API_STATUS"; fi
    if [ -n "${REFS:-}" ]; then printf '%s\n' "$REFS"; fi ;;
  "release view")
    for t in ${PUBLISHED:-}; do [ "$t" = "$3" ] && exit 0; done
    echo "release not found" >&2; exit 1 ;;
  *) echo "unexpected gh $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$scratch/bin/gh"

head=08737ddf0000000000000000000000000000000a
old=1111111111111111111111111111111111111111
# Runs the script on main's head $head, 0.5.1 on 2026-10-31; sets $status, $out (stdout) and $summary.
run() {
  rm -f "$scratch/gh.log" "$scratch/summary"
  status=0
  out="$(PATH="$scratch/bin:$PATH" SCRATCH="$scratch" GITHUB_REPOSITORY=o/r GITHUB_STEP_SUMMARY="$scratch/summary" \
    "$script" "$head" 0.5.1 20261031 2>"$scratch/err")" || status=$?
  summary="$(cat "$scratch/summary" 2>/dev/null || true)"
}
expect() { # <case> <expected stdout>
  [ "$status" = 0 ] || fail "$1: exits 0 (got $status: $(cat "$scratch/err"))"
  [ "$out" = "$2" ] || fail "$1: prints
$2
got
$out"
}
new_build() { printf 'skip=false\ntag=v%s\nversion=%s\nnew=true' "$1" "$1"; }

# --- scheduled (SCHEDULED=true)

# A new head: the day's nightly.
export SCHEDULED=true
REFS="v0.5.0 $old" run
expect "scheduled, new head" "$(new_build 0.5.1-nightly.20261031)"

# A new head, a nightly already out today on an older commit: the day's next build.
REFS="v0.5.0 $old
v0.5.1-nightly.20261031 $old" PUBLISHED=v0.5.1-nightly.20261031 run
expect "scheduled, new head, second build" "$(new_build 0.5.1-nightly.20261031.2)"

# The head is a published nightly: nothing, and says so.
REFS="v0.5.0 $old
v0.5.1-nightly.20261030 $head" PUBLISHED=v0.5.1-nightly.20261030 run
expect "scheduled, nightly at head" "skip=true"
printf '%s' "$summary" | grep -q "released already (v0.5.1-nightly.20261030): no nightly tonight" ||
  fail "scheduled, nightly at head: says so ($summary)"

# The head is a stable release (the release PR just merged): nothing, without looking at releases.
REFS="v0.5.0 $head" run
expect "scheduled, stable at head" "skip=true"
grep -q "release view" "$scratch/gh.log" && fail "scheduled, stable at head: looks up no release ($(cat "$scratch/gh.log"))"

# A stable release and an unpublished nightly at the head: still nothing (it is released).
REFS="v0.5.1-nightly.20261030 $head
v0.5.1 $head" run
expect "scheduled, stable and failed nightly at head" "skip=true"

# The head's nightly tag never got a release (a failed night): that tag again, not a new one.
REFS="v0.5.0 $old
v0.5.1-nightly.20261030 $head" run
expect "scheduled, failed night" "$(printf 'skip=false\ntag=v0.5.1-nightly.20261030\nversion=0.5.1-nightly.20261030\nnew=false')"
printf '%s' "$summary" | grep -q "Building v0.5.1-nightly.20261030 again" || fail "scheduled, failed night: says so ($summary)"

# The tag lookup fails: fails, and picks nothing.
API_STATUS=1 run
[ "$status" = 1 ] || fail "scheduled, lookup error: exits 1 (got $status)"
[ -z "$out" ] || fail "scheduled, lookup error: prints nothing ($out)"

# --- by hand (no SCHEDULED): always the day's next build, even on a released head.
unset SCHEDULED
REFS="v0.5.0 $old
v0.5.1-nightly.20261031 $head" PUBLISHED=v0.5.1-nightly.20261031 run
expect "by hand, nightly at head" "$(new_build 0.5.1-nightly.20261031.2)"
grep -q "release view" "$scratch/gh.log" && fail "by hand: looks up no release ($(cat "$scratch/gh.log"))"

REFS="v0.5.0 $head" run
expect "by hand, stable at head" "$(new_build 0.5.1-nightly.20261031)"

SCHEDULED=false REFS="v0.5.0 $head" run
expect "scheduled=false, stable at head" "$(new_build 0.5.1-nightly.20261031)"

# Another prefix (a dry run on a fork).
TAG_PREFIX=test-v REFS="test-v0.5.1-nightly.20261031 $old" run
expect "test prefix" "$(printf 'skip=false\ntag=test-v0.5.1-nightly.20261031.2\nversion=0.5.1-nightly.20261031.2\nnew=true')"

# Usage.
status=0; "$script" "$head" >/dev/null 2>&1 || status=$?
[ "$status" = 2 ] || fail "one argument: usage (got $status)"

if [ "$failed" = 0 ]; then echo "nightly-tag tests passed"; else exit 1; fi
