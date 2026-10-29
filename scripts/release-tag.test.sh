#!/usr/bin/env bash
# Tests for scripts/release-tag.sh with stand-in `git` and `gh`, in a scratch folder. Run: scripts/release-tag.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/release-tag.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

# Stand-ins: `git ls-remote` prints $LS_REMOTE_OUT and exits $LS_REMOTE_STATUS; `gh` logs its arguments to
# $scratch/gh.log, prints a 404 body on stdout as the real one does on an HTTP error, and exits $GH_STATUS.
mkdir -p "$scratch/bin"
cat > "$scratch/bin/git" <<'EOF'
#!/usr/bin/env bash
[ "$1" = ls-remote ] || { echo "unexpected git $*" >&2; exit 99; }
if [ -n "${LS_REMOTE_OUT:-}" ]; then printf '%s\n' "$LS_REMOTE_OUT"; fi
if [ "${LS_REMOTE_STATUS:-0}" = 128 ]; then echo "fatal: Authentication failed" >&2; fi
exit "${LS_REMOTE_STATUS:-0}"
EOF
cat > "$scratch/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SCRATCH/gh.log"
if [ "${GH_STATUS:-0}" != 0 ]; then echo '{"message":"Not Found","status":"404"}'; fi
exit "${GH_STATUS:-0}"
EOF
chmod +x "$scratch/bin/git" "$scratch/bin/gh"

sha=08737ddf0000000000000000000000000000000a
other=1111111111111111111111111111111111111111
# Runs the script with the stand-ins; sets $status and $out (stdout and stderr).
run() {
  rm -f "$scratch/gh.log"
  status=0
  out="$(PATH="$scratch/bin:$PATH" SCRATCH="$scratch" GITHUB_REPOSITORY=o/r "$script" "$@" 2>&1)" || status=$?
}
gh_log() { cat "$scratch/gh.log" 2>/dev/null || true; }

# No tag yet (ls-remote --exit-code exits 2): it is made at the commit.
LS_REMOTE_OUT='' LS_REMOTE_STATUS=2 run v0.2.0 "$sha"
[ "$status" = 0 ] || fail "no tag: exits 0 (got $status: $out)"
[ "$(gh_log)" = "api repos/o/r/git/refs -f ref=refs/tags/v0.2.0 -f sha=$sha" ] || fail "no tag: makes it (gh: $(gh_log))"

# The tag at the same commit: a re-run, nothing made.
LS_REMOTE_OUT="$sha	refs/tags/v0.2.0" LS_REMOTE_STATUS=0 run v0.2.0 "$sha"
[ "$status" = 0 ] || fail "same sha: exits 0 (got $status: $out)"
[ -z "$(gh_log)" ] || fail "same sha: makes nothing (gh: $(gh_log))"

# The tag at another commit: fails and names it.
LS_REMOTE_OUT="$other	refs/tags/v0.2.0" LS_REMOTE_STATUS=0 run v0.2.0 "$sha"
[ "$status" = 1 ] || fail "other sha: exits 1 (got $status)"
printf '%s' "$out" | grep -q "already points at $other, not $sha" || fail "other sha: says where it points ($out)"
[ -z "$(gh_log)" ] || fail "other sha: makes nothing (gh: $(gh_log))"

# The lookup fails (auth, network): fails, and makes nothing.
LS_REMOTE_OUT='' LS_REMOTE_STATUS=128 run v0.2.0 "$sha"
[ "$status" = 1 ] || fail "lookup error: exits 1 (got $status)"
printf '%s' "$out" | grep -q "Couldn't read the tags" || fail "lookup error: says so ($out)"
[ -z "$(gh_log)" ] || fail "lookup error: makes nothing (gh: $(gh_log))"

# Making the tag fails: fails, with GitHub's body not mistaken for anything.
LS_REMOTE_OUT='' LS_REMOTE_STATUS=2 GH_STATUS=1 run v0.2.0 "$sha"
[ "$status" = 1 ] || fail "create error: exits 1 (got $status)"
printf '%s' "$out" | grep -q "Couldn't make v0.2.0" || fail "create error: says so ($out)"

# Usage.
run v0.2.0
[ "$status" = 2 ] || fail "one argument: usage (got $status)"

if [ "$failed" = 0 ]; then echo "release-tag tests passed"; else exit 1; fi
