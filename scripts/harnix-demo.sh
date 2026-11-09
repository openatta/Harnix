#!/usr/bin/env bash
# Run the Harnix app by hand, in a directory of its own.
#
#   scripts/harnix-demo.sh start | stop | restart | status
#
# Everything it keeps — program state, the engine directory, projects, the log and the pid — is
# under $HARNIX_DEMO_DIR (default ~/harnix-demo), so it never touches ~/.harnix or ~/.agix; delete
# that directory to start over. The model comes from `.env` when there is one (it is read into the
# environment, not written into any settings); without it, configure one on the page.
#
# `start` builds first when there is no binary, or when the interface or the Rust sources are newer
# than it (`pnpm --filter @harnix/shell build`, then `cargo build --release -p harnix-cli`).
#
# The first `start` in a fresh directory brings in demo data (xops-03 §7, xops-04 §8): the pack of
# `scripts/demo/` (flows, skills, rules, policies, gates, hooks) into the flow module's writable
# packs; a few runs in different states — finished, waiting for a confirmation, paused, cancelled,
# waiting for an external hand-in — made by the real kernel (`crates/flow/examples/demo_seed.rs`);
# and conversations in each flow's conversation, the general one and each run's main session,
# said once to the application under --replay-dir (`scripts/demo/conversations.json`). None has
# work in hand, so nothing calls a model until you act. The Harnix app ships none of it.
#
#   HARNIX_DEMO_DIR    where it keeps everything (default ~/harnix-demo)
#   HARNIX_DEMO_PORT   the port (default 4080)
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

DIR="${HARNIX_DEMO_DIR:-$HOME/harnix-demo}"
PORT="${HARNIX_DEMO_PORT:-4080}"
BIN="$root/target/release/harnix"
PIDFILE="$DIR/harnix.pid"
LOG="$DIR/harnix.log"
URL="http://127.0.0.1:$PORT/"

running() {
  [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null
}

stale() {
  [ -x "$BIN" ] || return 0
  [ -n "$(find crates ui/packages Cargo.toml Cargo.lock \
      \( -name node_modules -o -name dist -o -name target \) -prune -o \
      -type f -newer "$BIN" -print -quit)" ]
}

build() {
  echo "building (the interface, then the binary) …"
  [ -d node_modules ] || pnpm install --frozen-lockfile
  pnpm --filter @harnix/shell build
  cargo build --release -p harnix-cli
}

# The flow module's data directory in the demo's state.
FLOW_DATA="$DIR/state/modules/flow"

seed() {
  [ -f "$DIR/.seeded" ] && return 0
  echo "bringing in the demo flows and runs …"
  mkdir -p "$FLOW_DATA/packs" "$FLOW_DATA/runs"
  # The pack: flows, skills, rules, policies, gates, hooks.
  for kind in flows skills rules policies gates hooks; do
    [ -d "scripts/demo/$kind" ] && mkdir -p "$FLOW_DATA/packs/$kind" && cp -R "scripts/demo/$kind/." "$FLOW_DATA/packs/$kind/"
  done
  cargo run --quiet --release -p harnix-flow --example demo_seed -- scripts/demo "$FLOW_DATA/runs"
  talk
  touch "$DIR/.seeded"
}

# The conversations (xops-04 §8): the application once under --replay-dir, the demo's script said
# to it (each flow's conversation, the general one, each run's main session), then stopped. The
# sessions are real; only the model's answers come from the recordings made from the same script.
talk() {
  echo "bringing in the demo conversations …"
  python3 scripts/demo/recordings.py scripts/demo/conversations.json \
    tests/live/fixtures/recordings/flow-main "$FLOW_DATA/workspaces" "$DIR/recordings" >/dev/null
  # Run from the demo's directory: a session without a project of its own works where the
  # process does, and should see the demo's files, not this repository.
  (cd "$DIR" && HARNIX_STATE_DIR="$DIR/state" exec "$BIN" \
    --port "$PORT" \
    --agix-dir "$DIR/agix" \
    --data-dir "$DIR/projects" \
    --replay-dir "$DIR/recordings") \
    >"$DIR/seed.log" 2>&1 &
  local pid=$!
  local up=""
  for _ in $(seq 1 60); do
    if curl --noproxy "*" -fsS -o /dev/null "$URL" 2>/dev/null; then up=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.5
  done
  if [ -n "$up" ]; then
    node scripts/demo/talk.mjs "$URL" "$DIR/state/token" scripts/demo/conversations.json \
      || echo "some conversations were not made; see $DIR/seed.log" >&2
  else
    echo "the application did not start for the conversations; see $DIR/seed.log" >&2
  fi
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

start() {
  if running; then
    echo "already running (pid $(cat "$PIDFILE")) → $URL"
    return 0
  fi
  if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port $PORT is taken by another process; set HARNIX_DEMO_PORT" >&2
    exit 1
  fi
  if stale; then build; fi

  mkdir -p "$DIR/state" "$DIR/agix" "$DIR/projects"
  seed
  if [ -f .env ]; then
    set -a; . ./.env; set +a
  fi
  (cd "$DIR" && HARNIX_STATE_DIR="$DIR/state" exec nohup "$BIN" \
    --port "$PORT" \
    --agix-dir "$DIR/agix" \
    --data-dir "$DIR/projects") \
    >"$LOG" 2>&1 &
  echo $! >"$PIDFILE"

  for _ in $(seq 1 60); do
    if ! running; then
      rm -f "$PIDFILE"
      echo "it exited while starting; the end of $LOG:" >&2
      tail -n 30 "$LOG" >&2
      exit 1
    fi
    if curl --noproxy "*" -fsS -o /dev/null "$URL" 2>/dev/null; then
      echo "running (pid $(cat "$PIDFILE")) → $URL"
      echo "log: $LOG"
      return 0
    fi
    sleep 0.5
  done
  echo "started (pid $(cat "$PIDFILE")) but $URL does not answer yet; see $LOG" >&2
}

stop() {
  if ! running; then
    rm -f "$PIDFILE"
    echo "not running"
    return 0
  fi
  local pid; pid="$(cat "$PIDFILE")"
  kill "$pid"
  for _ in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.5
  done
  if kill -0 "$pid" 2>/dev/null; then
    echo "did not stop in 10s; killing"
    kill -9 "$pid"
  fi
  rm -f "$PIDFILE"
  echo "stopped"
}

status() {
  if running; then
    echo "running (pid $(cat "$PIDFILE")) → $URL"
  else
    echo "not running"
  fi
  echo "dir: $DIR"
  echo "log: $LOG"
  if stale; then echo "the binary is older than the sources: the next start rebuilds"; fi
}

case "${1:-}" in
  start)   start ;;
  stop)    stop ;;
  restart) stop; start ;;
  status)  status ;;
  *) echo "usage: scripts/harnix-demo.sh start|stop|restart|status" >&2; exit 2 ;;
esac
