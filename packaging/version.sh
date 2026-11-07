#!/bin/bash
# Prints the version. The only place it is set is [workspace.package] in Cargo.toml.
set -euo pipefail

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$(dirname "$0")/../Cargo.toml")
if [ -z "$version" ]; then
  echo "version.sh: no version in Cargo.toml" >&2
  exit 1
fi
printf '%s\n' "$version"
