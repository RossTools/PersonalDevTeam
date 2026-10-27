#!/usr/bin/env bash
# Upgrade rehearsal (PDT-179): runs this checkout's pdtd against a copy of a real data dir and checks that the upgrade
# would go through: migrations, integrations, connections, workflows, dry runs, tool lists and the GitHub login.
#
# The source data dir is only read: its database is copied with SQLite's online backup (consistent while the stable
# instance runs) and nothing else of it is touched. The copy is served by a dev instance in rehearsal mode
# (PDT_REHEARSAL=1), which resumes no run or session, starts no background loop (poll, workflow engine, scheduler,
# remote access, updates) and keeps secrets in memory, so it never reads the keychain or acts on the copy's repos.
#
# Usage: scripts/rehearse-upgrade.sh [--from DIR] [--port N] [--bin PDTD] [--report FILE] [--keep]
#   --from DIR     the data dir to copy (default: the stable instance's, e.g. ~/Library/Application Support/pdt)
#   --port N       the rehearsal instance's port (default 7421; stop the dev instance first, or pick another)
#   --bin PDTD     a pdtd binary to run instead of building this checkout's (cargo build -p pdtd)
#   --report FILE  also write the report (Markdown) there
#   --keep         keep the copy and the instance's log afterwards (their folder is printed)
# Exit status: 0 when every check passes, 1 when one fails, 2 when the rehearsal could not run.
# Needs: cargo (unless --bin), curl, jq, sqlite3. See docs/reference/host.md "Upgrade rehearsal".
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT=7421
FROM=""
BIN=""
REPORT_FILE=""
KEEP=0

die() { echo "rehearse-upgrade: $*" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="${2:?--from needs a folder}"; shift 2 ;;
    --port) PORT="${2:?--port needs a number}"; shift 2 ;;
    --bin) BIN="${2:?--bin needs a path}"; shift 2 ;;
    --report) REPORT_FILE="${2:?--report needs a file}"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

for tool in curl jq sqlite3; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done
[ "$PORT" != 7420 ] || die "port 7420 is the stable instance's; pick another"

if [ -z "$FROM" ]; then
  case "$(uname -s)" in
    Darwin) FROM="$HOME/Library/Application Support/pdt" ;;
    *) FROM="${XDG_DATA_HOME:-$HOME/.local/share}/pdt" ;;
  esac
fi
[ -f "$FROM/pdt.db" ] || die "no database at $FROM/pdt.db (pass --from)"
FROM="$(cd "$FROM" && pwd)"

if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$PORT/api/health"; then
  die "port $PORT is in use (a dev instance?): stop it or pass --port"
fi

# What is being rehearsed.
HEAD="$(git -C "$ROOT" rev-parse --short HEAD)"
BRANCH="$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)"
MAIN="$(git -C "$ROOT" rev-parse --short origin/main 2>/dev/null || echo '?')"
if [ "$HEAD" != "$MAIN" ]; then
  echo "note: this checkout ($BRANCH at $HEAD) is not origin/main ($MAIN); rehearsing it anyway" >&2
fi

if [ -z "$BIN" ]; then
  echo "building pdtd ($BRANCH at $HEAD)…" >&2
  (cd "$ROOT" && cargo build -q -p pdtd) || die "cargo build -p pdtd failed"
  BIN="$ROOT/target/debug/pdtd"
