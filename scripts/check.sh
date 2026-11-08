#!/usr/bin/env bash
# The checks CI runs, run locally before pushing (AGENTS.md, docs/SDLC.md "Checks before pushing").
#
#   scripts/check.sh               every check: lint, test, web, site, changelog
#   scripts/check.sh --docs        a docs-only change: site (the docs check, the website's tests and build), changelog
#   scripts/check.sh --no-rust     skip lint and test (the Rust checks)
#   scripts/check.sh --no-web      skip web;  --no-site skips site
#   scripts/check.sh --only <group>[,<group>...]   just those groups (CI's lint job runs its group this way)
#
# Groups, in the order they run (cheap ones first, so a slip fails fast):
#   changelog  the changelog fragments and version (scripts/changelog.sh check, scripts/version.sh check) and the
#              scripts' own tests, that third-party actions are pinned to a commit (check-action-pins.sh) and that
#              the labels .github uses are listed (check-labels.sh)
#   site       the user docs check (pnpm --filter site check-docs), the website's tests and build
#   lint       version, cargo fmt --check, clippy (and pdtd without its integration features)
#   web        web typecheck, unused exports and files (knip), tests, build
#   test       cargo test (pdtd also without its integration features), then the generated files are committed
#
# CI runs tests with cargo nextest and splits the groups across jobs; this runs plain `cargo test`. It stops at the
# first failure. Bash 3.2 compatible (macOS).
set -euo pipefail

cd "$(dirname "$0")/.."

ALL="changelog site lint web test"
groups="$ALL"
usage() { sed -n '4,9p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
drop() { groups="$(printf '%s\n' $groups | grep -vx "$1" | tr '\n' ' ')"; }
while [ $# -gt 0 ]; do
  case "$1" in
    --docs) groups="changelog site" ;;
    --no-rust) drop lint; drop test ;;
    --no-web) drop web ;;
    --no-site) drop site ;;
    --only)
      [ $# -ge 2 ] || usage
      groups="$(printf '%s' "$2" | tr ',' ' ')"
      for g in $groups; do printf '%s\n' $ALL | grep -qx "$g" || { echo "check: unknown group: $g" >&2; usage; }; done
      shift ;;
    -h|--help) usage ;;
    *) echo "check: unknown option: $1" >&2; usage ;;
  esac
  shift
done

step() { printf '\n==> %s\n' "$*"; "$@"; }

check_changelog() {
  step scripts/changelog.sh check
  step scripts/version.sh check
  step scripts/changelog.test.sh
  step scripts/version.test.sh
  step scripts/release-tag.test.sh
  step scripts/scratch-daemon.test.sh
  step scripts/clean-targets.test.sh
  step scripts/google-build-values.test.sh
  step scripts/release-sign.test.sh
  step scripts/notarize-macos.test.sh
  step scripts/check-action-pins.sh
  step scripts/check-labels.test.sh
  step scripts/check-labels.sh
}

check_site() {
  step pnpm --filter site check-docs
  step pnpm --filter site test
  # Astro wants a newer Node than some dev machines have; CI builds the website on Node 24.
  if [ "$(node -p 'process.versions.node.split(".")[0]')" -ge 24 ]; then
    step pnpm --filter site build
  else
    printf '\n==> skipped the website build: Node %s is older than CI'"'"'s 24\n' "$(node -v)"
  fi
}

check_lint() {
  step scripts/version.sh check
  step cargo fmt --all -- --check
  step cargo clippy --all-targets -- -D warnings
  step cargo clippy -p pdtd --no-default-features --all-targets -- -D warnings
}

check_web() {
  step pnpm --dir web typecheck
  step pnpm --dir web knip
  step pnpm --dir web test
  step pnpm --dir web build
}

check_test() {
  step cargo test
  step cargo test -p pdtd --no-default-features
  # cargo test regenerates the TS types, the web tests' fixtures and the website's plugin list: CI fails unless they
  # are committed.
  out="$(git status --porcelain -- web/src/api/gen site/src/data)"
  if [ -n "$out" ]; then printf 'Generated files are not committed:\n%s\n' "$out" >&2; exit 1; fi
}

for g in $ALL; do
  case " $groups " in *" $g "*) printf '\n=== %s\n' "$g"; "check_$g" ;; esac
done
printf '\nAll checks passed (%s).\n' "$(printf '%s' "$groups" | xargs)"
