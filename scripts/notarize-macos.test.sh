#!/usr/bin/env bash
# Tests for scripts/notarize-macos.sh with a stand-in xcrun (notarytool, stapler) and ditto, in a scratch folder.
# Run: scripts/notarize-macos.test.sh (CI runs it in the `changelog` job.)
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/notarize-macos.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
failed=0

fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }
has() { grep -qF -- "$2" <<<"$1"; }

# The stand-ins log each call to $STUB/calls; `notarytool info` answers with the next line of $STUB/statuses (the
# last one repeats; FAIL makes the call fail).
mkdir -p "$scratch/bin"
cat >"$scratch/bin/xcrun" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB/calls"
case "$1 $2" in
  "notarytool submit") echo '{"id":"new-sub","message":"Successfully uploaded file","path":"app.zip"}' ;;
  "notarytool info")
    n="$(grep -c '^notarytool info' "$STUB/calls")"
    s="$(sed -n "${n}p" "$STUB/statuses")"
    [ -n "$s" ] || s="$(tail -n 1 "$STUB/statuses")"
    if [ "$s" = FAIL ]; then echo "network down" >&2; exit 1; fi
    printf '{"createdDate":"2026-11-04T08:17:00Z","id":"%s","name":"app.zip","status":"%s"}\n' "$3" "$s" ;;
  "notarytool log") echo '{"issues":[{"message":"The binary is not signed."}]}' ;;
  "stapler staple" | "stapler validate") echo "stapler $2 ok" ;;
  *) echo "unexpected: $*" >&2; exit 1 ;;
esac
EOF
cat >"$scratch/bin/ditto" <<'EOF'
#!/usr/bin/env bash
echo "ditto $*" >>"$STUB/calls"
EOF
chmod +x "$scratch/bin/xcrun" "$scratch/bin/ditto"

app="$scratch/PDT.app"
mkdir -p "$app/Contents/MacOS"
printf 'exe' >"$app/Contents/MacOS/pdt"
ln -s pdt "$app/Contents/MacOS/link"

# Runs the script with arguments "$@" and the statuses in $statuses; sets $status, $out and $calls.
run() {
  rm -rf "$scratch/stub" "$scratch/out" "$scratch/summary"
  mkdir -p "$scratch/stub"
  printf '%s\n' "$statuses" >"$scratch/stub/statuses"
  : >"$scratch/stub/calls"; : >"$scratch/out"; : >"$scratch/summary"
  status=0
  out="$(env -i PATH="$scratch/bin:$PATH" STUB="$scratch/stub" TMPDIR="$scratch" \
    MACOS_NOTARY_KEY="$(printf key | base64)" MACOS_NOTARY_KEY_ID=KEYID MACOS_NOTARY_ISSUER=ISSUER \
    PDT_NOTARY_POLL_SECONDS=0 PDT_NOTARY_LOG_MINUTES=0 PDT_NOTARY_TIMEOUT_MINUTES="${timeout:-5}" \
    PDT_NOTARY_FINGERPRINT="${fp:-}" PDT_NOTARY_RECORDS="${records:-}" PDT_NOTARY_RECORD="${record:-}" \
    GITHUB_OUTPUT="$scratch/out" GITHUB_STEP_SUMMARY="$scratch/summary" GITHUB_RUN_ATTEMPT="${attempt:-1}" \
    "$script" "$@" 2>&1)" || status=$?
  calls="$(cat "$scratch/stub/calls")"
}

# --- the fingerprint: the same for the same files, another for other files or another certificate
fp1="$(env -i PATH="$PATH" "$script" fingerprint "$app")"
[ "${#fp1}" = 64 ] || fail "fingerprint: a sha256, got: $fp1"
[ "$(env -i PATH="$PATH" "$script" fingerprint "$app")" = "$fp1" ] || fail "fingerprint: stable"
[ "$(env -i PATH="$PATH" MACOS_SIGNING_P12=other "$script" fingerprint "$app")" != "$fp1" ] \
  || fail "fingerprint: changes with the certificate"
printf 'exe2' >"$app/Contents/MacOS/pdt"
[ "$(env -i PATH="$PATH" "$script" fingerprint "$app")" != "$fp1" ] || fail "fingerprint: changes with the files"
printf 'exe' >"$app/Contents/MacOS/pdt"

# --- submit: uploads, records the id at once, and puts it and the check command in the summary
fp="$fp1"; records="$scratch/records"; record="$scratch/record/submission"; attempt=1; statuses="In Progress"
run submit "$app"
[ "$status" = 0 ] || fail "submit: exits 0 (got $status): $out"
has "$calls" "notarytool submit" || fail "submit: uploads, calls: $calls"
has "$calls" "--key-id KEYID --issuer ISSUER" || fail "submit: passes the key, calls: $calls"
has "$calls" "--wait" && fail "submit: doesn't wait, calls: $calls"
[ "$(cat "$record")" = "$fp1 new-sub 1" ] || fail "submit: records fingerprint, id and attempt, got: $(cat "$record")"
has "$(cat "$scratch/out")" "id=new-sub" || fail "submit: outputs the id"
has "$(cat "$scratch/out")" "resumed=false" || fail "submit: outputs resumed=false"
has "$(cat "$scratch/summary")" "xcrun notarytool info new-sub --key" || fail "submit: summary has the check command"

