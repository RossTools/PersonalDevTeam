#!/usr/bin/env bash
# Tests for scripts/scratch-daemon.sh, with a stand-in pdtd (python's http.server) and a scratch TMPDIR.
# Run: scripts/scratch-daemon.test.sh (CI runs it in the `changelog` job).
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/scratch-daemon.sh"
scratch="$(mktemp -d)"
started=""
cleanup() {
  for p in $started; do "$script" stop "$p" >/dev/null 2>&1 || true; done
  rm -rf "$scratch"
}
trap cleanup EXIT
failed=0
fail() { printf 'FAIL: %s\n' "$*" >&2; failed=1; }

export TMPDIR="$scratch/tmp"
mkdir -p "$TMPDIR"
parent="$(cd "$TMPDIR" && pwd -P)/pdt-scratch"

# A stand-in pdtd: answers /api/health on PDT_PORT, from a folder in its data dir.
fake="$scratch/fake-pdtd"
cat >"$fake" <<'EOF'
#!/usr/bin/env bash
mkdir -p "$PDT_DATA_DIR/www/api" && echo ok >"$PDT_DATA_DIR/www/api/health"
cd "$PDT_DATA_DIR/www" && exec python3 -m http.server "$PDT_PORT" --bind 127.0.0.1
EOF
chmod +x "$fake"
export PDT_SCRATCH_BIN="$fake"

# Something outside the parent that must survive every refusal.
outside="$scratch/outside"
mkdir -p "$outside" && echo keep >"$outside/file"

refuses() { # name, args...
  local name="$1"; shift
  if "$script" stop "$@" >/dev/null 2>&1; then fail "stop refuses $name"; fi
}
refuses "a path" "$outside"
refuses "a relative path" ../outside
refuses "a parent escape" ..
refuses "an empty argument" ""
refuses "a glob" '*'
refuses "a port with no scratch folder" 7799
mkdir -p "$parent"
ln -s "$outside" "$parent/7798"
refuses "a link out of the parent" 7798
[ -f "$outside/file" ] || fail "nothing outside the parent is deleted"
[ -L "$parent/7798" ] && rm "$parent/7798"
if "$script" stop >/dev/null 2>&1; then fail "stop needs a port"; fi
if "$script" stop 1 2 >/dev/null 2>&1; then fail "stop takes one port"; fi

# start → a port, a folder under the parent, a daemon answering; stop → both gone.
out="$("$script" start 2>/dev/null)" || fail "start"
port="$(printf '%s\n' "$out" | sed -n 's/^port=//p')"
dir="$(printf '%s\n' "$out" | sed -n 's/^dir=//p')"
started="$port"
[ "$dir" = "$parent/$port" ] || fail "start's folder is <parent>/<port> (got '$dir')"
[ -d "$dir" ] || fail "start makes the folder"
curl -sf "http://127.0.0.1:$port/api/health" >/dev/null || fail "the daemon answers"
"$script" list | grep -q "^$port	up	" || fail "list shows it up"
"$script" stop "$port" >/dev/null || fail "stop"
started=""
[ -d "$dir" ] && fail "stop deletes the folder"
if curl -sf "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then fail "stop kills the daemon"; fi

# start prunes folders older than a day whose daemon is gone, and keeps newer ones.
mkdir -p "$parent/7790" "$parent/7791"
touch -t 202001010000 "$parent/7790"
out="$("$script" start 2>/dev/null)" || fail "second start"
started="$(printf '%s\n' "$out" | sed -n 's/^port=//p')"
[ -d "$parent/7790" ] && fail "start prunes a day-old folder"
[ -d "$parent/7791" ] || fail "start keeps a new folder"
"$script" stop "$started" >/dev/null || fail "stop the second"
started=""
"$script" stop 7791 >/dev/null || fail "stop a folder with no daemon"

[ -f "$outside/file" ] || fail "nothing outside the parent is deleted"
if [ "$failed" -ne 0 ]; then echo "scratch-daemon tests failed" >&2; exit 1; fi
echo "scratch-daemon tests passed"
