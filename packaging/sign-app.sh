#!/bin/bash
# Signs a built Soulcaster.app from the inside out, without --deep, as Apple asks.
# Usage: packaging/sign-app.sh <identity> <Soulcaster.app>
# An identity of "-" is ad-hoc: fine for a contributor's own build, never for a release.
set -euo pipefail

identity="$1"
app="$2"

if [ "$identity" = "-" ]; then
  flags=(--timestamp=none)
else
  flags=(--timestamp --options runtime)
fi

bundle_id=$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")

codesign --force --sign "$identity" "${flags[@]}" --identifier "$bundle_id.cli" \
  "$app/Contents/Helpers/soulcaster"
codesign --force --sign "$identity" "${flags[@]}" "$app"
