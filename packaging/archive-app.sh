#!/bin/bash
# Makes the release archive and its checksum, and proves the signature survives the round trip.
# Usage: packaging/archive-app.sh <MacFlayer.app> <output directory>
set -euo pipefail

app="$1"
out="$2"
name=MacFlayer-macos-arm64.zip

mkdir -p "$out"
ditto -c -k --keepParent "$app" "$out/$name"
(cd "$out" && shasum -a 256 "$name" > "$name.sha256")

extracted=$(mktemp -d)
trap 'rm -rf "$extracted"' EXIT
ditto -x -k "$out/$name" "$extracted"
codesign --verify --deep --strict "$extracted/MacFlayer.app"

cat "$out/$name.sha256"
