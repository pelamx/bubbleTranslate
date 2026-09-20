#!/bin/bash
# Builds bubbleTranslate-linux-x86_64 — the download itself, no installer.
#
# Linux has nothing to sign, notarize or package: the release asset is the
# executable. What this script is for is the part that used to be done by hand,
# and so was the part that drifted — rewriting latest.json, which is what tells
# installed copies that a new version exists. release.sh and release.ps1 do the
# same for the platforms they build; this is Linux's.
#
# The binary is linked against the glibc of the machine that builds it and will
# not start on an older one, so build on the oldest system you intend to
# support. README.md names the floor the current download was built against.

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

echo "==> building"
cargo build --release
cp target/release/bubbleTranslate "$OUT"
chmod +x "$OUT"

# --- checksum ---------------------------------------------------------------
#
# Uploaded beside the binaries as one file covering all three platforms, so
# each release script replaces its own line and leaves the others alone: the
# file survives a release on one machine and is completed by the next.

touch "$SUMS"
grep -v "  $OUT\$" "$SUMS" > "$SUMS.new" || true
sha256sum "$OUT" >> "$SUMS.new"
sort -k2 "$SUMS.new" -o "$SUMS"
rm -f "$SUMS.new"

# --- tell installed copies --------------------------------------------------
#
# latest.json is what running copies read to say "a new version is available".
# Only the Linux line is touched: the other platforms are released on their own
# machines. Commit it together with the upload, or nobody is told.

PUBLISHED="$(perl -0ne 'print $1 if /"linux"\s*:\s*\{\s*"version"\s*:\s*"([^"]*)"/' latest.json)"
if [[ -z "$PUBLISHED" ]]; then
    echo "error: latest.json has no linux version to update" >&2
    exit 1
fi
perl -0pi -e 's/("linux"\s*:\s*\{\s*"version"\s*:\s*")[^"]*(")/${1}'"$VERSION"'${2}/' latest.json
if [[ "$VERSION" == "$PUBLISHED" ]]; then
    echo
    echo "warning: latest.json already says Linux $VERSION — bump the version in"
    echo "         Cargo.toml, or installed copies will not be told about this build"
fi

# --- what is left to do -----------------------------------------------------
#
# The download links in README.md and on the website point at
# /releases/latest/download/, which GitHub resolves to whichever release is
# current, so publishing the release is what moves them. There is nothing to
# edit there per release.

echo
echo "built $OUT ($(du -h "$OUT" | cut -f1)), version $VERSION"
echo "checksummed into $SUMS"
echo
echo "  gh release create v$VERSION --title \"bubbleTranslate $VERSION\" \\"
echo "      $OUT $SUMS"
echo
echo "use this version's section of CHANGELOG.md as the release notes -- it is"
echo "what the download page shows to whoever just saw the update banner"
echo
echo "then commit latest.json (the binary is not tracked — latest.json and the"
echo "version in Cargo.toml are the only things that are)"
