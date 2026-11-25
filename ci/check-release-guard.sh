#!/bin/sh
# Release guard (docs/ARCHITECTURE.md §22.2, §28 D5): fail if the CI definition
# drifts from the approved release-token design. It checks that
#
#   * exactly two jobs, auth-check and publish, have an `environment:`, and it
#     is `release` (the scope of GITHUB_RELEASE_TOKEN);
#   * every rule of those two jobs is `when: manual` and is limited to
#     protected refs and tags;
#   * no other job uses the token (only `env -u` and the canary's emptiness
#     test are allowed), and no script outside ci/ mentions it;
#   * the file pulls in no other CI definition (`include:`, `trigger:`).
#
# This catches ACCIDENTS. It lives in the same repository as what it checks,
# so a deliberately malicious change could edit it too: changes to
# .gitlab-ci.yml and ci/ still need a line-by-line review.
#
#   sh ci/check-release-guard.sh [path/to/.gitlab-ci.yml]
set -eu

CI_FILE=${1:-.gitlab-ci.yml}
[ -f "$CI_FILE" ] || { echo "release-guard: no such file: $CI_FILE" >&2; exit 2; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0
bad() {
    echo "release-guard: FAIL — $*" >&2
    fail=1
}

# Split the file into one block per top-level key, with comment lines removed.
mkdir "$tmp/blocks"
grep -Ev '^[[:space:]]*#' "$CI_FILE" >"$tmp/stripped"
awk -v dir="$tmp/blocks" '
    /^[A-Za-z0-9_.-]+:/ { name = $1; sub(/:.*/, "", name); out = dir "/" name }
    out != "" { print > out }
' "$tmp/stripped"

# 1. Which blocks declare an environment?
env_jobs=$(
    for f in "$tmp"/blocks/* "$tmp"/blocks/.[!.]*; do
        [ -e "$f" ] || continue
        if grep -Eq '^[[:space:]]+environment:' "$f"; then basename "$f"; fi
    done | sort | tr '\n' ' '
)
[ "$env_jobs" = "auth-check publish " ] \
    || bad "the jobs with an environment must be exactly 'auth-check publish' (found: ${env_jobs:-none})"

# 2. The two release jobs: environment 'release', manual, protected refs only.
for job in auth-check publish; do
    f="$tmp/blocks/$job"
    if [ ! -f "$f" ]; then
        bad "job $job is missing"
        continue
    fi
    grep -Eq 'name:[[:space:]]*release([^A-Za-z0-9_-]|$)' "$f" \
        || bad "$job: the environment name must be 'release'"
    rules=$(grep -Ec '^[[:space:]]+- if:' "$f" || true)
    manual=$(grep -Ec '^[[:space:]]+when:[[:space:]]*manual[[:space:]]*$' "$f" || true)
    other=$(grep -Ec '^[[:space:]]+when:[[:space:]]*(on_success|always|delayed|on_failure)' "$f" || true)
    protected=$(grep -E '^[[:space:]]+- if:' "$f" | grep -c 'CI_COMMIT_REF_PROTECTED == "true"' || true)
    tagged=$(grep -E '^[[:space:]]+- if:' "$f" | grep -c 'CI_COMMIT_TAG' || true)
    [ "$rules" -ge 1 ] || bad "$job: it has no rules"
    [ "$manual" -eq "$rules" ] || bad "$job: every rule must be 'when: manual' ($manual of $rules are)"
    [ "$other" -eq 0 ] || bad "$job: a rule starts automatically"
    [ "$protected" -eq "$rules" ] || bad "$job: every rule must require CI_COMMIT_REF_PROTECTED == \"true\""
    [ "$tagged" -eq "$rules" ] || bad "$job: every rule must require CI_COMMIT_TAG"
done

# 3. No other job in the CI file may use the token. Two protective mentions are
#    allowed everywhere: `env -u GITHUB_RELEASE_TOKEN` (start a command without
#    it) and the emptiness test `[ -n "${GITHUB_RELEASE_TOKEN:-}" ]` of the
#    token-isolation canary. Any other mention is a failure.
for f in "$tmp"/blocks/* "$tmp"/blocks/.[!.]*; do
    [ -e "$f" ] || continue
    job=$(basename "$f")
    case "$job" in auth-check | publish) continue ;; esac
    # shellcheck disable=SC2016  # the pattern contains a literal ${...} on purpose
    if sed -e 's/env -u GITHUB_RELEASE_TOKEN//g' \
        -e 's/\[ -n "\${GITHUB_RELEASE_TOKEN:-}" \]//g' "$f" | grep -q 'GITHUB_RELEASE_TOKEN'; then
        bad "job or section '$job' uses GITHUB_RELEASE_TOKEN"
    fi
done

# 4. No other CI definitions pulled in.
if grep -Eq '^include:' "$tmp/stripped"; then bad "the file uses include:"; fi
if grep -Eq '^[[:space:]]+trigger:' "$tmp/stripped"; then bad "the file uses trigger:"; fi

# 5. No other script outside ci/ mentions the token.
grep -rl 'GITHUB_RELEASE_TOKEN' salat xtask packaging scripts install.sh >"$tmp/token-files" 2>/dev/null || true
while IFS= read -r f; do
    bad "$f mentions GITHUB_RELEASE_TOKEN"
done <"$tmp/token-files"

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "release-guard: OK — only auth-check and publish use the 'release' environment, both manual and protected-only"
