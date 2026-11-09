#!/bin/bash
# Builds target/package/MacFlayer.app from app/ and the macflayer CLI, ad-hoc signed, and
# checks the result. Needs Rust and Command Line Tools. Installs nothing.
# Usage: packaging/build-app.sh
set -euo pipefail

cd "$(dirname "$0")/.."

rust_target=aarch64-apple-darwin
app=target/package/MacFlayer.app
contents="$app/Contents"
version=$(packaging/version.sh)
minimum_macos=$(plutil -extract LSMinimumSystemVersion raw packaging/Info.plist)

MACOSX_DEPLOYMENT_TARGET="$minimum_macos" cargo build --release --locked \
  --target "$rust_target" -p macflayer-app -p macflayer-cli -p macflayer-engine

rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Helpers" "$contents/Resources/licenses"

# The CLI sits in Helpers because Contents/MacOS/MacFlayer and Contents/MacOS/macflayer would be
# the same file on a case-insensitive volume.
cp "target/$rust_target/release/macflayer-app" "$contents/MacOS/MacFlayer"
cp "target/$rust_target/release/macflayer" "$contents/Helpers/macflayer"
cp "target/$rust_target/release/macflayer-askpass" "$contents/Helpers/macflayer-askpass"

cp packaging/Info.plist "$contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"

iconset=$(mktemp -d)/MacFlayer.iconset
mkdir "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" packaging/icon/icon-1024.png \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" packaging/icon/icon-1024.png \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$contents/Resources/MacFlayer.icns" "$iconset"
rm -rf "$(dirname "$iconset")"

# The fonts, logos and data are compiled into the binary, so only their licenses travel as files.
cp app/assets/fonts/*-OFL.txt "$contents/Resources/licenses/"
cp app/assets/icons/SimpleIcons-LICENSE.md app/assets/icons/Devicon-LICENSE.txt "$contents/Resources/licenses/"

packaging/sign-app.sh - "$app"
packaging/check-app.sh "$app"

echo
echo "Built $app ($version), ad-hoc signed. Open it with: open $app"
