#!/bin/sh
# `verify-version` job (docs/ARCHITECTURE.md §22.2): the Git tag must equal
# the salat crate's Cargo version exactly, so a v0.2.0 release can never
# ship a binary that reports 0.1.0. Prerelease versions are accepted
# (v0.1.0-rc.1 needs Cargo.toml at 0.1.0-rc.1); build metadata is not.
set -eu

tag=${CI_COMMIT_TAG:-}
[ -n "$tag" ] || { echo "verify-version: not a tag pipeline (CI_COMMIT_TAG is empty)" >&2; exit 1; }

num='(0|[1-9][0-9]*)'
pre='[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*'
if ! printf '%s\n' "$tag" | grep -Eq "^v${num}\.${num}\.${num}(-${pre})?\$"; then
    echo "verify-version: tag '$tag' is not vMAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH-PRERELEASE" >&2
    exit 1
fi

# --no-deps: reads the workspace manifests only, downloads nothing.
version=$(cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select(.name == "salat") | .version')
case "$version" in
    "" | *[!0-9A-Za-z.+-]*) echo "verify-version: could not read the salat version from cargo metadata" >&2; exit 1 ;;
esac

if [ "${tag#v}" != "$version" ]; then
    echo "verify-version: FAIL — tag '$tag' expects version '${tag#v}', but Cargo.toml says '$version'" >&2
    exit 1
fi
echo "verify-version: OK — tag $tag == salat $version"
