#!/usr/bin/env bash
# Build the application. The checks first: a drifted dependency block or a
# pin nobody can fetch is cheaper to hear about before the build than after.
set -euo pipefail
cd "$(dirname "$0")/.."
node harnix/scripts/sync-app-deps.mjs . --check
node harnix/scripts/check-pins.mjs . "$@"
# The interface, with every space's interface entry compiled in
# (`spaces/<x>/ui/src/index.tsx`, in name order), into `target/ui`, which the
# binary then embeds. It will not build without one.
app="$PWD"
modules="$(ls "$app"/spaces/*/ui/src/index.tsx 2>/dev/null | paste -sd, - || true)"
pnpm --dir harnix install --frozen-lockfile
# The app's own workspace (spaces/*/ui, tests), when it has one: the shell build resolves the
# spaces' imports, third-party ones included, from there.
if [ -f pnpm-workspace.yaml ]; then pnpm install --frozen-lockfile; fi
HARNIX_UI_MODULES="$modules" HARNIX_UI_OUT="$app/target/ui" \
  pnpm --dir harnix --filter @harnix/shell build
HARNIX_UI_DIST="$app/target/ui" cargo build --release -p myapp
node harnix/scripts/check-lock.mjs .
