#!/usr/bin/env bash
# Every layer, in one command.
#
#   scripts/check.sh                 # everything that can run here
#   scripts/check.sh --fast          # skip the model and the browser
#   scripts/check.sh --with-remote   # also the pairing/TLS path (see below)
#   scripts/check.sh --only api      # one layer
#
# The layers are ordered cheapest-first, so a mistake that a two-second check
# would have caught does not wait behind a four-minute one.
#
# # Skips are reported, never silent
#
# Three layers need something this machine may not have — a model in `.env`,
# a chromium, a network this listener may be opened on. Each says so and the
# run goes on; what it does not do is quietly report a green that covered less
# than it looks like. A layer that could not run is not a layer that passed.
#
# # `--with-remote` opens a listener on this machine's network
#
# `tests/remote-smoke.mjs` is about a **reachable** backend: on loopback the
# per-process token is the whole story by design, so its rules have nothing to
# gate. Testing it means binding somewhere reachable, briefly, with TLS. That
# is off by default because it is not this script's call to make.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

FAST=0; WITH_REMOTE=0; ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --fast) FAST=1 ;;
    --with-remote) WITH_REMOTE=1 ;;
    --only) ONLY="${2:-}"; shift ;;
    -h|--help) sed -n '2,26p' "$0" | sed 's|^# \{0,1\}||'; exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
  shift
done

# Ports chosen above everything the suites use themselves (4269–4291).
PORT_UI=4360
PORT_TLS=4361

CRATES=(-p harnix -p harnix-app -p harnix-assembly -p harnix-authz -p harnix-builtin
        -p harnix-contract -p harnix-module -p harnix-hub -p harnix-transport
        -p harnix-testapp)

scratch="$(mktemp -d)"
backend_pid=""
cleanup() {
  [ -n "$backend_pid" ] && kill "$backend_pid" 2>/dev/null
  rm -rf "$scratch"
}
trap cleanup EXIT

# ── reporting ─────────────────────────────────────────────────────────────
results=()
failed=0
bold() { printf '\033[1m%s\033[0m\n' "$1"; }
record() { results+=("$1|$2|$3"); [ "$2" = "FAIL" ] && failed=$((failed + 1)); return 0; }

# Run a layer. Its output goes to a file and is shown only when it fails —
# a passing run should say one line, or nobody reads any of them.
layer() {
  local name="$1"; shift
  if [ -n "$ONLY" ] && [ "$ONLY" != "$name" ]; then return 0; fi
  local log="$scratch/$name.log"
  printf '  %-12s ' "$name"
  local started=$SECONDS
  if "$@" > "$log" 2>&1; then
    local took=$((SECONDS - started))
    printf 'ok   (%ss)\n' "$took"
    record "$name" "ok" "${took}s"
  else
    printf 'FAIL\n'
    sed 's/^/      | /' "$log" | tail -30
    record "$name" "FAIL" "see above"
  fi
}

skip() {
  local name="$1" why="$2"
  if [ -n "$ONLY" ] && [ "$ONLY" != "$name" ]; then return 0; fi
  printf '  %-12s --   %s\n' "$name" "$why"
  record "$name" "skipped" "$why"
}

has_model() { [ -f .env ] && grep -qE '^\s*(export\s+)?ANTHROPIC_(API_KEY|AUTH_TOKEN)=' .env; }
has_chromium() { [ -d "$HOME/Library/Caches/ms-playwright" -o -d "$HOME/.cache/ms-playwright" ]; }

# ── the layers ────────────────────────────────────────────────────────────
# The test application is built beside the binary: the module suites run it.
run_build()   { cargo build --release -p harnix -p harnix-testapp; }
run_rust()    { cargo test "${CRATES[@]}"; }
# `--no-deps` matters: without it `-D warnings` is applied to the Agix
# submodule too, and this script goes red for lints in code this repository
# does not own. Harnix's own crates are held to zero warnings.
run_clippy()  { cargo clippy "${CRATES[@]}" --all-targets --no-deps -- -D warnings; }

