#!/usr/bin/env bash
# Build the application. The checks first: a drifted dependency block or a
# pin nobody can fetch is cheaper to hear about before the build than after.
set -euo pipefail
cd "$(dirname "$0")/.."
node harnix/scripts/sync-app-deps.mjs . --check
node harnix/scripts/check-pins.mjs . "$@"
cargo build --release -p myapp
node harnix/scripts/check-lock.mjs .
