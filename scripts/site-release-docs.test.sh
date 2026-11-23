#!/usr/bin/env bash
# Tests for scripts/site-release-docs.sh in a scratch repository. Run: scripts/site-release-docs.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/site-release-docs.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

repo="$scratch/repo"
mkdir -p "$repo"
cd "$repo"
git init -q
git config user.email test@example.com
git config user.name test
commit() { git add -A && git commit -qm "$1"; }
write() { mkdir -p "$(dirname "$1")" && printf '%s\n' "$2" > "$1"; }

# No stable tag: nothing changes.
write docs/user/a/page.md 'first'
commit first
out="$("$script" 2>/dev/null)"
[ -z "$out" ] || fail "no tag: prints no tag (got $out)"
[ -z "$(git status --porcelain)" ] || fail "no tag: changes nothing ($(git status --porcelain))"

# v0.9.0, v0.10.0 (newer, though it sorts first as text), then a nightly and main's changes.
git tag v0.9.0
write docs/user/a/page.md 'released'
write docs/agent-guide.md 'guide released'
write docs/user/contributing/contributing.md 'contributing released'
commit second
git tag v0.10.0
write docs/user/a/page.md 'nightly'
commit third
git tag v0.10.1-nightly.20261123
write docs/user/a/page.md 'main'
write docs/user/a/new.md 'unreleased page'
write docs/agent-guide.md 'guide main'
write docs/user/contributing/contributing.md 'contributing main'
write docs/dev/architecture.md 'dev main'
commit main

out="$("$script")"
[ "$out" = v0.10.0 ] || fail "picks the newest stable tag (got $out)"
[ "$(cat docs/user/a/page.md)" = released ] || fail "a changed page is the release's"
[ ! -e docs/user/a/new.md ] || fail "a page added since the release is gone"
[ "$(cat docs/agent-guide.md)" = 'guide released' ] || fail "the agent guide is the release's"
[ "$(cat docs/user/contributing/contributing.md)" = 'contributing main' ] || fail "Contributing stays main's"
[ "$(cat docs/dev/architecture.md)" = 'dev main' ] || fail "the developer guide stays main's"

if [ "$failed" = 0 ]; then echo "site-release-docs: all tests passed"; fi
exit "$failed"
