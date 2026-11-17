#!/usr/bin/env bash
# The flow library builds and passes its tests copied out of this workspace, as another host takes
# it in (crates/flow/docs/embed.en.md): `crates/flow` alone in a fresh workspace, with no
# Cargo.lock, so its dependencies resolve from its own manifest the way a stranger's would.
#
#   scripts/check-flow-copy.sh
#
# Offline when cargo is (`CARGO_NET_OFFLINE=true`): the versions then come from what is already
# downloaded. Its target directory is a temporary one, removed with the copy; `CARGO_TARGET_DIR`
# keeps one between runs instead.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

cp -R "$root/crates/flow" "$scratch/flow"
rm -rf "$scratch/flow/target"
cat > "$scratch/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["flow"]

[profile.dev]
debug = "line-tables-only"
EOF

cd "$scratch"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$scratch/target}"
cargo check --tests --examples
# Its own guards included: what it links and what it offers are checked from the copy too.
cargo test
