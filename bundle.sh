#!/bin/bash
# Builds bubbleTranslate.app.
#
# The bundle is not cosmetic. macOS grants Accessibility permission to a code
# signature, not to a path, so a bare `cargo run` binary loses the grant on
# every rebuild and the app silently stops seeing selections. An ad-hoc
# signature over a stable bundle identifier keeps the grant across rebuilds.

set -euo pipefail
cd "$(dirname "$0")"

APP="bubbleTranslate.app"
# The Accessibility grant is keyed to this identifier, so changing it makes
# macOS treat the app as new and ask for permission again.
BUNDLE_ID="com.pelamx.bubbleTranslate"

STAGE_BIN="$(mktemp -t bubbleTranslate-bin)"
trap 'rm -f "$STAGE_BIN"' EXIT

# --- build a universal binary ------------------------------------------------
#
# Both architectures, joined with lipo. An arm64-only build simply does not
# launch on an Intel Mac: Rosetta translates x86_64 to arm64, never the other
# way, so there is no fallback to rely on.
#
# cargo runs whichever `rustc` comes first on PATH. If Homebrew's rust is
# ahead of rustup's it will not know about any added target, and the build
# fails claiming the target is not installed when it plainly is -- so prefer
# rustup's toolchain here rather than depending on the caller's PATH.
[[ -x "$HOME/.cargo/bin/rustc" ]] && export PATH="$HOME/.cargo/bin:$PATH"

ARM="aarch64-apple-darwin"
INTEL="x86_64-apple-darwin"

cargo build --release --target "$ARM"

# Intel is best-effort: without its std the build still produces a working
# Apple Silicon app, which is better than failing the release outright. The
# warning is loud because shipping that DMG excludes every Intel Mac.
if rustup target list --installed 2>/dev/null | grep -qx "$INTEL"; then
    cargo build --release --target "$INTEL"
    lipo -create -output "$STAGE_BIN" \
        "target/$ARM/release/bubbleTranslate" \
        "target/$INTEL/release/bubbleTranslate"
else
    echo "warning: $INTEL not installed -- building Apple Silicon only." >&2
    echo "         Intel Macs cannot run this build. Fix with:" >&2
    echo "           rustup target add $INTEL" >&2
    cp "target/$ARM/release/bubbleTranslate" "$STAGE_BIN"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$STAGE_BIN" "$APP/Contents/MacOS/bubbleTranslate"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>bubbleTranslate</string>
    <key>CFBundleDisplayName</key>     <string>bubbleTranslate</string>
    <key>CFBundleIdentifier</key>      <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>      <string>bubbleTranslate</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>0.1.0</string>
    <key>CFBundleVersion</key>         <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>  <string>11.0</string>
    <key>NSHumanReadableCopyright</key> <string>by pelamx</string>
    <!-- Menu-bar-less background app: no Dock icon, and showing the bubble
         never pulls focus away from whatever is being read. -->
    <key>LSUIElement</key>             <true/>
    <key>NSHighResolutionCapable</key> <true/>
</dict>
</plist>
PLIST

# Sign with the self-signed certificate from ./setup-signing.sh when it exists.
#
# This is what keeps the Accessibility grant alive across rebuilds. An ad-hoc
# signature's designated requirement is the binary's own hash, so every build
# looks like a different program to macOS and the grant stops applying while
# still appearing enabled. A certificate pins the requirement to the identity
# instead, and later builds keep satisfying it.
SIGNING_KEYCHAIN="bubbletranslate-signing"
SIGNING_CERT="bubbleTranslate Signing"
KEYCHAIN_PW_FILE="$HOME/.config/bubbletranslate/signing.pw"

if security find-identity -p codesigning "$SIGNING_KEYCHAIN" 2>/dev/null | grep -q "$SIGNING_CERT"; then
    # Unlocking is a no-op when it is already open, and the build fails with an
    # opaque error if it is not.
    if [[ -f "$KEYCHAIN_PW_FILE" ]]; then
        security unlock-keychain -p "$(cat "$KEYCHAIN_PW_FILE")" "$SIGNING_KEYCHAIN" 2>/dev/null || true
    fi
    codesign --force --sign "$SIGNING_CERT" --keychain "$SIGNING_KEYCHAIN" \
        --identifier "$BUNDLE_ID" "$APP"
    echo "signed with: $SIGNING_CERT (Accessibility grant survives rebuilds)"
else
    codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"
    echo "ad-hoc signed — run ./setup-signing.sh to stop re-granting Accessibility"
fi

echo
echo "Built $APP"
echo
echo "First run:"
echo "  open $APP"
echo "  then allow it in System Settings › Privacy & Security › Accessibility"
echo "  and relaunch (the event tap is installed at startup)."
