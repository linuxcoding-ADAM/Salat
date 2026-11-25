#!/bin/sh
# `publish` job (docs/ARCHITECTURE.md §22.2, §28 D4/D5): create the GitHub
# Release for a stable vX.Y.Z tag from THIS pipeline's own verified
# artifacts. Manual, protected tags only. The tag must already exist on
# GitHub (--verify-tag). The token is read from GITHUB_RELEASE_TOKEN by
# gh as GH_TOKEN and is never printed; no shell tracing here.
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
gh release create "$tag" "$asset" dist/SHA256SUMS \
    --repo "$REPO" --verify-tag --title "Salat ${version}" --notes-file "$notes"
echo "publish: OK — release $tag created on $REPO"
