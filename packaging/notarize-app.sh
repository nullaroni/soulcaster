#!/bin/bash
# Signs a built MacFlayer.app with Developer ID and the hardened runtime, notarizes it and staples
# the ticket. Runs in the release workflow only. docs/releasing.md lists the secrets.
# Usage: packaging/notarize-app.sh <MacFlayer.app>
set -euo pipefail

app="$1"

: "${APPLE_CERTIFICATE_P12_BASE64:?}" "${APPLE_CERTIFICATE_PASSWORD:?}"
: "${APPLE_NOTARY_KEY_P8_BASE64:?}" "${APPLE_NOTARY_KEY_ID:?}" "${APPLE_NOTARY_ISSUER_ID:?}"

umask 077
work=$(mktemp -d)
keychain="$work/signing.keychain-db"
cleanup() {
  security delete-keychain "$keychain" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

keychain_password=$(/usr/bin/openssl rand -hex 32)
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"

printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/certificate.p12"
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" \
  -f pkcs12 -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" \
  "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" login.keychain-db

identity=$(security find-identity -v -p codesigning "$keychain" |
  sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -n 1)
if [ -z "$identity" ]; then
  echo "notarize-app.sh: no Developer ID Application identity in the certificate" >&2
  exit 1
fi

# The installer accepts one team. A release signed by any other is one it would refuse.
team_id=$(printf '%s' "$identity" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')
expected_team_id=$(sed -n 's/^  local team_id="\(.*\)"$/\1/p' "$(dirname "$0")/../install.sh")
if [ "$team_id" != "$expected_team_id" ]; then
  echo "notarize-app.sh: signing team '$team_id' isn't the team in install.sh, '$expected_team_id'" >&2
  exit 1
fi

"$(dirname "$0")/sign-app.sh" "$identity" "$app"

printf '%s' "$APPLE_NOTARY_KEY_P8_BASE64" | base64 --decode > "$work/notary-key.p8"
notary_auth=(--key "$work/notary-key.p8" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID")

ditto -c -k --keepParent "$app" "$work/submission.zip"
xcrun notarytool submit "$work/submission.zip" "${notary_auth[@]}" --wait --output-format json \
  > "$work/submission.json" || true
submission_id=$(plutil -extract id raw "$work/submission.json")
status=$(plutil -extract status raw "$work/submission.json")

# Apple says to read the log even on success: it can carry warnings.
xcrun notarytool log "$submission_id" "${notary_auth[@]}"
if [ "$status" != "Accepted" ]; then
  echo "notarize-app.sh: notarization ended as '$status'" >&2
  exit 1
fi

xcrun stapler staple "$app"
xcrun stapler validate "$app"

# The same two checks install.sh makes, so a release it would refuse never gets published.
codesign --verify --deep --strict \
  -R="anchor apple generic and certificate leaf[subject.OU] = \"$team_id\"" "$app"
assessment=$(spctl -a -vv -t exec "$app" 2>&1)
printf '%s\n' "$assessment"
case "$assessment" in
  *"source=Notarized Developer ID"*) ;;
  *)
    echo "notarize-app.sh: Gatekeeper doesn't see this app as notarized" >&2
    exit 1
    ;;
esac
