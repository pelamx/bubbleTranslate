#!/usr/bin/env bash
#
# Copies the *other* platforms' current downloads into a release.
#
# `https://github.com/bubbleTranslate/downloads/releases/latest/download/<file>`
# is one pointer per repository, and it follows whichever release was published
# last. The three platforms are released separately, on three machines, so the
# newest release is routinely the only one — and the two platforms missing from
# it have their `latest/download` link answer 404 for everybody.
#
# That is not hypothetical. It has broken the Windows and Linux downloads three
# separate times, most recently when macOS 0.2.10 went out carrying only the
# dmg, and each time it was silent: nothing fails, nothing is logged, the link
# simply stops working until somebody clicks it.
#
# So every release carries all of them. This reads `latest.json` — the file
# that already knows each platform's current version and exact url, and the
# reason there is no second place to keep that — downloads what the other two
# platforms are currently on, and uploads them here.
#
# Idempotent: run it twice and the second run re-uploads the same bytes.
#
#   scripts/fill-release.sh v0.3.1 windows
#
# The second argument is the platform this release is *for*, which is the one
# that is skipped: its own files were just built and uploaded by the release
# script that called this.

set -euo pipefail

TAG="${1:-}"
MINE="${2:-}"
REPO="bubbleTranslate/downloads"

if [[ -z "$TAG" || -z "$MINE" ]]; then
    echo "usage: $(basename "$0") <tag> <macos|linux|windows>" >&2
    exit 2
fi

# `gh` only. Its own `--jq` reads the API response, and the manifest inside is
# picked apart with perl the way publish-manifest.sh already does it — a
# release machine is not guaranteed to have jq installed, and the Windows one
# does not.
if ! command -v gh >/dev/null || ! command -v perl >/dev/null; then
    echo "error: gh and perl are needed to fill $TAG; the other platforms were" >&2
    echo "       not copied in, and their latest/download links will 404" >&2
    exit 1
fi

# Only base64 lines are kept: a gh reached through a version-manager shim can
# print a line of its own on stdout first, and one stray line is enough to make
# the whole decode come back empty -- which reads as "no entry" for every
# platform and fills nothing.
MANIFEST="$(gh api "repos/$REPO/contents/latest.json" --jq .content 2>/dev/null | grep -E '^[A-Za-z0-9+/=]+$' | tr -d '\n' | base64 -d 2>/dev/null || true)"
if [[ -z "$MANIFEST" ]]; then
    echo "error: could not read latest.json from $REPO; nothing was copied" >&2
    exit 1
fi

# One platform's field out of the manifest, by name.
field() {
    perl -0ne 'print $1 if /"'"$1"'"\s*:\s*\{[^}]*?"'"$2"'"\s*:\s*"([^"]*)"/' <<<"$MANIFEST"
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

copied=0
for os in macos linux windows; do
    [[ "$os" == "$MINE" ]] && continue

    url="$(field "$os" url)"
    version="$(field "$os" version)"
    if [[ -z "$url" || -z "$version" ]]; then
        echo "warning: latest.json has no $os entry; its link will 404 on $TAG" >&2
        continue
    fi

    # The release to take it from is the one named in the url, and the file is
    # the last segment of it. Both come from latest.json rather than being
    # spelled out here, so a renamed asset needs no change in this script.
    from_tag="$(sed -E 's#.*/releases/download/([^/]+)/.*#\1#' <<<"$url")"
    file="${url##*/}"

    if [[ "$from_tag" == "$TAG" ]]; then
        echo "  $os $version is already this release"
        continue
    fi

    # Windows offers the bare .exe beside the zip, for anyone who would rather
    # not unpack one. latest.json names only the zip, because that is what the
    # download buttons point at — so the exe is asked for by name.
    want=("$file")
    [[ "$os" == "windows" ]] && want+=("bubbleTranslate.exe")

    for name in "${want[@]}"; do
        if ! gh release download "$from_tag" -R "$REPO" -p "$name" -D "$WORK" --clobber >/dev/null 2>&1; then
            echo "warning: $from_tag has no $name; skipped" >&2
            continue
        fi
        gh release upload "$TAG" -R "$REPO" "$WORK/$name" --clobber >/dev/null
        echo "  copied $name from $from_tag ($os $version)"
        copied=$((copied + 1))
    done
done

if [[ "$copied" -eq 0 ]]; then
    echo "nothing to copy into $TAG"
else
    echo "$TAG now carries every platform: latest/download works for all three"
fi
