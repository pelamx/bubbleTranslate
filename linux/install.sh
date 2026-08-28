#!/bin/bash
# Builds and installs bubbleTranslate into ~/.local, no root needed.
#
# Everything lands under the XDG user directories, which every desktop already
# searches: the binary in ~/.local/bin, the launcher and icon where the
# application menu will find them. Uninstalling is deleting the three files
# this prints at the end.

set -euo pipefail
cd "$(dirname "$0")/.."

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"

echo "==> checking the session"
if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    # The interface is an X11 client even here; without XWayland it cannot
    # open a window at all, so this is worth catching before the build.
    if [[ -z "${DISPLAY:-}" ]]; then
        echo "    warning: Wayland session with no XWayland (DISPLAY is unset)."
        echo "             bubbleTranslate draws through X11 and will not start."
    fi
    echo "    Wayland: selections are read over wlr-data-control."
    echo "    GNOME does not implement it; the bubble will stay quiet there."
else
    echo "    X11: selections are read from the primary selection."
fi

echo "==> building"
cargo build --release

echo "==> installing"
install -Dm755 target/release/bubbleTranslate "$BIN_DIR/bubbleTranslate"
install -Dm644 linux/bubbleTranslate.desktop "$APP_DIR/bubbleTranslate.desktop"
install -Dm644 linux/bubbleTranslate.svg "$ICON_DIR/bubbleTranslate.svg"

# Only some desktops need this, and it is harmless on the ones that do not.
command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" || true

echo
echo "Installed:"
echo "  $BIN_DIR/bubbleTranslate"
echo "  $APP_DIR/bubbleTranslate.desktop"
echo "  $ICON_DIR/bubbleTranslate.svg"
echo
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH." ;;
esac
echo "Run it with:  bubbleTranslate"
echo "Check the backends without opening a window:  bubbleTranslate --check"
