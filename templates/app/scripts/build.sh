#!/usr/bin/env bash
# Build the application. The checks first: a drifted dependency block or a
# pin nobody can fetch is cheaper to hear about before the build than after.
set -euo pipefail
cd "$(dirname "$0")/.."
node harnix/scripts/sync-app-deps.mjs . --check
node harnix/scripts/check-pins.mjs . "$@"
# The interface: the binary embeds Harnix's built `ui/dist` and will not build
# without it. (From M5 an application's spaces join this build.)
pnpm --dir harnix install --frozen-lockfile
pnpm --dir harnix --filter @harnix/shell build
cargo build --release -p myapp
node harnix/scripts/check-lock.mjs .
