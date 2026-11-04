#!/usr/bin/env bash
# Tests for scripts/google-build-values.sh, in a scratch folder. Run: scripts/google-build-values.test.sh
# (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/google-build-values.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

all=(PDT_GOOGLE_CLIENT_ID=id PDT_GOOGLE_CLIENT_SECRET=secret PDT_GOOGLE_AGENT_CLIENT_ID=aid
  PDT_GOOGLE_AGENT_CLIENT_SECRET=asecret PDT_GOOGLE_DESKTOP_CLIENT_ID=did PDT_GOOGLE_DESKTOP_CLIENT_SECRET=dsecret
  PDT_GOOGLE_DESKTOP_AGENT_CLIENT_ID=daid PDT_GOOGLE_DESKTOP_AGENT_CLIENT_SECRET=dasecret PDT_GOOGLE_PICKER_KEY=key
  PDT_GOOGLE_PROJECT_NUMBER=123)
# Runs the script with only the given environment (and PATH); sets $status, $out and $summary.
run() {
  rm -f "$scratch/summary"
  status=0
  out="$(env -i PATH="$PATH" GITHUB_STEP_SUMMARY="$scratch/summary" "$@" "$script" 2>&1)" || status=$?
  summary="$(cat "$scratch/summary" 2>/dev/null || true)"
}
has() { grep -qF -- "$2" <<<"$1"; }

# --- every value set: no warning, no summary
run "${all[@]}"
[ "$status" = 0 ] || fail "all set: exits 0 (got $status)"
has "$out" "::warning" && fail "all set: no warning, got: $out"
[ -z "$summary" ] || fail "all set: nothing in the summary, got: $summary"

# --- the Desktop clients missing: a warning for each
run "${all[@]}" PDT_GOOGLE_DESKTOP_CLIENT_ID= PDT_GOOGLE_DESKTOP_AGENT_CLIENT_SECRET=
[ "$(grep -c '::warning' <<<"$out")" = 2 ] || fail "desktop empty: two warnings, got: $out"
has "$out" "::PDT_GOOGLE_DESKTOP_CLIENT_ID is empty" || fail "desktop empty: names the client, got: $out"

# --- the picker key empty (v0.6.0): one warning naming it, and a summary line
run "${all[@]}" PDT_GOOGLE_PICKER_KEY=
[ "$status" = 0 ] || fail "picker key empty: exits 0, never failing the build (got $status)"
has "$out" "::warning title=Google build value missing::PDT_GOOGLE_PICKER_KEY is empty" \
  || fail "picker key empty: warns naming it, got: $out"
[ "$(grep -c '::warning' <<<"$out")" = 1 ] || fail "picker key empty: exactly one warning, got: $out"
has "$summary" '`PDT_GOOGLE_PICKER_KEY` is empty' || fail "picker key empty: in the summary, got: $summary"
has "$summary" 'PDT_GOOGLE_CLIENT_ID' && fail "picker key empty: only the empty one in the summary, got: $summary"

# --- none set (unset, not only empty): a warning for each of the ten
run
[ "$status" = 0 ] || fail "none set: exits 0 (got $status)"
[ "$(grep -c '::warning' <<<"$out")" = 10 ] || fail "none set: ten warnings, got: $out"
for n in PDT_GOOGLE_CLIENT_ID PDT_GOOGLE_CLIENT_SECRET PDT_GOOGLE_AGENT_CLIENT_ID PDT_GOOGLE_AGENT_CLIENT_SECRET \
  PDT_GOOGLE_DESKTOP_CLIENT_ID PDT_GOOGLE_DESKTOP_CLIENT_SECRET PDT_GOOGLE_DESKTOP_AGENT_CLIENT_ID \
  PDT_GOOGLE_DESKTOP_AGENT_CLIENT_SECRET PDT_GOOGLE_PICKER_KEY PDT_GOOGLE_PROJECT_NUMBER; do
  has "$out" "::$n is empty" || fail "none set: warns about $n"
done

# --- no job summary (outside Actions): still warns
status=0
out="$(env -i PATH="$PATH" "${all[@]}" PDT_GOOGLE_PROJECT_NUMBER= "$script" 2>&1)" || status=$?
[ "$status" = 0 ] || fail "no summary file: exits 0 (got $status)"
has "$out" "PDT_GOOGLE_PROJECT_NUMBER is empty" || fail "no summary file: still warns, got: $out"

if [ "$failed" = 0 ]; then echo "google-build-values: all tests passed"; fi
exit "$failed"
