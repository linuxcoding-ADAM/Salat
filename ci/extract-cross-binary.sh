#!/bin/sh
# Extract the binary that `cross build` (remote mode) produced in THIS job
# into dist/salat, and record where it came from (dist/reports/).
#
# Why this exists: in remote mode the Docker daemon runs in the
# Docker-in-Docker service, so cross builds inside Docker volumes on that
# daemon, not in the job's own directory. A throwaway helper container
# mounts each volume read-only and streams the binary out.
#
# Freshness — the output can never be a stale binary:
#   * the dind daemon is a fresh service container for every job, so every
#     volume it holds was created by this job's build;
#   * the job deletes target/<triple>/ before building (see .gitlab-ci.yml);
#   * the binary's modification time must not predate BUILD_START.
# If cross instead copies the binary back into the job's target/ directory,
# that copy is accepted under the same freshness rule and the report says so.
#
# Required environment: HELPER_IMAGE (pinned image with find/stat/cat),
# BUILD_START (epoch seconds taken just before `cross build`).
set -eu

TARGET=x86_64-unknown-linux-musl
REL="$TARGET/release/salat"
OUT=dist/salat
REPORT=dist/reports/build-provenance.txt
SKEW=300 # tolerated clock difference between job and dind containers, seconds

: "${HELPER_IMAGE:?HELPER_IMAGE is not set}"
: "${BUILD_START:?BUILD_START is not set}"

fail() {
    echo "extract: FAIL: $*" >&2
    echo "--- diagnostics: docker volumes" >&2
    docker volume ls >&2 || true
    echo "--- diagnostics: local target/$TARGET" >&2
    ls -la "target/$TARGET/release" >&2 2>/dev/null || echo "(absent)" >&2
    exit 1
}

mkdir -p dist/reports
rm -f "$OUT"
source=""
mtime=""

# 1. The volumes on this job's own Docker daemon.
for vol in $(docker volume ls -q); do
    path=$(docker run --rm -v "$vol:/v:ro" "$HELPER_IMAGE" \
        find /v -type f -path "*/target/$REL" 2>/dev/null | head -n 1) || path=""
    if [ -n "$path" ]; then
        docker run --rm -v "$vol:/v:ro" "$HELPER_IMAGE" cat "$path" >"$OUT"
        mtime=$(docker run --rm -v "$vol:/v:ro" "$HELPER_IMAGE" stat -c %Y "$path")
        source="docker volume $vol: $path"
        break
    fi
done

# 2. A copy cross wrote back into the job directory, if it did.
if [ -z "$source" ] && [ -f "target/$REL" ]; then
    cp "target/$REL" "$OUT"
    mtime=$(stat -c %Y "target/$REL")
    source="job directory (cross copy-back): target/$REL"
fi

[ -n "$source" ] || fail "no freshly built target/$REL in any dind volume or in the job directory"
[ -s "$OUT" ] || fail "$OUT is empty"
# ELF magic: 7f 45 4c 46.
magic=$(od -An -tx1 -N4 "$OUT" | tr -d ' \n')
[ "$magic" = "7f454c46" ] || fail "$OUT is not an ELF file (magic: $magic)"
[ "$mtime" -ge $((BUILD_START - SKEW)) ] ||
    fail "$OUT predates this build (mtime $mtime < build start $BUILD_START - ${SKEW}s): stale output"
chmod 0755 "$OUT"

{
    echo "source:        $source"
    echo "sha256:        $(sha256sum "$OUT" | cut -d' ' -f1)"
    echo "size_bytes:    $(stat -c %s "$OUT")"
    echo "binary_mtime:  $mtime"
    echo "build_start:   $BUILD_START"
    echo "commit:        ${CI_COMMIT_SHA:-unknown}"
    echo "pipeline:      ${CI_PIPELINE_ID:-unknown}"
    echo "job:           ${CI_JOB_ID:-unknown}"
    echo "cross:         $(cross --version 2>/dev/null | head -n 1)"
} | tee "$REPORT"
echo "extract: OK"
