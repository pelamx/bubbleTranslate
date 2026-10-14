#!/usr/bin/env bash
#
# Leaves one release in the downloads repository: the one just published.
#
# Only the current version is offered. An old build is not something anyone
# should be able to download by accident from a link they found somewhere, and
# every installed copy is told about the newest one anyway.
#
# Two things have to be true before anything is deleted, and this checks both
# rather than assuming them:
#
# - latest.json names this release for all three platforms. The release
#   scripts rewrite only their own line, so the other two still point at the
#   release they were last built in -- the one about to be deleted. After
#   fill-release.sh has copied their files in here, their urls are moved over
#   too; the version beside each stays what it was, because the bytes did.
# - every file latest.json names is actually in this release. A url that
#   answers 404 is an update nobody can install, and silently.
#
# Nothing is deleted that is not kept first. Every release is downloaded into
# ~/bubbleTranslate-archive/pelamx/<tag>/ on the Mac, with its notes, and each
# file is checked against the size GitHub reports before the release goes. That
# archive is the only copy of an old build once it is gone from GitHub, so on
# Windows and Linux this moves latest.json and stops there: the old release
# stays up until this is run again on the Mac, which archives and deletes it.
#
# Only `v*` releases go. `ocr-models-1` is downloaded by the Linux app at run
# time and is never touched.
#
#   scripts/prune-releases.sh v0.4.9

set -euo pipefail

TAG="${1:-}"
REPO="pelamx/downloads"
export MISE_QUIET=1

if [[ -z "$TAG" ]]; then
    echo "usage: $(basename "$0") <tag>" >&2
    exit 2
fi
# `gh` and perl only, as in fill-release.sh: the Windows release machine has no
# jq, so the API answers are read with gh's own --jq and the manifest with perl.
if ! command -v gh >/dev/null || ! command -v perl >/dev/null; then
    echo "error: gh and perl are needed to prune; nothing was deleted" >&2
    exit 1
fi

ASSETS="$(gh release view "$TAG" -R "$REPO" --json assets --jq '.assets[].name')"
if [[ -z "$ASSETS" ]]; then
    echo "error: $TAG has no assets in $REPO; nothing was deleted" >&2
    exit 1
fi

# --- latest.json: every platform onto this release ---------------------------

# Content and sha from one read, so the file put back is built on the one read.
FILE="$(gh api "repos/$REPO/contents/latest.json" --jq '.sha + " " + (.content | gsub("\n"; ""))' 2>/dev/null \
    | grep -E '^[0-9a-f]{40} ')"
SHA="${FILE%% *}"
CURRENT="$(printf '%s' "${FILE#* }" | tr -d '\n' | base64 -d 2>/dev/null || true)"
if [[ -z "$SHA" || -z "$CURRENT" ]]; then
    echo "error: could not read latest.json from $REPO; nothing was deleted" >&2
    exit 1
fi
NEXT="$CURRENT"

for os in macos linux windows; do
    url="$(perl -0ne 'print $1 if /"'"$os"'"\s*:\s*\{[^}]*?"url"\s*:\s*"([^"]*)"/' <<<"$CURRENT")"
    if [[ -z "$url" ]]; then
        echo "error: latest.json has no $os url; nothing was deleted" >&2
        exit 1
    fi
    file="${url##*/}"
    if ! grep -qxF "$file" <<<"$ASSETS"; then
        echo "error: $TAG has no $file, which latest.json gives $os; run" >&2
        echo "       scripts/fill-release.sh first. Nothing was deleted" >&2
        exit 1
    fi
    want="https://github.com/$REPO/releases/download/$TAG/$file"
    if [[ "$url" != "$want" ]]; then
        NEXT="$(OS="$os" URL="$want" perl -0pe '
            s{("$ENV{OS}"\s*:\s*\{.*?"url"\s*:\s*")[^"]*(")}{$1 . $ENV{URL} . $2}se;
        ' <<<"$NEXT")"
        echo "  $os now downloads from $TAG"
    fi
done

if [[ "$NEXT" != "$CURRENT" ]]; then
    gh api "repos/$REPO/contents/latest.json" -X PUT \
        -f message="point every platform at $TAG" \
        -f sha="$SHA" \
        -f content="$(printf '%s' "$NEXT" | base64 -w0 2>/dev/null || printf '%s' "$NEXT" | base64 | tr -d '\n')" \
        --jq .commit.sha >/dev/null
fi

# --- archived on the Mac, then the rest go -----------------------------------

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "older releases were left up: they are archived on the Mac before they"
    echo "are deleted. There, run:  scripts/prune-releases.sh $TAG"
    exit 0
fi

ARCHIVE="$HOME/bubbleTranslate-archive/pelamx"

# Downloads a release into the archive and says whether every file arrived
# whole. A file already there at the right size is not fetched again.
archive() {
    local tag="$1" dir="$ARCHIVE/$1" name size have ok=0
    mkdir -p "$dir"
    gh release view "$tag" -R "$REPO" --json name,body,publishedAt,assets > "$dir/release.json"
    gh release download "$tag" -R "$REPO" -D "$dir" --skip-existing >/dev/null
    while IFS=$'\t' read -r name size; do
        have="$(stat -f %z "$dir/$name" 2>/dev/null || echo 0)"
        if [[ "$have" != "$size" ]]; then
            echo "error: $tag/$name is $have bytes in the archive, $size on GitHub" >&2
            ok=1
        fi
    done < <(gh release view "$tag" -R "$REPO" --json assets \
        --jq '.assets[] | .name + "\t" + (.size | tostring)')
    return "$ok"
}

# The new release too, so the archive holds every version that was published
# and not only the ones that have since been replaced.
archive "$TAG" && echo "  archived $TAG in $ARCHIVE/$TAG"

deleted=0
for old in $(gh release list -R "$REPO" -L 1000 --json tagName --jq '.[].tagName'); do
    [[ "$old" == v* && "$old" != "$TAG" ]] || continue
    if ! archive "$old"; then
        echo "error: $old was not archived whole, so it was not deleted" >&2
        continue
    fi
    gh release delete "$old" -R "$REPO" --cleanup-tag --yes
    echo "  archived and deleted $old"
    deleted=$((deleted + 1))
done

echo "$REPO now offers $TAG only ($deleted older release(s) deleted)"