fi
[ -x "$BIN" ] || die "$BIN is not an executable"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/pdt-rehearsal.XXXXXX")"
DATA="$WORK/data"
mkdir -p "$DATA"
PID=""
cleanup() {
  if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  if [ "$KEEP" = 1 ]; then
    echo "kept the copy and the log in $WORK" >&2
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

# ---- the copy ----------------------------------------------------------------------------------------------------
echo "copying $FROM…" >&2
# Read-only: the online backup takes a consistent snapshot while the stable instance writes.
sqlite3 "file:$FROM/pdt.db?mode=ro" ".backup '$DATA/pdt.db'" || die "could not copy $FROM/pdt.db"
for d in sessions pm; do
  [ -d "$FROM/$d" ] && cp -R "$FROM/$d" "$DATA/$d"
done
q() { sqlite3 -readonly "$DATA/pdt.db" "$1"; }
MIGRATIONS_BEFORE="$(q 'SELECT count(*) FROM _sqlx_migrations WHERE success = 1' 2>/dev/null || echo 0)"

# ---- the rehearsal instance --------------------------------------------------------------------------------------
WEB_DIR=""
[ -f "$ROOT/web/dist/index.html" ] && WEB_DIR="$ROOT/web/dist"
LOG="$WORK/pdtd.out"
# Every PDT_* variable of this shell is dropped, so nothing points the instance anywhere real.
env $(env | sed -n 's/^\(PDT_[A-Za-z0-9_]*\)=.*/-u \1/p') \
  PDT_PROFILE=dev PDT_PORT="$PORT" PDT_DATA_DIR="$DATA" PDT_REHEARSAL=1 PDT_SECRET_STORE=memory \
  PDT_TAILSCALE_BIN="$WORK/no-tailscale" PDT_WEB_DIR="$WEB_DIR" \
  PDT_CLAUDE_PROGRAM="$WORK/no-claude" PDT_CODEX_PROGRAM="$WORK/no-codex" PDT_DEVIN_PROGRAM="$WORK/no-devin" \
  "$BIN" >"$LOG" 2>&1 &
PID=$!

API="http://127.0.0.1:$PORT/api"
get() { curl -sf --max-time 30 "$API$1"; }
# Prints the body either way; fails unless the answer is a 2xx.
post() {
  local out code
  out="$(curl -s --max-time 60 -w '\n%{http_code}' -H 'content-type: application/json' -X POST --data-binary @- "$API$1")"
  code="${out##*$'\n'}"
  printf '%s' "${out%$'\n'*}"
  [ "${code:0:1}" = 2 ]
}

UP=0
for _ in $(seq 1 240); do
  if get /health >/dev/null 2>&1; then UP=1; break; fi
  kill -0 "$PID" 2>/dev/null || break
  sleep 0.5
done

# ---- the report --------------------------------------------------------------------------------------------------
REPORT="$WORK/report.md"
FAILS=0
{
  echo "# Upgrade rehearsal"
  echo
  echo "- Build: \`$BRANCH\` at \`$HEAD\` (origin/main is \`$MAIN\`)"
  echo "- Data: a copy of \`$FROM\` taken $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo
} >"$REPORT"
line() { echo "$1" >>"$REPORT"; }
pass() { line "- PASS — $1"; }
fail() { line "- **FAIL** — $1"; FAILS=$((FAILS + 1)); }
note() { line "- note — $1"; }
section() { line ""; line "## $1"; line ""; }

section "Migrations"
MIGRATION_FILES="$(find "$ROOT/crates/pdt-store/migrations" -name '*.sql' | wc -l | tr -d ' ')"
if [ "$UP" != 1 ]; then
  fail "pdtd did not start on the copy; its log ends:"
  line '```'
  tail -n 30 "$LOG" >>"$REPORT"
  line '```'
else
  APPLIED="$(q 'SELECT count(*) FROM _sqlx_migrations WHERE success = 1')"
  FAILED="$(q 'SELECT group_concat(description, ", ") FROM _sqlx_migrations WHERE success = 0')"
  if [ "$APPLIED" = "$MIGRATION_FILES" ] && [ -z "$FAILED" ]; then
    pass "all $APPLIED migrations applied ($((APPLIED - MIGRATIONS_BEFORE)) new to this data)"
  else
    fail "$APPLIED of $MIGRATION_FILES migrations applied${FAILED:+; failed: $FAILED}"
  fi
fi

if [ "$UP" = 1 ]; then
  INTEGRATIONS="$(get /integrations)"
  CONNECTIONS="$(get /connections)"
  KINDS="$(get /connections/kinds | jq '[.[].id]')"
  CATALOG="$(get /workflows/catalog)"
  WORKFLOWS="$(get /workflows)"

  section "Integrations"
  note "enabled: $(echo "$INTEGRATIONS" | jq -r '[.[] | select(.enabled) | .label] | if length == 0 then "none" else join(", ") end')"
  INT_GAPS=0
  while IFS=$'\t' read -r name kind; do
    [ -n "$name" ] || continue
    if ! echo "$KINDS" | jq -e --arg k "$kind" 'index($k) != null' >/dev/null; then
      fail "connection **$name** is a \`$kind\` connection, whose integration is not enabled"
      INT_GAPS=$((INT_GAPS + 1))
    fi
  done < <(echo "$CONNECTIONS" | jq -r '.[] | [.name, .kind] | @tsv')
  while IFS=$'\t' read -r id name; do
    [ -n "$id" ] || continue
    missing="$(get "/workflows/$id" | jq -r --argjson cat "$(echo "$CATALOG" | jq '[.[].id]')" \
      '[.doc.nodes[].kind | select(. as $k | $cat | index($k) | not)] | unique | join(", ")')"
    if [ -n "$missing" ]; then
      fail "workflow **$name** uses node kinds no enabled integration has: $missing"
      INT_GAPS=$((INT_GAPS + 1))
    fi
  done < <(echo "$WORKFLOWS" | jq -r '.[] | select(.builtin == null) | [.id, .name] | @tsv')
  [ "$INT_GAPS" = 0 ] && pass "every integration a connection or workflow uses is enabled"

  section "Connections"
  while IFS=$'\t' read -r id name kind secret; do
    [ -n "$id" ] || continue
    if ! get "/connections/$id" >/dev/null; then
      fail "connection **$name** ($kind) does not load"
    elif [ "$secret" = true ]; then
      note "connection **$name** ($kind) loads; needs secret (kept in the keychain, which a rehearsal never reads)"
    else
      pass "connection **$name** ($kind) loads"
    fi
  done < <(echo "$CONNECTIONS" | jq -r '.[] | [.id, .name, .kind, .hasSecret] | @tsv')
  [ "$(echo "$CONNECTIONS" | jq length)" = 0 ] && note "no connections"
  SECRETS="$(get /secrets | jq length 2>/dev/null || echo 0)"
  [ "$SECRETS" != 0 ] && note "$SECRETS secret(s) in Settings → Secrets: needs secret (their values are in the keychain)"

  section "Workflows (validate and dry run)"
  TRIGGERS="$(echo "$CATALOG" | jq '[.[] | select(.category == "trigger") | .id]')"
  while IFS=$'\t' read -r id name enabled; do
    [ -n "$id" ] || continue
    detail="$(get "/workflows/$id")" || { fail "workflow **$name** does not load"; continue; }
    label="workflow **$name**$([ "$enabled" = true ] || echo ' (disabled)')"
    issues="$(echo "$detail" | jq '{doc}' | post /workflows/validate)" || { fail "$label: validation failed to run"; continue; }
    errors="$(echo "$issues" | jq -r '[.[] | select(.severity == "error") | .message] | join("; ")')"
    if [ -n "$errors" ]; then
      fail "$label does not validate: $errors"
      continue
    fi
    # One dry run per trigger: the newest logged event it matches (of the last 20 of its type), else a made-up one.
    results=""
    while IFS=$'\t' read -r node kind; do
      [ -n "$node" ] || continue
      made_up=0
      case "$kind" in
        schedule) reqs="$(jq -nc --arg w "$id" --arg n "$node" \
            '{workflowId: $w, event: {type: "schedule.fired", payload: {workflowId: $w, nodeId: $n}}}')" ;;
        manual) reqs="$(jq -nc --arg w "$id" '{workflowId: $w, event: {type: "manual", payload: {}}}')" ;;
        *)
          etype="$kind"
          [ "$kind" = webhook ] && etype="webhook.received"
          reqs="$(get "/events?type=$etype&limit=20" | jq -c --arg w "$id" '.[] | {workflowId: $w, eventId: .id}')"
          if [ -z "$reqs" ]; then
            made_up=1
            reqs="$(jq -nc --arg w "$id" --arg k "$etype" '{workflowId: $w, event: {type: $k, payload: {}}}')"
          fi ;;
      esac
      verdict=""
      tried=0
      while IFS= read -r req; do
        tried=$((tried + 1))
        r="$(echo "$req" | post /workflows/dry-run)" || { verdict="error: the dry run was refused: $r"; break; }
        verdict="$(echo "$r" | jq -r '
          if ([.issues[] | select(.severity == "error")] | length) > 0 then "error: " + ([.issues[].message] | join("; "))
          elif .run == null then "error: no run"
          elif ([.run.triggers[] | select(.node == $n and .matched)] | length) == 0 then
            "nomatch: " + ([.run.triggers[] | select(.node == $n) | .reason // empty] | join("; "))
          elif ([.run.steps[] | select(.error != null)] | length) > 0 then
            ([.run.steps[] | select(.error != null) | .nodeId + ": " + .error] | join("; ")) as $e
            | if ($e | test("secret"; "i")) then "secret: " + $e else "error: " + $e end
          elif .run.status == "failed" then "error: the run would fail"
          else "ok: " + (.run.status // "?") end' --arg n "$node")"
        case "$verdict" in nomatch*) ;; *) break ;; esac
      done <<<"$reqs"
      case "$made_up:$verdict" in
        # An empty made-up event lacks the fields its steps read: that says nothing about the upgrade.
        1:ok*) ;;
        1:*) verdict="untested: no $etype event is logged (an empty one gives: ${verdict#*: })" ;;
        0:nomatch*) verdict="nomatch: none of the last $tried event(s) of its type match${verdict#nomatch:}" ;;
      esac
      results+="$node → $verdict"$'\n'
    done < <(echo "$detail" | jq -r --argjson t "$TRIGGERS" \
      '.doc.nodes[] | select(.kind as $k | $t | index($k)) | [.id, .kind] | @tsv')
    if [ -z "$results" ]; then
      fail "$label validates but has no trigger to dry-run"
    elif echo "$results" | grep -q '→ error'; then
      fail "$label validates; dry run: $(echo "$results" | grep -v '^$' | paste -sd ';' - | sed 's/;/; /g')"
    elif echo "$results" | grep -q '→ secret'; then
      note "$label validates; dry run needs secret: $(echo "$results" | grep '→ secret' | paste -sd ';' -)"
    elif ! echo "$results" | grep -q '→ ok'; then
      note "$label validates; no dry run of a logged event reached its steps: $(echo "$results" | grep -v '^$' | paste -sd ';' - | sed 's/;/; /g')"
    else
      pass "$label validates; dry run: $(echo "$results" | grep -v '^$' | paste -sd ';' - | sed 's/;/; /g')"
    fi
  done < <(echo "$WORKFLOWS" | jq -r '.[] | [.id, .name, .enabled] | @tsv')

  section "Tool lists"
  ALL_TOOLS="$(get /personas/tools | jq '[.[].name]')"
  PM_TOOLS="$(get /personas | jq '[.[] | select(.builtin == "pm") | .tools[]]')"
  TOOL_GAPS=0
  while IFS=$'\t' read -r label tools pm; do
    [ -n "$label" ] || continue
    missing="$(jq -rn --argjson want "$tools" --argjson have "$ALL_TOOLS" '$want - $have | join(", ")')"
    missing_pm="$(jq -rn --argjson want "$pm" --argjson have "$PM_TOOLS" '$want - $have | join(", ")')"
    if [ -n "$missing" ]; then
      fail "$label's tools are missing from the agents' list: $missing"; TOOL_GAPS=$((TOOL_GAPS + 1))
    fi
    if [ -n "$missing_pm" ]; then
      fail "$label's PM tools are missing from the PM persona (edited by hand?): $missing_pm"; TOOL_GAPS=$((TOOL_GAPS + 1))
    fi
    [ -z "$missing$missing_pm" ] && pass "$label: $(jq -rn --argjson t "$tools" '$t | length') tool(s) for agents, $(jq -rn --argjson t "$pm" '$t | length') for the PM"
  done < <(echo "$INTEGRATIONS" | jq -r '.[] | select(.enabled) | [.label, (.tools | tojson), (.pmTools | tojson)] | @tsv')
  [ "$(echo "$INTEGRATIONS" | jq '[.[] | select(.enabled)] | length')" = 0 ] && note "no integration is enabled"
  [ "$(echo "$PM_TOOLS" | jq length)" = 0 ] && fail "there is no PM persona with tools"

  section "GitHub login"
  LOGIN="$(get /github/login)"
  if [ "$(echo "$LOGIN" | jq -r .connected)" = false ]; then
    pass "with no token, Settings → This host → GitHub asks to connect"
    if [ "$(echo "$LOGIN" | jq -r .deviceFlow)" != true ]; then
      note "Connect GitHub (the device flow) is off: no OAuth App client ID is built in, so only a pasted token signs in (docs/reference/github.md \"Registering PDT's OAuth App\")"
    fi
  else
    fail "the GitHub login reads as connected with no token set: $(echo "$LOGIN" | jq -c .)"
  fi
  if [ -n "$WEB_DIR" ]; then
    if curl -sf --max-time 10 "http://127.0.0.1:$PORT/" | grep -qi '<html'; then
      pass "the web UI is served (web/dist)"
    else
      fail "the web UI is not served from $WEB_DIR"
    fi
  else
    note "web/dist is not built, so the UI was not checked (pnpm --dir web build)"
  fi
fi

section "Result"
if [ "$FAILS" = 0 ]; then line "Every check passed."; else line "$FAILS check(s) failed."; fi

cat "$REPORT"
[ -z "$REPORT_FILE" ] || cp "$REPORT" "$REPORT_FILE"
[ "$FAILS" = 0 ]
