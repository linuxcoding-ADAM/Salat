#!/bin/sh
# `auth-check` job (docs/ARCHITECTURE.md §22.2, §28 D4/D5): on prerelease
# tags, prove that the publishing token works WITHOUT publishing anything.
# One read-only GET of the repository; the token must be present, accepted
# by GitHub, and report push (Contents write) access. The token travels to
# curl on stdin, never on a command line, and is never printed.
set -eu

REPO=linuxcoding-ADAM/Salat

if [ -z "${GITHUB_RELEASE_TOKEN:-}" ]; then
    echo "auth-check: GITHUB_RELEASE_TOKEN is not available." >&2
    echo "  Either the CI/CD variable is not set, or this tag is not protected" >&2
    echo "  (the variable is protected: only protected tags can read it)." >&2
    exit 1
fi

body=$(mktemp)
status=$(printf 'header = "Authorization: Bearer %s"\n' "$GITHUB_RELEASE_TOKEN" \
    | curl -sS -K - -o "$body" -w '%{http_code}' \
        -H 'Accept: application/vnd.github+json' \
        -H 'X-GitHub-Api-Version: 2022-11-28' \
        -A 'salat-ci-auth-check' \
        "https://api.github.com/repos/$REPO") || { echo "auth-check: could not reach api.github.com" >&2; exit 1; }

case "$status" in
    200) ;;
    401) echo "auth-check: GitHub rejected the token (401: invalid, expired or revoked)" >&2; exit 1 ;;
    403 | 404) echo "auth-check: the token cannot see $REPO (HTTP $status: wrong repository scope or missing permission)" >&2; exit 1 ;;
    *) echo "auth-check: unexpected HTTP $status from GitHub" >&2; exit 1 ;;
esac

full=$(jq -r '.full_name' "$body")
push=$(jq -r '.permissions.push // false' "$body")
[ "$full" = "$REPO" ] || { echo "auth-check: GitHub answered for '$full', expected $REPO" >&2; exit 1; }
if [ "$push" != "true" ]; then
    echo "auth-check: the token can read $REPO but has no write access (needs Contents: read and write)" >&2
    exit 1
fi
echo "auth-check: OK — token accepted for $REPO with push access (nothing was published)"
