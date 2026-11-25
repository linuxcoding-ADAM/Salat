#!/bin/sh
# Tests for the release scripts, with no network and no real credential.
#
# * ci/publish.sh runs against a FAKE `gh` that records its calls and answers
#   from FAKE_* variables. Every scenario states whether `gh release create`
#   may be called; the dummy token must never appear in any output or log.
# * ci/auth-check.sh is checked for its no-token refusal.
# * ci/check-release-guard.sh must accept the real CI file and reject
#   deliberately broken copies of it.
#
# Run from the repository root:  sh ci/test-release-scripts.sh
# The t_* tweak functions are called indirectly, through "$tweak" (SC2317, SC2329);
# each case runs in its own subshell on purpose, so its environment cannot leak
# into the next one (SC2030, SC2031); the printf strings below contain a literal
# $ on purpose (SC2016).
# shellcheck disable=SC2317,SC2329,SC2016,SC2030,SC2031
set -eu

ROOT=$(pwd)
CI_FILE=${CI_FILE:-.gitlab-ci.yml}
DUMMY="DUMMY-TOKEN-NOT-A-SECRET-0123456789"
SHA=0123456789abcdef0123456789abcdef01234567

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
BIN="$tmp/bin"
W="$tmp/work"
LOG="$tmp/gh.log"
mkdir "$BIN"

cat >"$BIN/gh" <<'FAKE'
#!/bin/sh
# Fake gh: logs the call (never the token) and answers from FAKE_* variables.
echo "gh $*" >>"$FAKE_LOG"
[ -n "${GH_TOKEN:-}" ] && echo "(GH_TOKEN was set)" >>"$FAKE_LOG"
case "$*" in
    "api repos/linuxcoding-ADAM/Salat --jq .permissions.push") printf '%s\n' "${FAKE_PUSH:-true}" ;;
    "api repos/linuxcoding-ADAM/Salat/commits/"*" --jq .sha")
        [ -n "${FAKE_TAG_SHA:-}" ] || exit 1
        printf '%s\n' "$FAKE_TAG_SHA"
        ;;
    "release view "*) [ "${FAKE_RELEASE_EXISTS:-0}" = 1 ] ;;
    "release create "*) : ;;
    *)
        echo "fake gh: unexpected call: $*" >&2
        exit 99
        ;;
esac
FAKE
chmod +x "$BIN/gh"

pass=0
failed=0
ok() {
    pass=$((pass + 1))
    printf 'PASS  %s\n' "$1"
}
no() {
    failed=$((failed + 1))
    printf 'FAIL  %s\n' "$1"
}

# A fresh, valid release workspace for every case.
fresh_workspace() {
    rm -rf "$W"
    mkdir -p "$W/dist"
    printf 'tarball placeholder\n' >"$W/dist/salat-0.1.0-x86_64-unknown-linux-musl.tar.gz"
    (cd "$W/dist" && sha256sum salat-0.1.0-x86_64-unknown-linux-musl.tar.gz >SHA256SUMS)
    printf '# Changelog\n\n## [0.1.0] - 2026-01-01\n\nFirst release notes.\n\n## [0.0.1]\n\nOld.\n' >"$W/CHANGELOG.md"
}

# try <label> <ok|fail> <tweak-function>
# Runs publish.sh with the valid defaults, after the tweak has changed them.
try() {
    label=$1
    expect=$2
    tweak=$3
    : >"$LOG"
    fresh_workspace
    rc=0
    out=$(
        cd "$W" &&
            export PATH="$BIN:$PATH" FAKE_LOG="$LOG" \
                CI_COMMIT_TAG=v0.1.0 CI_COMMIT_REF_PROTECTED=true \
                GITHUB_RELEASE_TOKEN="$DUMMY" CI_COMMIT_SHA="$SHA" FAKE_TAG_SHA="$SHA" &&
            "$tweak" &&
            sh "$ROOT/ci/publish.sh" 2>&1
    ) || rc=$?
    created=0
    grep -q '^gh release create ' "$LOG" && created=1
    leaked=0
    if printf '%s\n' "$out" | grep -qF "$DUMMY" || grep -qF "$DUMMY" "$LOG"; then leaked=1; fi
    if [ "$leaked" -eq 1 ]; then
        no "$label — THE TOKEN APPEARED IN OUTPUT"
    elif [ "$expect" = ok ] && [ "$rc" -eq 0 ] && [ "$created" -eq 1 ]; then
        ok "$label"
    elif [ "$expect" = fail ] && [ "$rc" -ne 0 ] && [ "$created" -eq 0 ]; then
        ok "$label"
    else
        no "$label (rc=$rc, release created=$created)"
        printf '%s\n' "$out" | sed 's/^/      /'
    fi
}

