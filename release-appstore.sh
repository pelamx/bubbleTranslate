#!/bin/bash
# Builds, signs and uploads bubbleTranslate to App Store Connect.
#
# Separate from release.sh on purpose: the direct download is signed with a
# Developer ID and notarized, the App Store build is sandboxed and signed with
# the App Store certificates, and neither release should be able to touch the
# other.
#
# Needs, once, on the release Mac:
#   - "Apple Distribution" and "3rd Party Mac Developer Installer" (Mac
#     Installer Distribution) certificates in the login keychain
#   - an App Store provisioning profile for com.pelamx.bubbleTranslate, saved
#     as appstore/embedded.provisionprofile
#   - an App Store Connect API key for the upload:
#       ASC_KEY_ID=...  ASC_ISSUER_ID=...  (the .p8 in ~/.appstoreconnect/private_keys)
#
# Usage: ./release-appstore.sh            build, sign, package
#        UPLOAD=1 ./release-appstore.sh   ...and upload to App Store Connect

set -euo pipefail
cd "$(dirname "$0")"

APP="bubbleTranslate.app"
PKG="bubbleTranslate-appstore.pkg"
ENTITLEMENTS="appstore/bubbleTranslate.entitlements"
PROFILE="appstore/embedded.provisionprofile"
APP_IDENTITY="${APP_IDENTITY:-Apple Distribution}"
INSTALLER_IDENTITY="${INSTALLER_IDENTITY:-3rd Party Mac Developer Installer}"

[[ -f "$PROFILE" ]] || { echo "error: $PROFILE is missing (see the header)" >&2; exit 1; }

APPSTORE=1 ./bundle.sh

# Every upload needs a build number higher than the last, even for the same
# version, so it is stamped here rather than kept by hand.
plutil -replace CFBundleVersion -string "$(date -u +%Y%m%d%H%M)" "$APP/Contents/Info.plist"

cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
codesign --force --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APP_IDENTITY" "$APP"
codesign --verify --strict "$APP"
codesign -d --entitlements - "$APP" 2>/dev/null | grep -q app-sandbox \
    || { echo "error: the sandbox entitlement did not make it into the signature" >&2; exit 1; }

rm -f "$PKG"
productbuild --component "$APP" /Applications --sign "$INSTALLER_IDENTITY" "$PKG"
echo "packaged: $PKG"

if [[ "${UPLOAD:-0}" == 1 ]]; then
    : "${ASC_KEY_ID:?set ASC_KEY_ID}" "${ASC_ISSUER_ID:?set ASC_ISSUER_ID}"
    xcrun altool --upload-app --type macos --file "$PKG" \
        --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID"
    echo "uploaded; it appears in TestFlight once App Store Connect has processed it"
fi
