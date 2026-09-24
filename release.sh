#!/bin/bash
# Builds bubbleTranslate.dmg — a drag-to-Applications installer.
#
# Works with no Apple developer account: the app is ad-hoc signed and the DMG
# installs fine, but Gatekeeper warns on first launch and the user has to
# clear it once via System Settings › Privacy & Security › Open Anyway.
#
# With an account, set both variables and the same script produces a release
# that opens with no warning at all:
#
#   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
#   NOTARY_PROFILE=bubbleTranslate-notary \
#   ./release.sh
#
# The notary profile is created once with:
#   xcrun notarytool store-credentials bubbleTranslate-notary \
#     --apple-id you@example.com --team-id TEAMID --password <app-specific-password>

set -euo pipefail
cd "$(dirname "$0")"

APP="bubbleTranslate.app"
DMG="bubbleTranslate.dmg"
VOLNAME="bubbleTranslate"
STAGE="$(mktemp -d)/dmg"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

SIGN_IDENTITY="${SIGN_IDENTITY:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"

# --- the record -------------------------------------------------------------
#
# Refused rather than warned: a release whose entry is written afterwards is
# written from memory, and the people it is for are the two running the app.

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if ! grep -q "^## $VERSION " CHANGELOG.md && ! grep -q "^## $VERSION$" CHANGELOG.md; then
    echo "error: CHANGELOG.md has no section for $VERSION." >&2
    echo "       Add one -- rename '## Unreleased' to '## $VERSION -- $(date +%Y-%m-%d)'" >&2
    echo "       and say, for the people using the app, what changed and why." >&2
    exit 1
fi

# --- build the .app ---------------------------------------------------------

./bundle.sh > /dev/null
echo "built $APP"

# --- sign -------------------------------------------------------------------

if [[ -n "$SIGN_IDENTITY" ]]; then
    # --options runtime enables the hardened runtime, which notarization
    # requires. It does not interfere with the Accessibility permission.
    codesign --force --deep --options runtime --timestamp \
        --sign "$SIGN_IDENTITY" "$APP"
    echo "signed with: $SIGN_IDENTITY"
else
    echo "no SIGN_IDENTITY — keeping the ad-hoc signature (Gatekeeper will warn)"
fi

# --- notarize ---------------------------------------------------------------
#
# Notarization runs on a zip of the app rather than the DMG, so the ticket can
# be stapled into the app before the DMG is built around it.

if [[ -n "$NOTARY_PROFILE" ]]; then
    if [[ -z "$SIGN_IDENTITY" ]]; then
        echo "error: notarization needs a Developer ID signature; set SIGN_IDENTITY" >&2
        exit 1
    fi
    ZIP="$(dirname "$STAGE")/bubbleTranslate.zip"
    ditto -c -k --keepParent "$APP" "$ZIP"
    xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$APP"
    echo "notarized and stapled"
fi

# --- lay out the installer --------------------------------------------------

mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
# The symlink is what makes the window a drag-to-install target.
ln -s /Applications "$STAGE/Applications"

cat > "$STAGE/Read me first.txt" <<'TXT'
bubbleTranslate - by pelamx


Three steps and about a minute.

Step 2 is a macOS warning that says the app cannot be verified. That is
expected for an app not notarized by Apple, and step 2 clears it for good.
It is not a sign that the download is broken.


--- 1. Install -------------------------------------------------------------

Drag bubbleTranslate.app onto the Applications folder, here in this window.


--- 2. Allow the first launch ----------------------------------------------

Open bubbleTranslate from your Applications folder. macOS refuses to run it
and says it "could not verify" the developer. To clear that, once:

  1. Dismiss the warning.
  2. Open System Settings > Privacy & Security.
  3. Scroll down to Security. A line there names bubbleTranslate.
  4. Click "Open Anyway", confirm with Touch ID or your password, then
     click "Open Anyway" in the dialog that follows.

Two confirmations in a row is normal - the second one is the last.

Control-click > Open does not work for this on macOS 15 and later.

If you are comfortable with Terminal, this one line replaces all four steps
above, and the app opens normally from then on:

  xattr -dr com.apple.quarantine /Applications/bubbleTranslate.app


--- 3. Turn on Accessibility -----------------------------------------------

bubbleTranslate works by reading the text you select, and macOS keeps that
behind the Accessibility permission. The app asks on first launch and opens
the right page for you:

  System Settings > Privacy & Security > Accessibility

Turn bubbleTranslate on there. It starts watching the moment you do -
nothing to quit, nothing to relaunch.


--- Done -------------------------------------------------------------------

Select text in any app - double-click a word, drag a phrase, triple-click a
line - and the translation appears at your cursor.

There is no Dock icon while it runs in the background. The globe in the menu
bar reopens the window, switches the languages, or quits.
TXT

# --- build the disk image ---------------------------------------------------

rm -f "$DMG"
hdiutil create \
    -volname "$VOLNAME" \
    -srcfolder "$STAGE" \
    -ov -format UDZO \
    "$DMG" > /dev/null

if [[ -n "$SIGN_IDENTITY" ]]; then
    codesign --force --sign "$SIGN_IDENTITY" "$DMG"
    # Verifies the DMG the way Gatekeeper will on the user's machine.
    spctl -a -vvv -t install "$DMG" || true
fi

# --- tell installed copies ---------------------------------------------------
#
# latest.json lives in the downloads repository, because that is the one that
# stays public and every installed copy reads it on startup. Only the macOS
# line is touched: the other platforms are released on their own machines.

# The upload comes first and the manifest second, and the order is the whole
# point: latest.json is a promise that a file is there to be downloaded, so
# making it before uploading leaves a window -- minutes, if the upload is slow
# or fails -- where every installed copy is told about a version it would get a
# 404 for.
REPO="bubbleTranslate/downloads"
if gh release view "v$VERSION" -R "$REPO" >/dev/null 2>&1; then
    gh release upload "v$VERSION" -R "$REPO" "$DMG" --clobber
else
    gh release create "v$VERSION" -R "$REPO" "$DMG" --title "bubbleTranslate $VERSION" \
        --notes "$(sed -n "/^## $VERSION /,/^## /p" CHANGELOG.md | sed '1d;$d')"
fi
echo "uploaded $DMG to v$VERSION in $REPO"

# Every asset in place before anything promises it: this copies the other
# platforms' downloads into the new release, so /releases/latest/download does
# not 404 for them the moment this one becomes the newest.
./scripts/fill-release.sh "v$VERSION" macos

# And the promise last of all.
./scripts/publish-manifest.sh macos "$VERSION" "$DMG"

echo
echo "built and published $DMG ($(du -h "$DMG" | cut -f1)), version $VERSION"
if [[ -z "$NOTARY_PROFILE" ]]; then
    echo
    echo "Not notarized. On another Mac the first launch is blocked; clear it once"
    echo "via System Settings > Privacy & Security > Open Anyway. See this script's"
    echo "header for the signed, warning-free release path."
fi
