#!/usr/bin/env bash
# Set every Harnix crate's version, then bring Cargo.lock along.
#
#   scripts/set-version.sh 0.3.2-2026102601
#
# The crates state their versions themselves rather than inheriting one from
# the workspace (see the note in Cargo.toml), so a release changes them all
# here, in one place.
set -euo pipefail
version="${1:?usage: scripts/set-version.sh <x.y.z-YYYYMMDDNN>}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9]{10}$ ]] || {
  echo "not a version of the form x.y.z-YYYYMMDDNN: $version" >&2
  exit 2
}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for manifest in "$root"/crates/*/Cargo.toml; do
  perl -pi -e "s/^version(\s*)= \"[^\"]*\"/version\$1= \"$version\"/" "$manifest"
done
(cd "$root" && cargo update -w)
echo "every crate is now $version"