run_offline() {
  # No backend, no model, no browser: the checks that should never be the
  # slow ones.
  for t in crate-metadata app-scripts style-lint contrib-smoke reducer-smoke i18n-smoke readme-pairing; do
    echo "── $t ──"
    node "tests/$t.mjs" || return 1
  done
}

# The front-end packages, without a backend: types, unit tests, the protocol
# declarations against the Rust they are generated from, and the coverage
# table against the tests' own tags.
run_frontend() {
  [ -d node_modules ] || pnpm install --frozen-lockfile || return 1
  echo "── typecheck ──"; pnpm -r --if-present run typecheck || return 1
  echo "── unit ──";      pnpm -r --if-present run test || return 1
  echo "── protocol types ──"
  HARNIX_TS_OUT="$scratch/ts" cargo test -q -p harnix-contract --features ts --test ts_export || return 1
  diff -u ui/packages/core/src/protocol/generated/contract.ts "$scratch/ts/contract.ts" || {
    echo "generated protocol types are stale: cargo test -p harnix-contract --features ts"; return 1; }
  echo "── coverage table ──"; node tests/live/coverage.mjs --check || return 1
}

# `templates/app`, copied out and built offline as a fresh application would
# be. Its own target directory, kept between runs, so only the first is slow.
run_template() { HARNIX_TEMPLATE_TARGET="$root/target/template" node tests/app-template.mjs; }

# The LIVE suites (tests/live, Playwright Test). The binary was built by the
# first layer; `HARNIX_BIN` stops each run from asking cargo again.
live() { HARNIX_BIN="$root/target/release/harnix" node tests/live/run.mjs "$@"; }

# No flag is the replayed mode. `--no-model` is a third thing — neither a
# provider nor a fixture — under which four suites skip themselves, so a run
# that used it looked like the deterministic one and covered a good deal less.
run_api() {
  for topology in single_duplex split_streams; do
    echo "── replayed · $topology ──"
    live --project api --topology "$topology" || return 1
  done
}

run_live() {
  for topology in single_duplex split_streams; do
    echo "── live · $topology ──"
    live --project api --live --topology "$topology" || return 1
  done
}

run_browser() {
  live --project ui || return 1
  node tests/package-e2e.mjs || return 1
}

# One backend, configured for all three of the tests that need an external
# one: `--scenes chat` for the interface smokes, both topologies for parity.
start_backend() {
  cat > "$scratch/profile.toml" <<EOF
[transport]
topologies = ["single_duplex", "split_streams"]
host = "127.0.0.1"
port = $PORT_UI
EOF
  set -a; . ./.env; set +a
  target/release/harnix --profile "$scratch/profile.toml" --scenes chat \
    --ui-dir ui --agix-dir "$scratch/atta" --data-dir "$scratch/projects" \
    > "$scratch/backend.log" 2>&1 &
  backend_pid=$!
  for _ in $(seq 200); do
    curl -sf --noproxy '*' "http://127.0.0.1:$PORT_UI/" >/dev/null && return 0
    sleep 0.25
  done
  echo "the backend did not start"; cat "$scratch/backend.log"; return 1
}

run_interface() {
  start_backend || return 1
  local token; token="$(cat "$scratch/projects/.harnix/token")"
  echo "── topology-parity ──"; node tests/topology-parity.mjs "$PORT_UI" "$token" || return 1
  echo "── tool-smoke ──";      node tests/tool-smoke.mjs "$PORT_UI" "$token" || return 1
  echo "── ui-smoke ──";        node tests/ui-smoke.mjs "$PORT_UI" "$token" || return 1
  kill "$backend_pid" 2>/dev/null; backend_pid=""
}