# --- submit on a re-run: an earlier attempt's submission for this build is resumed, not uploaded again
mkdir -p "$records"
printf '%s old-sub 1\n' "$fp1" >"$records/a"
printf 'other-build other-sub 2\n' >"$records/b"
rm -f "$record"; attempt=2; statuses="In Progress"
run submit "$app"
[ "$status" = 0 ] || fail "resume: exits 0 (got $status): $out"
has "$calls" "notarytool submit" && fail "resume: doesn't upload again, calls: $calls"
has "$calls" "notarytool info old-sub" || fail "resume: asks about the earlier submission, calls: $calls"
has "$out" "Resuming submission old-sub (In Progress)" || fail "resume: says so, got: $out"
has "$(cat "$scratch/out")" "id=old-sub" || fail "resume: outputs the earlier id"
has "$(cat "$scratch/out")" "resumed=true" || fail "resume: outputs resumed=true"
[ ! -e "$record" ] || fail "resume: records nothing new"

# --- the newest attempt's record wins
printf '%s newer-sub 3\n' "$fp1" >"$records/c"
run submit "$app"
has "$(cat "$scratch/out")" "id=newer-sub" || fail "resume: takes the newest attempt, got: $(cat "$scratch/out")"
rm -f "$records/c"

# --- an earlier submission Apple rejected is uploaded again
statuses="Invalid"
run submit "$app"
[ "$status" = 0 ] || fail "rejected earlier: exits 0 (got $status): $out"
has "$calls" "notarytool submit" || fail "rejected earlier: uploads again, calls: $calls"
has "$(cat "$scratch/out")" "id=new-sub" || fail "rejected earlier: outputs the new id"
[ "$(cat "$record")" = "$fp1 new-sub 2" ] || fail "rejected earlier: records the new one"

# --- no fingerprint: never resumes
fp=""; statuses="In Progress"
run submit "$app"
has "$calls" "notarytool submit" || fail "no fingerprint: uploads, calls: $calls"
fp="$fp1"

# --- wait: Apple is slow, then accepts; logs progress, staples and checks
statuses="$(printf 'In Progress\nIn Progress\nAccepted')"
run wait "$app" old-sub
[ "$status" = 0 ] || fail "accept: exits 0 (got $status): $out"
has "$out" "still In Progress after 0 min, submission old-sub" || fail "accept: logs progress, got: $out"
[ "$(grep -c '^notarytool info old-sub' <<<"$calls")" = 3 ] || fail "accept: polls until accepted, calls: $calls"
has "$calls" "stapler staple $app" || fail "accept: staples, calls: $calls"
has "$calls" "stapler validate $app" || fail "accept: validates, calls: $calls"
has "$(cat "$scratch/summary")" "Accepted" || fail "accept: says so in the summary"

# --- wait: rejected fails with Apple's log in the output
statuses="Invalid"
run wait "$app" old-sub
[ "$status" != 0 ] || fail "reject: fails"
has "$out" "The binary is not signed." || fail "reject: prints Apple's log, got: $out"
has "$out" "::error title=Notarization rejected::" || fail "reject: says why, got: $out"
has "$calls" "stapler" && fail "reject: staples nothing, calls: $calls"

# --- wait: still working at the limit fails and says to re-run
statuses="In Progress"; timeout=0
run wait "$app" old-sub
[ "$status" != 0 ] || fail "timeout: fails"
has "$out" "::error title=Notarization timed out::" || fail "timeout: says so, got: $out"
has "$out" "Re-run the failed job" || fail "timeout: says to re-run, got: $out"
timeout=5

# --- wait: a failed `notarytool info` is asked again
statuses="$(printf 'FAIL\nAccepted')"
run wait "$app" old-sub
[ "$status" = 0 ] || fail "retry: exits 0 (got $status): $out"
has "$out" "will ask again" || fail "retry: says so, got: $out"

# --- the whole thing in one go (local use): upload, wait, staple
records=""; record=""; statuses="Accepted"
run "$app"
[ "$status" = 0 ] || fail "one go: exits 0 (got $status): $out"
has "$calls" "notarytool submit" || fail "one go: uploads, calls: $calls"
has "$calls" "notarytool info new-sub" || fail "one go: waits on the new submission, calls: $calls"
has "$out" "Notarized $app" || fail "one go: says so, got: $out"

if [ "$failed" = 0 ]; then echo "notarize-macos: ok"; else exit 1; fi
