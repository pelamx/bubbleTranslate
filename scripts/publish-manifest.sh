#!/bin/bash
# Publishes one platform's line of latest.json to the downloads repository.
#
# latest.json lives there rather than beside the source, because it is what
# every installed copy reads on startup and the source repository is private.
# Each release script rewrites only its own platform's line, so a release on
# one machine leaves the other two alone -- the file is fetched, patched and
# put back in one go rather than kept in sync by hand.
#
#   publish-manifest.sh <linux|macos|windows> <version> <asset-file-name>
set -euo pipefail

REPO="pelamx/downloads"
# Some shells wrap gh in a version manager that greets on stdout; this quiets
# the common one, and the answer is read from its first `{` either way.
export MISE_QUIET=1
OS="${1:?which platform}"
VERSION="${2:?which version}"
ASSET="${3:?which file}"

if ! command -v gh >/dev/null || ! command -v jq >/dev/null; then
    echo "error: gh and jq are needed to publish latest.json; it was not published" >&2
    echo "       install it, or edit latest.json in $REPO by hand" >&2
    exit 1
fi

# One read, then the file and its sha out of the same answer: fetching twice
# could straddle someone else's release and put back a file built on the older
# of the two.
FILE="$(gh api "repos/$REPO/contents/latest.json" 2>/dev/null | sed -n '/^{/,$p')"
CURRENT="$(printf '%s' "$FILE" | jq -r .content | tr -d '\n' | base64 -d)"
SHA="$(printf '%s' "$FILE" | jq -r .sha)"
PUBLISHED="$(printf '%s' "$CURRENT" | perl -0ne 'print $1 if /"'"$OS"'"\s*:\s*\{\s*"version"\s*:\s*"([^"]*)"/')"
if [[ -z "$PUBLISHED" ]]; then
    echo "error: latest.json in $REPO has no $OS version to update" >&2
    exit 1
fi

URL="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
NEXT="$(
    OS="$OS" VERSION="$VERSION" URL="$URL" perl -0pe '
        s{("$ENV{OS}"\s*:\s*\{\s*"version"\s*:\s*")[^"]*(")}{$1 . $ENV{VERSION} . $2}se;
        s{("$ENV{OS}"\s*:\s*\{.*?"url"\s*:\s*")[^"]*(")}{$1 . $ENV{URL} . $2}se;
    ' <<<"$CURRENT"
)"

if [[ "$NEXT" == "$CURRENT" ]]; then
    echo "latest.json already says $OS $VERSION — nothing to publish"
    exit 0
fi

gh api "repos/$REPO/contents/latest.json" -X PUT \
    -f message="$OS $VERSION" \
    -f sha="$SHA" \
    -f content="$(printf '%s' "$NEXT" | base64 -w0 2>/dev/null || printf '%s' "$NEXT" | base64 | tr -d '\n')" \
    --jq .commit.sha >/dev/null

echo "published: installed copies on $OS are now told about $VERSION"
if [[ "$VERSION" == "$PUBLISHED" ]]; then
    echo "warning: $REPO already named $VERSION for $OS — bump the version in"
    echo "         Cargo.toml, or installed copies will not be told about this build"
fi