run_remote() {
  # Bound to 0.0.0.0 rather than an interface address: the test connects to
  # 127.0.0.1, and admission has to see a non-loopback bind. Both are true of
  # `0.0.0.0`, and nothing else satisfies them at once.
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "$scratch/key.pem" -out "$scratch/cert.pem" \
    -subj "/CN=127.0.0.1" -addext "subjectAltName=IP:127.0.0.1" 2>/dev/null || return 1
  cat > "$scratch/remote.toml" <<EOF
[transport]
topologies = ["single_duplex", "split_streams"]
host = "0.0.0.0"
port = $PORT_TLS
EOF
  set -a; . ./.env; set +a
  target/release/harnix --profile "$scratch/remote.toml" \
    --tls-cert "$scratch/cert.pem" --tls-key "$scratch/key.pem" \
    --ui-dir ui --agix-dir "$scratch/remote-atta" --data-dir "$scratch/remote-projects" \
    > "$scratch/remote.log" 2>&1 &
  backend_pid=$!
  for _ in $(seq 200); do
    curl -skf "https://127.0.0.1:$PORT_TLS/" >/dev/null && break
    sleep 0.25
  done
  # A reachable listener prints a one-time pairing code at startup: the only
  # thing that breaks the circle of "pairing is a method and methods need
  # admission".
  local code token
  code="$(grep -oE 'pairing code → [A-Z0-9]+' "$scratch/remote.log" | sed 's/.*→ //')"
  token="$(cat "$scratch/remote-projects/.harnix/token")"
  [ -n "$code" ] || { echo "no pairing code was printed"; cat "$scratch/remote.log"; return 1; }
  NODE_TLS_REJECT_UNAUTHORIZED=0 node tests/remote-smoke.mjs "$PORT_TLS" "$token" "$code" || return 1
  kill "$backend_pid" 2>/dev/null; backend_pid=""
}

# The budget measures latency and memory, so it goes last and alone —
# anything else running makes p99 a measurement of the other thing.
run_budget() { node tests/budget.mjs; }

# ── run ───────────────────────────────────────────────────────────────────
bold "Harnix — every layer"
echo

layer build  run_build
layer rust   run_rust
layer clippy run_clippy
layer offline run_offline
layer frontend run_frontend
layer api    run_api

if [ "$FAST" = 1 ]; then
  skip template "--fast"
  skip live "--fast"
  skip browser "--fast"
  skip interface "--fast"
elif ! has_model; then
  layer template run_template
  skip live "no ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN in .env"
  skip interface "needs a model — see above"
  if has_chromium; then layer browser run_browser; else skip browser "no chromium — pnpm install:browser"; fi
else
  layer template run_template
  layer live run_live
  if has_chromium; then layer browser run_browser; else skip browser "no chromium — pnpm install:browser"; fi
  layer interface run_interface
fi

if [ "$WITH_REMOTE" = 1 ]; then
  if has_model; then layer remote run_remote; else skip remote "needs a model to start a backend"; fi
else
  skip remote "not asked for — pass --with-remote (opens a listener on this network)"
fi

layer budget run_budget

# ── summary ───────────────────────────────────────────────────────────────
echo
bold "summary"
for row in "${results[@]}"; do
  IFS='|' read -r name status detail <<< "$row"
  case "$status" in
    ok)      printf '  \033[32m%-12s ok\033[0m   %s\n' "$name" "$detail" ;;
    FAIL)    printf '  \033[31m%-12s FAIL\033[0m %s\n' "$name" "$detail" ;;
    skipped) printf '  \033[33m%-12s --\033[0m   %s\n' "$name" "$detail" ;;
  esac
done
echo
if [ "$failed" -gt 0 ]; then
  bold "$failed layer(s) failed"
  exit 1
fi
skipped=$(printf '%s\n' "${results[@]}" | grep -c '|skipped|')
if [ "$skipped" -gt 0 ]; then
  bold "all $((${#results[@]} - skipped)) layer(s) that ran passed; $skipped did not run"
else
  bold "all ${#results[@]} layer(s) passed"
fi
