#!/usr/bin/env bash
# Drop the build caches that only ever grow.
#
#   scripts/prune.sh          # incremental caches everywhere; a whole target past its limit
#   scripts/prune.sh --all    # `cargo clean` in every workspace here
#
# Cargo writes a new artifact whenever a crate's source, features or flags change and never
# removes the old one, and each integration test is a binary linking all of Agix — so a target
# directory that is never cleaned grows by tens of gigabytes over a milestone. Nothing here is
# needed for correctness: the next build rebuilds what it is missing, only more slowly.
#
# The workspaces: this one (target/, with the template's build under target/template), Agix's own
# (agix/target, when Agix is built on its own) and the example application's.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIMIT_GB="${HARNIX_TARGET_LIMIT_GB:-25}"
ALL=0
[ "${1:-}" = "--all" ] && ALL=1

free() { df -Ph "$root" | awk 'NR==2 {print $4}'; }
echo "free before: $(free)"
for dir in "$root" "$root/agix" "$root/examples/hello-app"; do
  target="$dir/target"
  [ -d "$target" ] || continue
  gb=$(( $(du -sk "$target" | awk '{print $1}') / 1048576 ))
  if [ "$ALL" = 1 ] || [ "$gb" -ge "$LIMIT_GB" ]; then
    echo "${target#$root/}: ${gb} GB — cargo clean"
    (cd "$dir" && cargo clean --quiet)
  else
    echo "${target#$root/}: ${gb} GB — incremental caches only"
    find "$target" -maxdepth 3 -type d -name incremental -prune -exec rm -rf {} +
  fi
done
echo "free after: $(free)"
