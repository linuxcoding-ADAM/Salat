#!/bin/sh
# `publish` job (docs/ARCHITECTURE.md §22.2, §28 D4/D5): create the GitHub
# Release for a stable vX.Y.Z tag from THIS pipeline's own verified
# artifacts. Manual, protected tags only. The tag must already exist on
# GitHub (--verify-tag). The token is read from GITHUB_RELEASE_TOKEN by
# gh as GH_TOKEN and is never printed; no shell tracing here.
#
# Before it creates anything it makes read-only GitHub checks: the token has
# push access, the tag exists on GitHub AND points at the commit this pipeline
# verified, and no release for the tag exists yet.
set -eu

REPO=linuxcoding-ADAM/Salat
TARGET=x86_64-unknown-linux-musl

tag=${CI_COMMIT_TAG:-}
printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    || { echo "publish: '$tag' is not a stable vX.Y.Z tag — refusing" >&2; exit 1; }
[ "${CI_COMMIT_REF_PROTECTED:-}" = "true" ] \
    || { echo "publish: tag is not protected — refusing" >&2; exit 1; }
[ -n "${GITHUB_RELEASE_TOKEN:-}" ] \
    || { echo "publish: GITHUB_RELEASE_TOKEN is not available" >&2; exit 1; }

version=${tag#v}
asset="dist/salat-${version}-${TARGET}.tar.gz"
[ -f "$asset" ] || { echo "publish: missing $asset" >&2; exit 1; }
[ -f dist/SHA256SUMS ] || { echo "publish: missing dist/SHA256SUMS" >&2; exit 1; }
n=$(find dist -maxdepth 1 -name '*.tar.gz' | wc -l)
[ "$n" -eq 1 ] || { echo "publish: expected exactly one tarball in dist/, found $n" >&2; exit 1; }
(cd dist && sha256sum -c SHA256SUMS)

# Release notes: the CHANGELOG.md section whose heading starts "## [<version>]".
notes=$(mktemp)
awk -v h="## [${version}]" 'index($0, h) == 1 { f = 1; next } f && /^## \[/ { exit } f' CHANGELOG.md >"$notes"
grep -q '[^[:space:]]' "$notes" \
    || { echo "publish: CHANGELOG.md has no non-empty '## [${version}]' section" >&2; exit 1; }

GH_TOKEN=$GITHUB_RELEASE_TOKEN
export GH_TOKEN

# Preflight (read-only): nothing is created unless all of these hold.
push=$(gh api "repos/$REPO" --jq .permissions.push 2>/dev/null) || push=""
[ "$push" = true ] \
    || { echo "publish: the token has no push access to $REPO — refusing" >&2; exit 1; }
gh_sha=$(gh api "repos/$REPO/commits/$tag" --jq .sha 2>/dev/null) || gh_sha=""
[ -n "$gh_sha" ] \
    || { echo "publish: tag $tag is not on GitHub yet — push it first" >&2; exit 1; }
[ "$gh_sha" = "${CI_COMMIT_SHA:-}" ] \
    || { echo "publish: GitHub's $tag points at a different commit than this pipeline verified — refusing" >&2; exit 1; }
if gh release view "$tag" --repo "$REPO" >/dev/null 2>&1; then
    echo "publish: a release for $tag already exists — refusing" >&2
    exit 1
fi

gh release create "$tag" "$asset" dist/SHA256SUMS \
    --repo "$REPO" --verify-tag --title "Salat ${version}" --notes-file "$notes"
echo "publish: OK — release $tag created on $REPO"
