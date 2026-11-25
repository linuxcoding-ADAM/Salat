#!/bin/sh
# `verify-installer-*` jobs: test install.sh against the EXACT tarball and
# SHA256SUMS the `package` job produced (dist/, passed as artifacts — no
# binary is rebuilt here), inside a clean debian:oldstable or alpine image.
#
# The shipped install.sh is never modified. A temporary test copy differs
# from it in exactly one line — the download base — rewritten to a local
# busybox httpd that serves the package artifacts (§28 D4, decision 5).
#
# Interface install.sh must provide for these tests (Stage 3):
#   * exactly one line beginning `DOWNLOAD_BASE=`;
#   * assets fetched from "$DOWNLOAD_BASE/<tag>/<asset>", the same layout as
#     https://github.com/<owner>/<repo>/releases/download/<tag>/<asset>.
# Behaviour under test is §21.2 plus §28 D8 (aarch64 refused).
#
# Usage: sh ci/verify-installer.sh debian|alpine   (runs as root in CI)
set -eu

DISTRO=${1:?usage: verify-installer.sh debian|alpine}
TRIPLE=x86_64-unknown-linux-musl
LOGS="$PWD/verify-logs"
SRV=/srv/releases
PORT=8080
T=/opt/salat-test # test copy and shims, readable by the test user
U=tester
UH=/home/$U
# The test user's PATH: deliberately WITHOUT ~/.local/bin, so the
# installer's PATH advice is exercised.
UPATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin

mkdir -p "$LOGS"
pass=0
failn=0
note() { echo "verify: $*" | tee -a "$LOGS/summary.txt"; }
die() {
    note "SETUP FAIL: $*"
    exit 1
}
result() { # result NAME OK|FAIL [detail]
    if [ "$2" = OK ]; then
        pass=$((pass + 1))
        note "PASS  $1"
    else
        failn=$((failn + 1))
        note "FAIL  $1${3:+ — $3}"
    fi
}

# --- Tools ----------------------------------------------------------------
case "$DISTRO" in
debian)
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends curl ca-certificates busybox >/dev/null
    useradd --create-home --shell /bin/sh "$U"
    MKTAR=tar # GNU tar
    ;;
alpine)
    # GNU tar only to CRAFT the malicious fixtures; removed again before any
    # test so the installer runs against Alpine's own busybox tar.
    # busybox-extras only supplies the httpd that serves the test release;
    # the installer under test still uses Alpine's stock tar and wget.
    apk add --no-cache tar busybox-extras >/dev/null
    adduser -D -s /bin/sh "$U"
    MKTAR=tar
    ;;
*) die "unknown distro '$DISTRO'" ;;
esac

# --- The package artifacts -------------------------------------------------
set -- dist/salat-*-"$TRIPLE".tar.gz
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    die "expected exactly one dist/salat-*-$TRIPLE.tar.gz from the package job"
fi
TARBALL=$1
[ -f dist/SHA256SUMS ] || die "dist/SHA256SUMS missing from the package job"
VERSION=$(basename "$TARBALL" | sed "s/^salat-\(.*\)-$TRIPLE\.tar\.gz$/\1/")
TAG="v$VERSION"
note "artifacts: $TARBALL (version $VERSION, tag $TAG)"
(cd dist && sha256sum -c SHA256SUMS) >/dev/null || die "package artifacts fail their own SHA256SUMS"

publish() { # publish TAG TARBALL SUMSFILE — lay out a fake release directory
    mkdir -p "$SRV/$1"
    cp "$2" "$SRV/$1/"
    cp "$3" "$SRV/$1/SHA256SUMS"
}
publish "$TAG" "$TARBALL" dist/SHA256SUMS

