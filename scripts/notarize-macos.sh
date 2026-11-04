#!/usr/bin/env bash
# Notarizes a Developer ID-signed app bundle with Apple and staples the ticket, so Gatekeeper opens it without the
# "unidentified developer" warning. Run after scripts/sign-macos.sh.
#
# usage: scripts/notarize-macos.sh <path/to/App.app>               submit, wait, staple (local use)
#        scripts/notarize-macos.sh fingerprint <path/to/App.app>   print the build's fingerprint (run before signing)
#        scripts/notarize-macos.sh submit <path/to/App.app>        upload it, or resume an earlier submission
#        scripts/notarize-macos.sh wait <path/to/App.app> <id>     wait for Apple, then staple and check
# CI runs the three steps apart (.github/actions/bundle-macos) so the submission id is kept as soon as Apple has the
# upload: a re-run of a job that timed out waits on that submission instead of uploading the same build again.
#
# Needs an App Store Connect API key (docs/SDLC.md "Signing"):
#   MACOS_NOTARY_KEY (base64 of the AuthKey_<id>.p8 file), MACOS_NOTARY_KEY_ID, MACOS_NOTARY_ISSUER.
# Optional:
#   PDT_NOTARY_TIMEOUT_MINUTES  how long `wait` waits for Apple (default 120)
#   PDT_NOTARY_POLL_SECONDS     how often it asks (default 60)
#   PDT_NOTARY_LOG_MINUTES      how often it logs that Apple is still working (default 5)
#   PDT_NOTARY_FINGERPRINT      `submit`: this build's fingerprint (from `fingerprint`, before signing)
#   PDT_NOTARY_RECORDS          `submit`: a folder of earlier attempts' records; one for the same fingerprint that
#                               Apple hasn't rejected is resumed instead of uploading again
#   PDT_NOTARY_RECORD           `submit`: the file a new submission's record is written to
#   MACOS_SIGNING_P12           `fingerprint`: part of it, so a new certificate (another code hash) uploads again
set -euo pipefail

timeout_minutes="${PDT_NOTARY_TIMEOUT_MINUTES:-120}"
poll_seconds="${PDT_NOTARY_POLL_SECONDS:-60}"
log_minutes="${PDT_NOTARY_LOG_MINUTES:-5}"

# The value of a string field in notarytool's one-line JSON output.
json_field() { sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$2" | head -n 1; }

summary() { if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then printf '%s\n' "$@" >> "$GITHUB_STEP_SUMMARY"; fi; }

check_app() { [[ -d "$1/Contents/MacOS" ]] || { echo "not an app bundle: $1" >&2; exit 2; }; }

# What the build's notarization ticket covers, before signing: every file's content and every symlink, plus the
# certificate. A re-run of the same job joins the same two builds and signs them with the same certificate, so it has
# the same code hashes and Apple's ticket for an earlier submission staples onto it.
fingerprint() {
  local app="$1"
  check_app "$app"
  {
    (cd "$app" && find . \( -type f -o -type l \) | LC_ALL=C sort | while IFS= read -r f; do
      if [[ -L "$f" ]]; then printf 'link %s %s\n' "$f" "$(readlink "$f")"; else shasum -a 256 "$f"; fi
    done)
    printf 'certificate %s\n' "$(printf '%s' "${MACOS_SIGNING_P12:-}" | shasum -a 256 | cut -d' ' -f1)"
  } | shasum -a 256 | cut -d' ' -f1
}

work=""
key=""
setup() {
  : "${MACOS_NOTARY_KEY:?MACOS_NOTARY_KEY is required}"
  : "${MACOS_NOTARY_KEY_ID:?MACOS_NOTARY_KEY_ID is required}"
  : "${MACOS_NOTARY_ISSUER:?MACOS_NOTARY_ISSUER is required}"
  work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/pdt-notary.XXXXXX")"
  trap 'rm -rf "$work"' EXIT
  key="$work/AuthKey_$MACOS_NOTARY_KEY_ID.p8"
  printf '%s' "$MACOS_NOTARY_KEY" | base64 --decode > "$key"
}

notarytool() { xcrun notarytool "$@" --key "$key" --key-id "$MACOS_NOTARY_KEY_ID" --issuer "$MACOS_NOTARY_ISSUER"; }

# Sets $status to the submission's status ("In Progress", "Accepted", "Invalid", "Rejected"), or "" when Apple
# couldn't be asked.
info() {
  status=""
  if notarytool info "$1" --output-format json > "$work/info.json" 2> "$work/info.err"; then
    status="$(json_field status "$work/info.json")"
  else
    echo "notarytool info $1 failed (will ask again): $(cat "$work/info.err")" >&2
  fi
}

# Sets $id to the newest earlier submission for this build that Apple hasn't rejected, if any (and $status to its
# status).
resumable() {
  local dir="${PDT_NOTARY_RECORDS:-}" fp="${PDT_NOTARY_FINGERPRINT:-}"
  id=""
  [[ -n "$dir" && -n "$fp" && -d "$dir" ]] || return 0
  id="$(find "$dir" -type f -exec cat {} + 2>/dev/null \
    | awk -v fp="$fp" '$1 == fp && $3 + 0 >= best { best = $3 + 0; id = $2 } END { print id }')"
  [[ -n "$id" ]] || return 0
  info "$id"
  case "$status" in
    Invalid | Rejected)
      echo "An earlier attempt's submission $id was $status; uploading this build again."
      id="" ;;
  esac
}

