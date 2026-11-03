#!/usr/bin/env bash
# Warns when a release build is missing any of PDT's own Google sign-in values (PDT_GOOGLE_*, built into pdtd by
# crates/pdt-int-google/src/builtin.rs): a missing Actions secret would otherwise ship silently, and the build would
# lack that part of Google (the file picker, agents' sign-in, or the sign-in itself). Each empty one is an Actions
# `::warning::` naming it and a line in the job summary ($GITHUB_STEP_SUMMARY). It never fails the build.
# Run: scripts/google-build-values.sh (the bundle action runs it on release builds). Exits 0.
set -euo pipefail

names=(PDT_GOOGLE_CLIENT_ID PDT_GOOGLE_CLIENT_SECRET PDT_GOOGLE_AGENT_CLIENT_ID PDT_GOOGLE_AGENT_CLIENT_SECRET
  PDT_GOOGLE_PICKER_KEY PDT_GOOGLE_PROJECT_NUMBER)
missing=()
for n in "${names[@]}"; do
  if [ -z "${!n:-}" ]; then missing+=("$n"); fi
done
if [ "${#missing[@]}" = 0 ]; then
  echo "PDT's Google sign-in: all ${#names[@]} build values are set."
  exit 0
fi
for n in "${missing[@]}"; do
  echo "::warning title=Google build value missing::$n is empty: this build ships without it (set the Actions secret $n)."
done
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### ⚠️ PDT's Google sign-in is incomplete in this build"
    for n in "${missing[@]}"; do echo "- \`$n\` is empty: set the Actions secret of that name."; done
  } >> "$GITHUB_STEP_SUMMARY"
fi
