#!/bin/sh
# Install the GitHub CLI from its official release, verified against a pinned
# SHA-256 (GH_VERSION and GH_SHA256 come from .gitlab-ci.yml). This replaces
# `apk add github-cli`: nothing unpinned is installed in a job that can later
# receive the release token. Run it BEFORE the token is needed, and without it:
#   env -u GITHUB_RELEASE_TOKEN sh ci/install-gh.sh
set -eu

: "${GH_VERSION:?GH_VERSION is not set}"
: "${GH_SHA256:?GH_SHA256 is not set}"

name="gh_${GH_VERSION}_linux_amd64"
url="https://github.com/cli/cli/releases/download/v${GH_VERSION}/${name}.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

wget -q -O "$tmp/gh.tgz" "$url"
echo "${GH_SHA256}  $tmp/gh.tgz" | sha256sum -c -
tar -xzf "$tmp/gh.tgz" -C "$tmp" "$name/bin/gh"
install -m 0755 "$tmp/$name/bin/gh" /usr/local/bin/gh
gh --version | head -n 1
