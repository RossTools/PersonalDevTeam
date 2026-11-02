#!/usr/bin/env bash
# A throwaway dev pdtd for an agent to look at (screenshots, Playwright), started and stopped without commands an
# agent's approval prompt can't judge (`rm -rf "$(cat …)"`). Each one gets a free port and a scratch folder
# `<temp dir>/pdt-scratch/<port>`; `stop` deletes only folders directly under that parent and refuses anything else.
# Windows twin: scripts/scratch-daemon.ps1. Tests: scripts/scratch-daemon.test.sh.
#
#   scripts/scratch-daemon.sh start [--seed <seed>]   start one; prints port=, dir=, url=
#   scripts/scratch-daemon.sh pair <port>             a pairing link (good once, for 5 minutes): open it first in the
#                                                     test browser, or the app shows only "Pair this browser"
#   scripts/scratch-daemon.sh stop <port>             stop it by port and delete its scratch folder
#   scripts/scratch-daemon.sh list                    the scratch folders and whether each daemon is up
#
# Run from the checkout (it starts target/debug/pdtd and serves web/dist; build them first). Set PDT_SCRATCH_BIN for
# another pdtd. PDT_CLAUDE_PROGRAM / PDT_CODEX_PROGRAM / PDT_DEVIN_PROGRAM pass through, to avoid the real agents.
# `start` also removes scratch folders older than a day whose daemon is gone.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="${TMPDIR:-/tmp}"
mkdir -p "${tmp%/}/pdt-scratch"
parent="$(cd "${tmp%/}/pdt-scratch" && pwd -P)"
# Ports it picks from: clear of the stable (7420) and dev (7421) instances and the pdt run's usual ports.
first_port=7600
last_port=7799

die() { printf 'scratch-daemon: %s\n' "$*" >&2; exit 1; }
listening() { lsof -nP -ti "tcp:$1" -sTCP:LISTEN 2>/dev/null; }

# The scratch folder for a port, or a refusal: only `<parent>/<digits>`, a real folder (not a link) inside parent.
scratch_dir() {
  local port="$1"
  [[ "$port" =~ ^[0-9]+$ ]] || die "not a port: '$port' (stop takes the port start printed)"
  local dir="$parent/$port"
  [ -L "$dir" ] && die "refusing $dir: a link, not a scratch folder"
  [ -d "$dir" ] || die "no scratch folder for port $port under $parent"
  local real
  real="$(cd "$dir" && pwd -P)"
  [ "$real" = "$parent/$port" ] || die "refusing $dir: it resolves to $real, outside $parent"
  printf '%s\n' "$dir"
}

stop_port() {
  local port="$1" dir pids
  dir="$(scratch_dir "$port")" || exit 1
  [ -n "$dir" ] || exit 1
  pids="$(listening "$port" || true)"
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086 # one pid per word
    kill $pids 2>/dev/null || true
    for _ in $(seq 1 50); do
      [ -z "$(listening "$port" || true)" ] && break
      sleep 0.2
    done
    pids="$(listening "$port" || true)"
    # shellcheck disable=SC2086
    [ -n "$pids" ] && kill -9 $pids 2>/dev/null || true
  fi
  rm -rf -- "$dir"
  printf 'stopped port %s, deleted %s\n' "$port" "$dir"
}

# Folders older than a day whose daemon is gone (a crashed agent's leftovers).
prune() {
  local dir port
  while IFS= read -r dir; do
    port="$(basename "$dir")"
    [[ "$port" =~ ^[0-9]+$ ]] || continue
    [ -n "$(listening "$port" || true)" ] && continue
    (stop_port "$port") >/dev/null 2>&1 || true
  done < <(find "$parent" -mindepth 1 -maxdepth 1 -type d -mtime +0 2>/dev/null)
}

start() {
  local seed=demo
  while [ $# -gt 0 ]; do
    case "$1" in
      --seed) seed="${2?--seed needs a value}"; shift 2 ;;
      *) die "unknown option for start: $1" ;;
    esac
  done
  local bin="${PDT_SCRATCH_BIN:-$root/target/debug/pdtd}"
  [ -x "$bin" ] || die "no pdtd at $bin: build it first (cargo build -p pdtd)"
  prune
  local port="" p
  for p in $(seq "$first_port" "$last_port"); do
    if [ -z "$(listening "$p" || true)" ] && mkdir "$parent/$p" 2>/dev/null; then port="$p"; break; fi
  done
  [ -n "$port" ] || die "no free port in $first_port-$last_port"
  local dir="$parent/$port"
  PDT_PROFILE=dev PDT_PORT="$port" PDT_DATA_DIR="$dir/data" PDT_WEB_DIR="$root/web/dist" PDT_SECRET_STORE=memory \
    PDT_SEED="$seed" PDT_TAILSCALE_BIN=/nonexistent PDT_GITHUB_OFFLINE=1 PDT_BROWSER="$root/scripts/scratch-browser.sh" \
    nohup "$bin" >"$dir/pdtd.out" 2>&1 </dev/null &
  local url="http://127.0.0.1:$port"
  for _ in $(seq 1 150); do
    if curl -sf "$url/api/health" >/dev/null 2>&1; then
      printf 'port=%s\ndir=%s\nurl=%s\n' "$port" "$dir" "$url"
      printf 'stop it with: scripts/scratch-daemon.sh stop %s\n' "$port" >&2
      return 0
    fi
    sleep 0.2
  done
  printf 'scratch-daemon: pdtd did not answer on %s; its output:\n' "$url" >&2
  tail -n 20 "$dir/pdtd.out" >&2 || true
  (stop_port "$port") >/dev/null 2>&1 || true
  exit 1
}

# A pairing link: the daemon "opens" it with scratch-browser.sh, which leaves it in the scratch folder.
pair() {
  local port="$1" dir
  dir="$(scratch_dir "$port")" || exit 1
  rm -f -- "$dir/pair-url"
  # pdtd opens one browser every few seconds at most: a second pair soon after the first waits its turn.
  local tries=0
  until curl -sf -X POST "http://127.0.0.1:$port/api/ui/open" >/dev/null; do
    tries=$((tries + 1))
    [ "$tries" -lt 5 ] || die "pdtd on port $port opened no pairing link"
    sleep 1
  done
  for _ in $(seq 1 25); do
    [ -s "$dir/pair-url" ] && { printf 'pair=%s\n' "$(cat "$dir/pair-url")"; return 0; }
    sleep 0.2
  done
  die "no pairing link from pdtd on port $port"
}

list() {
  local dir port state
  for dir in "$parent"/*; do
    [ -d "$dir" ] || continue
    port="$(basename "$dir")"
    state=down
    [ -n "$(listening "$port" || true)" ] && state=up
    printf '%s\t%s\t%s\n' "$port" "$state" "$dir"
  done
}

case "${1:-}" in
  start) shift; start "$@" ;;
  pair) [ $# -eq 2 ] || die "usage: scratch-daemon.sh pair <port>"; pair "$2" ;;
  stop) [ $# -eq 2 ] || die "usage: scratch-daemon.sh stop <port>"; stop_port "$2" ;;
  list) list ;;
  *) die "usage: scratch-daemon.sh start [--seed <seed>] | pair <port> | stop <port> | list" ;;
esac