# --- Malicious fixtures (each with a VALID checksum, so the member check
# itself — not the checksum — is what must reject them) ---------------------
FIX=$(mktemp -d)
craft() { # craft LABEL — prepares $FIX/LABEL/<top>/ with a real binary
    v="9.9.9-$1"
    top="salat-$v-$TRIPLE"
    mkdir -p "$FIX/$1/$top"
    tar -xzf "$TARBALL" -O "salat-$VERSION-$TRIPLE/salat" >"$FIX/$1/$top/salat"
    chmod 0755 "$FIX/$1/$top/salat"
}
seal() { # seal LABEL — checksum the crafted tarball and publish it
    v="9.9.9-$1"
    a="salat-$v-$TRIPLE.tar.gz"
    (cd "$FIX/$1" && sha256sum "$a" >SHA256SUMS)
    publish "v$v" "$FIX/$1/$a" "$FIX/$1/SHA256SUMS"
}
# ../evil — a member that climbs out of the extraction directory.
craft dotdot
echo evil >"$FIX/evil"
(cd "$FIX/dotdot" && "$MKTAR" -P -czf "salat-9.9.9-dotdot-$TRIPLE.tar.gz" "salat-9.9.9-dotdot-$TRIPLE" ../evil)
seal dotdot
# /absolute path member.
craft abs
echo evil >/tmp/salat-abs-evil
(cd "$FIX/abs" && "$MKTAR" -P -czf "salat-9.9.9-abs-$TRIPLE.tar.gz" "salat-9.9.9-abs-$TRIPLE" /tmp/salat-abs-evil)
rm -f /tmp/salat-abs-evil
seal abs
# The binary replaced by a symlink.
craft symlink
rm "$FIX/symlink/salat-9.9.9-symlink-$TRIPLE/salat"
ln -s /bin/sh "$FIX/symlink/salat-9.9.9-symlink-$TRIPLE/salat"
(cd "$FIX/symlink" && "$MKTAR" -czf "salat-9.9.9-symlink-$TRIPLE.tar.gz" "salat-9.9.9-symlink-$TRIPLE")
seal symlink
# A FIFO — a non-regular, non-directory member (§21.2 step 11).
craft fifo
mkfifo "$FIX/fifo/salat-9.9.9-fifo-$TRIPLE/pipe"
(cd "$FIX/fifo" && "$MKTAR" -czf "salat-9.9.9-fifo-$TRIPLE.tar.gz" "salat-9.9.9-fifo-$TRIPLE")
seal fifo
# A device node (§21.5). Needs CAP_MKNOD; if the runner withholds it the
# FIFO case above still covers the same "not a regular file or directory"
# rule, and the summary says so explicitly.
craft device
if mknod "$FIX/device/salat-9.9.9-device-$TRIPLE/null" c 1 3 2>/dev/null; then
    (cd "$FIX/device" && "$MKTAR" -czf "salat-9.9.9-device-$TRIPLE.tar.gz" "salat-9.9.9-device-$TRIPLE")
    seal device
    HAVE_DEVICE=1
else
    HAVE_DEVICE=0
    note "NOTE  device-node fixture not creatable on this runner (no CAP_MKNOD); covered by the FIFO case"
fi
# Tampered SHA256SUMS for the genuine tarball: flip the first hex digit.
mkdir -p "$FIX/tamper"
ta="salat-9.9.9-tamper-$TRIPLE.tar.gz"
cp "$TARBALL" "$FIX/tamper/$ta"
good=$(sha256sum "$FIX/tamper/$ta" | cut -d' ' -f1)
first=$(printf %s "$good" | cut -c1)
flip=0
if [ "$first" = 0 ]; then flip=1; fi
printf '%s%s  %s\n' "$flip" "$(printf %s "$good" | cut -c2-)" "$ta" >"$FIX/tamper/SHA256SUMS"
publish v9.9.9-tamper "$FIX/tamper/$ta" "$FIX/tamper/SHA256SUMS"

if [ "$DISTRO" = alpine ]; then
    apk del --no-cache tar >/dev/null
    tar --help 2>&1 | grep -qi busybox || die "GNU tar still active; the installer must run against busybox tar"
    note "alpine: GNU tar removed; installer runs with busybox tar and busybox wget"
fi

# --- Serve the fake releases ------------------------------------------------
if [ "$DISTRO" = alpine ]; then
    httpd -p "127.0.0.1:$PORT" -h "$SRV"
else
    busybox httpd -p "127.0.0.1:$PORT" -h "$SRV"
fi
i=0
until busybox wget -q -O /dev/null "http://127.0.0.1:$PORT/$TAG/SHA256SUMS" 2>/dev/null; do
    i=$((i + 1))
    [ "$i" -lt 20 ] || die "local release server did not start"
    sleep 0.5
done

# --- The test copy: exactly one line differs ---------------------------------
[ -f install.sh ] || die "install.sh is missing"
n=$(grep -c '^DOWNLOAD_BASE=' install.sh || true)
[ "$n" = 1 ] || die "install.sh must contain exactly one 'DOWNLOAD_BASE=' line (found $n)"
mkdir -p "$T"
sed "s|^DOWNLOAD_BASE=.*|DOWNLOAD_BASE=\"http://127.0.0.1:$PORT\"|" install.sh >"$T/install.sh"
changed=$(diff install.sh "$T/install.sh" | grep -c '^[<>]' || true)
[ "$changed" = 2 ] || die "the test copy must differ from install.sh in exactly one line (diff: $changed lines)"
note "test copy: install.sh with only DOWNLOAD_BASE rewritten to http://127.0.0.1:$PORT"

