#!/usr/bin/env bash
# Removes stale Rust build output from this repository's cargo target folders: the main checkout's `target` and every
# `.worktrees/*/target`, or the folders given. Old builds pile up there (a test binary per dependency change, old
# incremental sessions) and a worktree's target/ grew to tens of GB. Tests: scripts/clean-targets.test.sh.
#
#   scripts/clean-targets.sh [--days N] [--dry-run] [<target folder>...]
#
# A build unit (one crate built one way: `.fingerprint/<name>-<hash>` with its `deps/*-<hash>*` and `build/*-<hash>`)
# is stale when cargo has not written it for N days (default 7); so is an incremental session, and a binary cargo
# copied up into the profile folder. Cargo rebuilds whatever it needs again, from sccache when that is set up
# (docs/dev/testing.md "Build output"). It refuses a folder cargo didn't make (one without its CACHEDIR.TAG).
set -euo pipefail

days=7
dry_run=0
targets=()

die() { printf 'clean-targets: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --days) [[ "${2:-}" =~ ^[0-9]+$ ]] || die "--days takes a whole number of days"; days="$2"; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "unknown option: $1" ;;
    *) targets+=("$1"); shift ;;
  esac
done

if [ ${#targets[@]} -eq 0 ]; then
  common="$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir)"
  repo="$(dirname "$common")"
  for t in "$repo/target" "$repo"/.worktrees/*/target; do
    [ -d "$t" ] && targets+=("$t")
  done
fi

kib() { du -sk "$1" 2>/dev/null | cut -f1; }

# Whether nothing under a path was written in the last N days. With 0 days everything is (GNU and BSD find read
# `-mtime -0` differently).
stale() { [ "$days" -eq 0 ] || [ -z "$(find "$1" -mtime "-$days" -print -quit 2>/dev/null)" ]; }

remove() {
  if [ "$dry_run" -eq 1 ]; then
    printf 'would remove %s\n' "$1"
  else
    rm -rf -- "$1"
  fi
}

total=0
for target in ${targets[@]+"${targets[@]}"}; do
  [ -d "$target" ] || die "not a folder: $target"
  [ -f "$target/CACHEDIR.TAG" ] || die "not a cargo target folder (no CACHEDIR.TAG): $target"
  before="$(kib "$target")"
  # Each profile folder (debug, release, <triple>/debug, ...) has a .fingerprint folder.
  while IFS= read -r fingerprints; do
    profile="$(dirname "$fingerprints")"
    for unit in "$fingerprints"/*; do
      [ -d "$unit" ] || continue
      hash="${unit##*-}"
      [[ "$hash" =~ ^[0-9a-f]{16}$ ]] || continue
      stale "$unit" || continue
      for f in "$profile/deps/"*"-$hash" "$profile/deps/"*"-$hash."* "$profile/build/"*"-$hash"; do
        [ -e "$f" ] && remove "$f"
      done
      remove "$unit"
    done
    for session in "$profile/incremental/"*; do
      [ -e "$session" ] && stale "$session" && remove "$session"
    done
    # The binaries cargo copies up (target/debug/pdtd): a hard link keeps the deps copy's space in use.
    for f in "$profile"/*; do
      [ -f "$f" ] && stale "$f" && remove "$f"
    done
  done < <(find "$target" -maxdepth 3 -type d -name .fingerprint)
  after="$(kib "$target")"
  if [ "$dry_run" -eq 0 ]; then
    freed=$((before - after))
    total=$((total + freed))
    printf '%s: %d MB freed, %d MB left\n' "$target" $((freed / 1024)) $((after / 1024))
  fi
done
[ "$dry_run" -eq 1 ] || printf 'total: %d MB freed\n' $((total / 1024))