check_command() {
  printf 'xcrun notarytool info %s --key AuthKey_<key id>.p8 --key-id <key id> --issuer <issuer id>' "$1"
}

# Uploads the app, or picks up an earlier submission of the same build; sets $id.
submit() {
  local app="$1" how
  check_app "$app"
  resumable
  if [[ -n "$id" ]]; then
    how="resumed: an earlier attempt uploaded this build"
    echo "Resuming submission $id (${status:-status unknown}) instead of uploading the same build again."
  else
    how="uploaded"
    # notarytool takes a zip; ditto keeps the bundle's symlinks and metadata.
    ditto -c -k --keepParent "$app" "$work/app.zip"
    if ! notarytool submit "$work/app.zip" --output-format json > "$work/submit.json"; then
      cat "$work/submit.json" >&2 || true
      echo "::error title=Notarization upload failed::notarytool submit failed" >&2
      exit 1
    fi
    cat "$work/submit.json"
    echo
    id="$(json_field id "$work/submit.json")"
    [[ -n "$id" ]] || { echo "::error::notarytool submit returned no submission id" >&2; exit 1; }
    echo "Uploaded; submission $id"
    if [[ -n "${PDT_NOTARY_RECORD:-}" && -n "${PDT_NOTARY_FINGERPRINT:-}" ]]; then
      mkdir -p "$(dirname "$PDT_NOTARY_RECORD")"
      printf '%s %s %s\n' "$PDT_NOTARY_FINGERPRINT" "$id" "${GITHUB_RUN_ATTEMPT:-1}" > "$PDT_NOTARY_RECORD"
    fi
  fi
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf 'id=%s\nresumed=%s\n' "$id" "$([[ "$how" == uploaded ]] && echo false || echo true)" >> "$GITHUB_OUTPUT"
  fi
  summary "### macOS notarization" "" "- Submission \`$id\` ($how)" \
    "- Check it: \`$(check_command "$id")\`"
}

# Waits for Apple's verdict on submission $2, then staples the ticket to the app and checks it.
wait_for() {
  local app="$1" id="$2" start last_log=-1 elapsed minutes
  check_app "$app"
  echo "Waiting up to $timeout_minutes min for Apple to notarize submission $id"
  start="$SECONDS"
  while :; do
    info "$id"
    elapsed=$((SECONDS - start))
    minutes=$((elapsed / 60))
    case "$status" in
      Accepted) break ;;
      Invalid | Rejected)
        echo "Apple's log for submission $id:"
        notarytool log "$id" || true
        echo "::error title=Notarization rejected::Apple says $status for submission $id; its log is above."
        summary "- ❌ Apple says **$status**; the job log has Apple's log."
        exit 1 ;;
    esac
    if ((elapsed >= timeout_minutes * 60)); then
      echo "::error title=Notarization timed out::Apple is still working on submission $id after $minutes min." \
        "Re-run the failed job: it waits on this submission instead of uploading again. Check it with: $(check_command "$id")"
      summary "- ⏳ Still ${status:-unknown} after $minutes min. Re-run the failed job to keep waiting on this submission."
      exit 1
    fi
    if ((last_log < 0 || elapsed - last_log >= log_minutes * 60)); then
      echo "still ${status:-unknown} after $minutes min, submission $id"
      last_log="$elapsed"
    fi
    sleep "$poll_seconds"
  done
  echo "Apple accepted submission $id after $minutes min"
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  summary "- ✅ Accepted after $minutes min and stapled."
  echo "Notarized $app"
}

case "${1:-}" in
  fingerprint) fingerprint "${2:?usage: $0 fingerprint <path/to/App.app>}" ;;
  submit)
    app="${2:?usage: $0 submit <path/to/App.app>}"
    setup
    submit "$app" ;;
  wait)
    app="${2:?usage: $0 wait <path/to/App.app> <id>}"
    sub="${3:?usage: $0 wait <path/to/App.app> <id>}"
    setup
    wait_for "$app" "$sub" ;;
  "") echo "usage: $0 <path/to/App.app> | fingerprint|submit <App.app> | wait <App.app> <id>" >&2; exit 2 ;;
  *)
    setup
    submit "$1"
    wait_for "$1" "$id" ;;
esac
