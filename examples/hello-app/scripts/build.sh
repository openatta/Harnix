#!/usr/bin/env bash
# Build hello-app as an application is built (design §11.4): the checks, then the interface with
# this application's modules compiled in, then the binary embedding that interface.
set -euo pipefail
cd "$(dirname "$0")/.."
app="$PWD"
node harnix/scripts/sync-app-deps.mjs . --check
pnpm --dir harnix install --frozen-lockfile
HARNIX_UI_MODULES="$app/ui/src/index.tsx" HARNIX_UI_OUT="$app/target/ui" \
  pnpm --dir harnix --filter @harnix/shell build
HARNIX_UI_DIST="$app/target/ui" cargo build --release -p hello-app "$@"
