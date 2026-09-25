#!/bin/bash
# Builds bubbleTranslate-linux-x86_64 — the download itself, no installer.
#
# Linux has nothing to sign, notarize or package: the release asset is the
# executable. What this script is for is the part that used to be done by hand,
# and so was the part that drifted — rewriting latest.json, which is what tells
# installed copies that a new version exists. release.sh and release.ps1 do the
# same for the platforms they build; this is Linux's.
#
# The binary is built against glibc 2.28 rather than the glibc of this machine,
# with cargo-zigbuild, so the one download starts on every distribution from
# Debian 10, Ubuntu 20.04 and RHEL 8 onwards. A plain `cargo build` on a
# rolling-release system links against whatever glibc it has — 2.43 on the
# machine this was written on — and the loader then refuses the download on
# Ubuntu and Debian outright. Linked directly it needs nothing else: GL, X11,
# xkbcommon and Wayland are all loaded at runtime.
#
# Needs `cargo install cargo-zigbuild` and zig on the PATH (or installed with
# mise, where it is looked for).

set -euo pipefail
cd "$(dirname "$0")"

OUT="bubbleTranslate-linux-x86_64"
SUMS="SHA256SUMS.txt"

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

# --- build ------------------------------------------------------------------

GLIBC_FLOOR="2.28"
TARGET="x86_64-unknown-linux-gnu"

if ! command -v cargo-zigbuild >/dev/null; then
    echo "error: cargo-zigbuild is missing -- cargo install cargo-zigbuild" >&2
    exit 1
fi
if ! command -v zig >/dev/null; then
    ZIG="$(mise which zig 2>/dev/null || true)"
    if [ -z "$ZIG" ]; then
        echo "error: zig is missing -- mise install zig@0.14 (or your package manager's zig)" >&2
        exit 1
    fi
    PATH="$(dirname "$ZIG"):$PATH"
fi

echo "==> building against glibc $GLIBC_FLOOR"
cargo zigbuild --release --target "$TARGET.$GLIBC_FLOOR"
cp "target/$TARGET/release/bubbleTranslate" "$OUT"
chmod +x "$OUT"

# Checked rather than trusted: one dependency that reaches for a newer symbol
# quietly raises the floor, and the first to find out would be someone on
# Ubuntu whose download will not start.
NEEDED="$(objdump -T "$OUT" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)"
if [ "$(printf '%s\n%s\n' "$NEEDED" "$GLIBC_FLOOR" | sort -V | tail -1)" != "$GLIBC_FLOOR" ]; then
    echo "error: $OUT needs glibc $NEEDED, above the $GLIBC_FLOOR floor" >&2
    exit 1
fi
echo "    needs glibc $NEEDED at most"

# --- checksum ---------------------------------------------------------------
#
# Uploaded beside the binaries as one file covering all three platforms, so
# each release script replaces its own line and leaves the others alone: the
# file survives a release on one machine and is completed by the next.

# Started from the newest release's copy rather than whatever is lying here,
# which is only as fresh as the last Linux release and would put the other
# platforms' old checksums back.
gh release download -R pelamx/downloads -p "$SUMS" -O "$SUMS" --clobber 2>/dev/null || true
touch "$SUMS"
grep -v "  $OUT\$" "$SUMS" > "$SUMS.new" || true
sha256sum "$OUT" >> "$SUMS.new"
sort -k2 "$SUMS.new" -o "$SUMS"
rm -f "$SUMS.new"

# --- upload, then tell installed copies -------------------------------------
#
# The upload comes first and the manifest second, and the order is the whole
# point: latest.json is a promise that a file is there to be downloaded, so
# making it before uploading leaves a window where every installed copy is told
# about a version it would get a 404 for. The same order release.sh keeps.

REPO="pelamx/downloads"
if gh release view "v$VERSION" -R "$REPO" >/dev/null 2>&1; then
    gh release upload "v$VERSION" -R "$REPO" "$OUT" "$SUMS" --clobber
else
    gh release create "v$VERSION" -R "$REPO" "$OUT" "$SUMS" --title "bubbleTranslate $VERSION" \
        --notes "$(sed -n "/^## $VERSION /,/^## /p" CHANGELOG.md | sed '1d;$d')"
fi
echo "uploaded $OUT to v$VERSION in $REPO"

# /releases/latest/download follows whichever release went out last, so this
# one has to carry the macOS and Windows downloads too, or their links 404 for
# everybody the moment it is published.
./scripts/fill-release.sh "v$VERSION" linux

# latest.json lives in the downloads repository, because that is the one that
# stays public and every installed copy reads it on startup. Only the Linux
# line is touched: the other platforms are released on their own machines.
./scripts/publish-manifest.sh linux "$VERSION" "$OUT"

echo
echo "built and published $OUT ($(du -h "$OUT" | cut -f1)), version $VERSION"
echo "the version in Cargo.toml is the only thing left to commit here"
