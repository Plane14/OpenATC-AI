#!/bin/bash
set -euo pipefail
if [ -z "${OPENATC_MAC_CERT_P12_BASE64:-}" ]; then exit 0; fi
: "${OPENATC_MAC_CERT_PASSWORD:?Missing signing certificate password}"
: "${OPENATC_MAC_SIGN_IDENTITY:?Missing Developer ID Application identity}"
certificate="$RUNNER_TEMP/openatc-signing.p12"
keychain="$RUNNER_TEMP/openatc-signing.keychain-db"
keychain_password=$(openssl rand -hex 32)
trap 'rm -f "$certificate"' EXIT
printf '%s' "$OPENATC_MAC_CERT_P12_BASE64" | base64 -D > "$certificate"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$certificate" -k "$keychain" -P "$OPENATC_MAC_CERT_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" "$HOME/Library/Keychains/login.keychain-db"
printf 'OPENATC_MAC_SIGN_IDENTITY=%s\n' "$OPENATC_MAC_SIGN_IDENTITY" >> "$GITHUB_ENV"
if [ -n "${OPENATC_APPLE_ID:-}${OPENATC_APPLE_PASSWORD:-}${OPENATC_APPLE_TEAM_ID:-}" ]; then
    : "${OPENATC_APPLE_ID:?Missing Apple ID}"
    : "${OPENATC_APPLE_PASSWORD:?Missing app-specific Apple password}"
    : "${OPENATC_APPLE_TEAM_ID:?Missing Apple team ID}"
    xcrun notarytool store-credentials openatc-ci --keychain "$keychain" --apple-id "$OPENATC_APPLE_ID" --password "$OPENATC_APPLE_PASSWORD" --team-id "$OPENATC_APPLE_TEAM_ID"
    # Use the imported keychain as the default so notarytool finds the profile.
    security default-keychain -d user -s "$keychain"
    echo 'OPENATC_NOTARY_PROFILE=openatc-ci' >> "$GITHUB_ENV"
fi
