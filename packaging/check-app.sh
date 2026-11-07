#!/bin/bash
# Checks a built Soulcaster.app: structure, Info.plist, architecture, linked libraries and
# signature. Reads only. Usage: packaging/check-app.sh <Soulcaster.app>
set -euo pipefail

app="$1"
root=$(cd "$(dirname "$0")/.." && pwd)
contents="$app/Contents"
binaries=("$contents/MacOS/Soulcaster" "$contents/Helpers/soulcaster" "$contents/Helpers/soulcaster-askpass")
failed=0

check() {
  if [ "$2" = "$3" ]; then
    echo "ok    $1: $2"
  else
    echo "FAIL  $1: expected '$3', got '$2'"
    failed=1
  fi
}

plist_value() {
  plutil -extract "$1" raw "$contents/Info.plist"
}

for path in "$contents/Info.plist" "$contents/PkgInfo" "${binaries[@]}" \
  "$contents/Resources/Soulcaster.icns" \
  "$contents/Resources/licenses/IBMPlexMono-OFL.txt" \
  "$contents/Resources/licenses/PlayfairDisplay-OFL.txt"; do
  if [ -f "$path" ]; then
    echo "ok    present: ${path#"$app"/}"
  else
    echo "FAIL  missing: ${path#"$app"/}"
    failed=1
  fi
done
if [ "$failed" -ne 0 ]; then
  exit 1
fi

plutil -lint "$contents/Info.plist"
check CFBundleExecutable "$(plist_value CFBundleExecutable)" Soulcaster
check CFBundleIdentifier "$(plist_value CFBundleIdentifier)" \
  "$(plutil -extract CFBundleIdentifier raw "$root/packaging/Info.plist")"
check CFBundleShortVersionString "$(plist_value CFBundleShortVersionString)" \
  "$("$root/packaging/version.sh")"
check CFBundleVersion "$(plist_value CFBundleVersion)" "$("$root/packaging/version.sh")"

minimum_macos=$(plist_value LSMinimumSystemVersion)
for binary in "${binaries[@]}"; do
  name=${binary#"$app"/}
  check "$name architecture" "$(lipo -archs "$binary")" arm64
  check "$name minimum macOS" \
    "$(vtool -show-build "$binary" | sed -n 's/^ *minos //p')" "$minimum_macos"
  # A bare Mac has only the system's libraries.
  outside=$(otool -L "$binary" | sed 1d | grep -vE '^[[:space:]]+(/usr/lib/|/System/Library/)' || true)
  check "$name libraries outside the system" "$outside" ""
done

check "soulcaster validate" \
  "$("$contents/Helpers/soulcaster" validate "$root/schema/examples/developer.json" >/dev/null && echo runs)" runs

if codesign --verify --deep --strict --verbose=2 "$app"; then
  echo "ok    codesign --verify --deep --strict"
else
  echo "FAIL  codesign --verify --deep --strict"
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  exit 1
fi
