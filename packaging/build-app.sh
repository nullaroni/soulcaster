#!/bin/bash
# Builds target/package/Soulcaster.app from app/ and the soulcaster CLI, ad-hoc signed, and
# checks the result. Needs Rust and Command Line Tools. Installs nothing.
# Usage: packaging/build-app.sh
set -euo pipefail

cd "$(dirname "$0")/.."

rust_target=aarch64-apple-darwin
app=target/package/Soulcaster.app
contents="$app/Contents"
version=$(packaging/version.sh)
minimum_macos=$(plutil -extract LSMinimumSystemVersion raw packaging/Info.plist)

MACOSX_DEPLOYMENT_TARGET="$minimum_macos" cargo build --release --locked \
  --target "$rust_target" -p soulcaster-app -p soulcaster-cli -p soulcaster-engine

rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Helpers" "$contents/Resources/licenses"

# The CLI sits in Helpers because Contents/MacOS/Soulcaster and Contents/MacOS/soulcaster would be
# the same file on a case-insensitive volume.
cp "target/$rust_target/release/soulcaster-app" "$contents/MacOS/Soulcaster"
cp "target/$rust_target/release/soulcaster" "$contents/Helpers/soulcaster"
cp "target/$rust_target/release/soulcaster-askpass" "$contents/Helpers/soulcaster-askpass"

cp packaging/Info.plist "$contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"

iconset=$(mktemp -d)/Soulcaster.iconset
mkdir "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" packaging/icon/icon-1024.png \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" packaging/icon/icon-1024.png \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$contents/Resources/Soulcaster.icns" "$iconset"
rm -rf "$(dirname "$iconset")"

# The fonts and data are compiled into the binary, so only the fonts' licenses travel as files.
cp app/assets/fonts/*-OFL.txt "$contents/Resources/licenses/"

packaging/sign-app.sh - "$app"
packaging/check-app.sh "$app"

echo
echo "Built $app ($version), ad-hoc signed. Open it with: open $app"