t_valid() { :; }
t_rc_tag() { export CI_COMMIT_TAG=v0.1.0-rc.1; }
t_bad_tag() { export CI_COMMIT_TAG=v0.1; }
t_unprotected() { export CI_COMMIT_REF_PROTECTED=false; }
t_no_token() { unset GITHUB_RELEASE_TOKEN; }
t_empty_token() { export GITHUB_RELEASE_TOKEN=; }
t_no_tarball() { rm -f dist/*.tar.gz; }
t_no_sums() { rm -f dist/SHA256SUMS; }
t_two_tarballs() { cp dist/salat-0.1.0-x86_64-unknown-linux-musl.tar.gz dist/extra.tar.gz; }
t_tampered() { printf 'tampered\n' >dist/salat-0.1.0-x86_64-unknown-linux-musl.tar.gz; }
t_no_changelog() { printf '# Changelog\n\n## [0.0.1]\n\nOld.\n' >CHANGELOG.md; }
t_empty_section() { printf '# Changelog\n\n## [0.1.0] - 2026-01-01\n\n## [0.0.1]\n\nOld.\n' >CHANGELOG.md; }
t_no_push() { export FAKE_PUSH=false; }
t_tag_not_on_github() { export FAKE_TAG_SHA=; }
t_tag_other_commit() { export FAKE_TAG_SHA=ffffffffffffffffffffffffffffffffffffffff; }
t_release_exists() { export FAKE_RELEASE_EXISTS=1; }

echo "== publish.sh"
try "valid stable tag publishes (and only then calls gh release create)" ok t_valid
try "refuses a prerelease tag" fail t_rc_tag
try "refuses a malformed tag" fail t_bad_tag
try "refuses an unprotected ref" fail t_unprotected
try "refuses when the token is unset" fail t_no_token
try "refuses when the token is empty" fail t_empty_token
try "refuses when the tarball is missing" fail t_no_tarball
try "refuses when SHA256SUMS is missing" fail t_no_sums
try "refuses when there are two tarballs" fail t_two_tarballs
try "refuses a tampered tarball (checksum mismatch)" fail t_tampered
try "refuses when the changelog has no section for the version" fail t_no_changelog
try "refuses when the changelog section is empty" fail t_empty_section
try "refuses when the token has no push access" fail t_no_push
try "refuses when the tag is not on GitHub" fail t_tag_not_on_github
try "refuses when GitHub's tag points at another commit" fail t_tag_other_commit
try "refuses when a release already exists" fail t_release_exists

# The create call must carry exactly the reviewed arguments.
: >"$LOG"
fresh_workspace
(
    cd "$W" &&
        export PATH="$BIN:$PATH" FAKE_LOG="$LOG" CI_COMMIT_TAG=v0.1.0 CI_COMMIT_REF_PROTECTED=true \
            GITHUB_RELEASE_TOKEN="$DUMMY" CI_COMMIT_SHA="$SHA" FAKE_TAG_SHA="$SHA" &&
        sh "$ROOT/ci/publish.sh" >/dev/null 2>&1
) || true
if grep -q '^gh release create v0.1.0 dist/salat-0.1.0-x86_64-unknown-linux-musl.tar.gz dist/SHA256SUMS --repo linuxcoding-ADAM/Salat --verify-tag --title Salat 0.1.0 --notes-file ' "$LOG"; then
    ok "the release is created with the tarball, SHA256SUMS, --verify-tag and notes"
else
    no "the release-create arguments are not the reviewed ones"
fi
if grep -q '(GH_TOKEN was set)' "$LOG"; then
    ok "gh receives the credential as GH_TOKEN"
else
    no "gh did not receive GH_TOKEN"
fi

echo "== auth-check.sh"
rc=0
out=$(env -u GITHUB_RELEASE_TOKEN sh "$ROOT/ci/auth-check.sh" 2>&1) || rc=$?
if [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -q 'not available'; then
    ok "auth-check refuses, with an explanation, when the token is not available"
else
    no "auth-check without a token (rc=$rc)"
fi

echo "== check-release-guard.sh"
guard() { sh "$ROOT/ci/check-release-guard.sh" "$1" >/dev/null 2>&1; }

if guard "$CI_FILE"; then ok "the guard accepts the real CI file"; else no "the guard rejects the real CI file"; fi

mutant() { # mutant <label> <file>
    if guard "$2"; then no "the guard accepted: $1"; else ok "the guard rejects: $1"; fi
}

# 1. an extra, automatic job with the release environment
cp "$CI_FILE" "$tmp/m1.yml"
printf '\nsneaky:\n  stage: check\n  environment: release\n  script:\n    - echo hi\n' >>"$tmp/m1.yml"
mutant "an extra job with environment: release" "$tmp/m1.yml"

# 2. publish starts automatically
sed '/^publish:/,/^[A-Za-z.]/ s/when: manual/when: on_success/' "$CI_FILE" >"$tmp/m2.yml"
mutant "publish that is not manual" "$tmp/m2.yml"

# 3. auth-check no longer limited to protected refs
sed '/^auth-check:/,/^[A-Za-z.]/ s/ && \$CI_COMMIT_REF_PROTECTED == "true"//' "$CI_FILE" >"$tmp/m3.yml"
mutant "auth-check without the protected-ref condition" "$tmp/m3.yml"

# 4. a wrong environment name
sed '/^publish:/,/^[A-Za-z.]/ s/name: release/name: production/' "$CI_FILE" >"$tmp/m4.yml"
mutant "publish in an environment other than release" "$tmp/m4.yml"

# 5. an ordinary job that reads the token
cp "$CI_FILE" "$tmp/m5.yml"
printf '\nleaky:\n  stage: check\n  script:\n    - echo "$GITHUB_RELEASE_TOKEN"\n' >>"$tmp/m5.yml"
mutant "an ordinary job that mentions the token" "$tmp/m5.yml"

# 6. another CI definition pulled in
{ printf 'include:\n  - local: other.yml\n'; cat "$CI_FILE"; } >"$tmp/m6.yml"
mutant "an include: of another CI file" "$tmp/m6.yml"

# 7. an environment on a hidden template
cp "$CI_FILE" "$tmp/m7.yml"
printf '\n.template:\n  environment: release\n' >>"$tmp/m7.yml"
mutant "a hidden template with environment: release" "$tmp/m7.yml"

echo
echo "release-scripts-test: $pass passed, $failed failed"
[ "$failed" -eq 0 ]