# Shim that makes `uname -m` report another machine (D8 test).
REAL_UNAME=$(command -v uname)
for arch in aarch64 arm64; do
    mkdir -p "$T/shim-$arch"
    # shellcheck disable=SC2016 # the $1/$@ are meant for the generated shim, not this shell
    printf '#!/bin/sh\nif [ "${1:-}" = -m ]; then echo %s; else exec %s "$@"; fi\n' "$arch" "$REAL_UNAME" >"$T/shim-$arch/uname"
    chmod 0755 "$T/shim-$arch/uname"
done
# A PATH holding every common tool EXCEPT sha256sum and shasum.
mkdir -p "$T/nosum"
for c in sh dash bash cat cp chmod cut dirname basename env grep head id install ln ls \
    mkdir mktemp mv rm rmdir sed sort tail tar tr uname wc curl wget gzip gunzip printf \
    test expr tee awk find readlink sleep date touch stat true false od; do
    p=$(command -v "$c" 2>/dev/null || true)
    case "$p" in /*) ln -sf "$p" "$T/nosum/$c" ;; esac
done
chmod -R a+rX "$T"

# --- Runners ------------------------------------------------------------------
# as_user LOG PATH ARGS... — run the test copy as the unprivileged user.
as_user() {
    log=$1
    path=$2
    shift 2
    su "$U" -s /bin/sh -c "cd $UH && HOME=$UH PATH=$path sh $T/install.sh $*" >"$LOGS/$log" 2>&1 </dev/null
}
bin_hash() { sha256sum "$1" 2>/dev/null | cut -d' ' -f1; }

# --- Tests ------------------------------------------------------------------
# 1. Root is refused (§21.2 step 3) and nothing is installed for root.
rc=0
HOME=/root PATH="$UPATH" sh "$T/install.sh" --version "$TAG" >"$LOGS/01-root-refused.log" 2>&1 </dev/null || rc=$?
if [ "$rc" = 1 ] && [ ! -e /root/.local/bin/salat ]; then
    result "root refused (exit 1)" OK
else
    result "root refused" FAIL "exit $rc"
fi

# 2. A normal install as the unprivileged user.
for f in .profile .bashrc .zshrc; do echo "# untouched" >"$UH/$f"; done
chown "$U" "$UH/.profile" "$UH/.bashrc" "$UH/.zshrc"
rc_before=$(cat "$UH/.profile" "$UH/.bashrc" "$UH/.zshrc" | sha256sum)
rc=0
as_user 02-install.log "$UPATH" --version "$TAG" || rc=$?
B="$UH/.local/bin/salat"
if [ "$rc" = 0 ] && [ -f "$B" ]; then
    result "install: exit 0, binary present" OK
else
    result "install" FAIL "exit $rc"
fi
mode=$(stat -c %a "$B" 2>/dev/null || echo none)
if [ "$mode" = 755 ]; then result "install: mode 0755" OK; else result "install: mode 0755" FAIL "mode $mode"; fi
got=$(su "$U" -s /bin/sh -c "$B --version" 2>&1 || true)
if [ "$got" = "salat $VERSION" ]; then
    result "install: --version = salat $VERSION" OK
else
    result "install: --version" FAIL "$got"
fi
packaged=$(tar -xzf "$TARBALL" -O "salat-$VERSION-$TRIPLE/salat" | sha256sum | cut -d' ' -f1)
if [ "$(bin_hash "$B")" = "$packaged" ]; then
    result "install: binary is the packaged binary, byte for byte" OK
else
    result "install: binary identity" FAIL
fi
if grep -q 'export PATH=' "$LOGS/02-install.log"; then
    result "install: prints the export PATH line" OK
else
    result "install: PATH advice" FAIL
fi
if [ "$(cat "$UH/.profile" "$UH/.bashrc" "$UH/.zshrc" | sha256sum)" = "$rc_before" ]; then
    result "install: no shell rc file modified" OK
else
    result "install: rc files" FAIL "an rc file changed"
fi

# 3. Re-running at the same version stops early; --force reinstalls.
h1=$(bin_hash "$B")
rc=0
as_user 03-same-version.log "$UPATH" --version "$TAG" || rc=$?
if [ "$rc" = 0 ] && grep -q 'already at' "$LOGS/03-same-version.log" && [ "$(bin_hash "$B")" = "$h1" ]; then
    result "re-run: 'already at', binary untouched" OK
else
    result "re-run" FAIL "exit $rc"
fi
rc=0
as_user 04-force.log "$UPATH" --version "$TAG" --force || rc=$?
if [ "$rc" = 0 ] && ! grep -q 'already at' "$LOGS/04-force.log" && [ "$(bin_hash "$B")" = "$h1" ]; then
    result "--force: reinstalled" OK
else
    result "--force" FAIL "exit $rc"
fi

# 4. --prefix installs elsewhere.
rc=0
as_user 05-prefix.log "$UPATH" --version "$TAG" --prefix "$UH/custom-bin" || rc=$?
if [ "$rc" = 0 ] && [ -x "$UH/custom-bin/salat" ]; then
    result "--prefix: installed into the given directory" OK
else
    result "--prefix" FAIL "exit $rc"
fi

# 5. Tampered SHA256SUMS: refused, nothing installed.
rc=0
as_user 06-tampered.log "$UPATH" --version v9.9.9-tamper --prefix "$UH/p-tamper" || rc=$?
if [ "$rc" != 0 ] && [ ! -e "$UH/p-tamper/salat" ]; then
    result "tampered SHA256SUMS: refused, nothing installed" OK
else
    result "tampered SHA256SUMS" FAIL "exit $rc"
fi

# 6. Malicious archives: rejected even though their checksums are valid.
rm -f "$UH/evil" /tmp/salat-abs-evil
for case in dotdot abs symlink fifo device; do
    if [ "$case" = device ] && [ "$HAVE_DEVICE" = 0 ]; then continue; fi
    rc=0
    as_user "07-malicious-$case.log" "$UPATH" --version "v9.9.9-$case" --prefix "$UH/p-$case" || rc=$?
    if [ "$rc" != 0 ] && [ ! -e "$UH/p-$case/salat" ] && [ ! -e "$UH/evil" ] && [ ! -e /tmp/salat-abs-evil ]; then
        result "malicious archive ($case): rejected, nothing written" OK
    else
        result "malicious archive ($case)" FAIL "exit $rc"
    fi
done

# 7. No checksum tool at all: abort, never install unverified.
rc=0
as_user 08-no-checksum-tool.log "$T/nosum" --version "$TAG" --prefix "$UH/p-nosum" || rc=$?
if [ "$rc" != 0 ] && [ ! -e "$UH/p-nosum/salat" ]; then
    result "no sha256sum/shasum: aborted, nothing installed" OK
else
    result "no checksum tool" FAIL "exit $rc"
fi

# 8. aarch64 / arm64: the D8 refusal, before any download.
for arch in aarch64 arm64; do
    rc=0
    as_user "09-unsupported-$arch.log" "$T/shim-$arch:$UPATH" --version "$TAG" --prefix "$UH/p-$arch" || rc=$?
    if [ "$rc" = 1 ] && grep -q 'aarch64 is not supported yet; supported: x86_64' "$LOGS/09-unsupported-$arch.log" &&
        [ ! -e "$UH/p-$arch/salat" ]; then
        result "uname -m = $arch: refused with the D8 message" OK
    else
        result "uname -m = $arch" FAIL "exit $rc"
    fi
done

# 9. Uninstall removes the binary and only LISTS the user's data.
for d in .config/salat .local/share/salat .local/state/salat; do
    mkdir -p "$UH/$d"
    echo keep >"$UH/$d/keep"
done
chown -R "$U" "$UH/.config" "$UH/.local"
rc=0
as_user 10-uninstall.log "$UPATH" --uninstall || rc=$?
if [ "$rc" = 0 ] && [ ! -e "$B" ] && [ -f "$UH/.config/salat/keep" ] && [ -f "$UH/.local/share/salat/keep" ] &&
    [ -f "$UH/.local/state/salat/keep" ] && grep -q '\.config/salat' "$LOGS/10-uninstall.log"; then
    result "--uninstall: binary removed, data listed and kept" OK
else
    result "--uninstall" FAIL "exit $rc"
fi
# --purge with no terminal to confirm on: deletes nothing.
rc=0
as_user 11-purge-no-tty.log "$UPATH" --uninstall --purge || rc=$?
if [ "$rc" != 0 ] && [ -f "$UH/.config/salat/keep" ]; then
    result "--uninstall --purge without a terminal: refused, data kept" OK
else
    result "--purge without confirmation" FAIL "exit $rc"
fi

# --- Summary ------------------------------------------------------------------
note "$DISTRO: $pass passed, $failn failed"
[ "$failn" = 0 ]
