#!/bin/sh
# Salat installer — docs/ARCHITECTURE.md §21.
#
#   curl -fsSL https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/main/install.sh | sh
#
# Installs one statically linked binary to ~/.local/bin (or --prefix). It
# never uses sudo, never runs as root unless SALAT_ALLOW_ROOT=1, never edits
# a shell rc file, and never installs anything it has not verified: the
# tarball's SHA-256 is checked against the release's SHA256SUMS BEFORE it is
# unpacked, and the archive's member list is validated BEFORE extraction.
#
# v0.1.0 supports x86_64 Linux only (aarch64 is deferred, §28 D8).

set -eu
IFS='
'
umask 022

REPO="linuxcoding-ADAM/Salat"
BIN="salat"
TRIPLE="x86_64-unknown-linux-musl"
# Release assets are fetched from "$DOWNLOAD_BASE/<tag>/<asset>", the layout
# GitHub Releases use. CI tests rewrite exactly this one line (§28 D4).
DOWNLOAD_BASE="https://github.com/$REPO/releases/download"
MAX_TARBALL_BYTES=104857600

say() { printf 'salat: %s\n' "$*"; }
warn() { printf 'salat: %s\n' "$*" >&2; }
die() {
    warn "error: $*"
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<'EOF'
Usage: install.sh [--prefix DIR] [--version vX.Y.Z] [--force]
       install.sh --uninstall [--purge]
       install.sh --help

Installs salat to ~/.local/bin (override with --prefix DIR or
SALAT_INSTALL_DIR). Without --version it installs the latest release.

  --prefix DIR     install into DIR instead of ~/.local/bin
  --version TAG    install this release, e.g. v0.1.0 or v0.1.0-rc.1
  --force          reinstall even if that version is already installed
  --uninstall      remove the binary and LIST your data (config, database,
                   logs, systemd unit) with the commands to delete it
  --purge          with --uninstall: also delete that data, after you
                   confirm. Needs a terminal: download the script and run
                   it directly instead of piping it into sh
  --help           show this help

Supported: Linux on x86_64.
EOF
}

usage_error() {
    warn "$1"
    usage >&2
    exit 2
}

valid_tag() {
    [ "${#1}" -le 64 ] && printf '%s\n' "$1" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
}

# --- Arguments --------------------------------------------------------------
PREFIX_ARG=""
TAG=""
FORCE=0
UNINSTALL=0
PURGE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
    --prefix)
        [ "$#" -ge 2 ] || usage_error "--prefix needs a directory"
        PREFIX_ARG=$2
        shift 2
        ;;
    --prefix=*)
        PREFIX_ARG=${1#--prefix=}
        shift
        ;;
    --version)
        [ "$#" -ge 2 ] || usage_error "--version needs a tag such as v0.1.0"
        TAG=$2
        shift 2
        ;;
    --version=*)
        TAG=${1#--version=}
        shift
        ;;
    --force)
        FORCE=1
        shift
        ;;
    --uninstall)
        UNINSTALL=1
        shift
        ;;
    --purge)
        PURGE=1
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *) usage_error "unknown option: $1" ;;
    esac
done
if [ "$PURGE" = 1 ] && [ "$UNINSTALL" = 0 ]; then usage_error "--purge only works together with --uninstall"; fi
if [ "$UNINSTALL" = 1 ] && { [ -n "$TAG" ] || [ "$FORCE" = 1 ]; }; then
    usage_error "--uninstall cannot be combined with --version or --force"
fi
if [ -n "$TAG" ] && ! valid_tag "$TAG"; then usage_error "invalid version '$TAG' (expected vX.Y.Z or vX.Y.Z-PRERELEASE)"; fi

# --- Refuse root (§21.2 step 3) --------------------------------------------
if [ "$(id -u)" = 0 ] && [ "${SALAT_ALLOW_ROOT:-}" != 1 ]; then
    warn "refusing to run as root: salat is a per-user tool and installs into your home directory."
    warn "run this as your normal user (set SALAT_ALLOW_ROOT=1 only if you really mean it)."
    exit 1
fi

if [ -z "${HOME:-}" ]; then die "\$HOME is not set"; fi
case "$HOME" in
/*) ;;
*) die "\$HOME is not an absolute path: $HOME" ;;
esac

PREFIX=${PREFIX_ARG:-${SALAT_INSTALL_DIR:-$HOME/.local/bin}}
case "$PREFIX" in
/*) ;;
*) PREFIX="$PWD/$PREFIX" ;;
esac
PREFIX=${PREFIX%/}
[ -n "$PREFIX" ] || PREFIX=/

# --- Uninstall --------------------------------------------------------------
xdg_dir() { # xdg_dir VALUE DEFAULT — XDG values must be absolute
    case "$1" in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$2" ;;
    esac
}

if [ "$UNINSTALL" = 1 ]; then
    CFG=$(xdg_dir "${XDG_CONFIG_HOME:-}" "$HOME/.config")
    DATA=$(xdg_dir "${XDG_DATA_HOME:-}" "$HOME/.local/share")
    STATE=$(xdg_dir "${XDG_STATE_HOME:-}" "$HOME/.local/state")
    UNIT="$CFG/systemd/user/salat.service"

    # Deleting data needs a person at a terminal. Decide that first, so a
    # refused purge changes nothing at all.
    if [ "$PURGE" = 1 ] && [ ! -t 0 ]; then
        warn "refusing to --purge without a terminal to confirm on: nothing was changed."
        warn "download install.sh and run it directly (sh install.sh --uninstall --purge)."
        exit 1
    fi

    if [ -e "$PREFIX/$BIN" ] || [ -L "$PREFIX/$BIN" ]; then
        rm -f "$PREFIX/$BIN"
        say "removed $PREFIX/$BIN"
    else
        say "no salat binary at $PREFIX/$BIN"
    fi

    found=0
    for d in "$CFG/salat" "$DATA/salat" "$STATE/salat"; do
        if [ -e "$d" ]; then
            [ "$found" = 1 ] || say "your data was NOT deleted. It is still here:"
            found=1
            say "  $d    (delete with: rm -rf '$d')"
        fi
    done
    if [ -e "$UNIT" ]; then
        [ "$found" = 1 ] || say "your data was NOT deleted. It is still here:"
        found=1
        say "  $UNIT    (stop it first: systemctl --user disable --now salat; then: rm -f '$UNIT')"
    fi
    [ "$found" = 1 ] || say "no salat data found."

    if [ "$PURGE" = 1 ] && [ "$found" = 1 ]; then
        printf 'salat: type "yes" to permanently delete the data listed above: '
        ans=""
        read -r ans || ans=""
        if [ "$ans" != yes ]; then
            say "nothing deleted."
            exit 1
        fi
        for d in "$CFG/salat" "$DATA/salat" "$STATE/salat"; do
            if [ -e "$d" ]; then
                rm -rf "$d"
                say "deleted $d"
            fi
        done
        if [ -e "$UNIT" ]; then
            rm -f "$UNIT"
            say "deleted $UNIT"
        fi
    fi
    exit 0
fi

# --- Platform (§21.2 steps 4-5) ---------------------------------------------
if [ "$(uname -s)" != Linux ]; then die "salat currently supports Linux only"; fi
case "$(uname -m)" in
x86_64 | amd64) ;;
aarch64 | arm64)
    warn "aarch64 is not supported yet; supported: x86_64"
    exit 1
    ;;
*)
    warn "unsupported architecture '$(uname -m)'; supported: x86_64"
    exit 1
    ;;
esac

# --- Required tools (§21.2 step 6) ------------------------------------------
if have curl; then
    DL=curl
elif have wget; then
    DL=wget
else
    die "need curl or wget to download the release"
fi
have tar || die "need tar"
if have sha256sum; then
    SUMTOOL=sha256sum
elif have shasum; then
    SUMTOOL=shasum
else
    die "need sha256sum or shasum: refusing to install anything that cannot be verified"
fi

sha256_of() {
    if [ "$SUMTOOL" = sha256sum ]; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

fetch() { # fetch URL DEST
    if [ "$DL" = curl ]; then
        curl -fsSL --connect-timeout 20 --max-time 600 --max-redirs 5 -o "$2" "$1"
    else
        wget -q -T 60 -O "$2" "$1"
    fi
}

# --- Scratch space (step 8) -------------------------------------------------
TMP=$(mktemp -d) || die "cannot create a temporary directory"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# --- Which release (step 7) -------------------------------------------------
if [ -z "$TAG" ]; then
    u=""
    if [ "$DL" = curl ]; then
        u=$(curl -fsSIL --max-redirs 5 -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null) || u=""
    fi
    case "$u" in
    */releases/tag/*) TAG=${u##*/} ;;
    esac
    if ! valid_tag "$TAG"; then
        TAG=""
        if fetch "https://api.github.com/repos/$REPO/releases/latest" "$TMP/latest.json" 2>/dev/null; then
            TAG=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$TMP/latest.json" | head -n 1)
        fi
    fi
    valid_tag "$TAG" || die "could not determine the latest release; pass --version vX.Y.Z"
fi
VER=${TAG#v}

# --- Upgrade / already installed ---------------------------------------------
HAVE=""
if [ -x "$PREFIX/$BIN" ]; then
    HAVE=$("$PREFIX/$BIN" --version 2>/dev/null | sed -n 's/^salat //p' | head -n 1) || HAVE=""
fi
if [ -n "$HAVE" ] && [ "$HAVE" = "$VER" ] && [ "$FORCE" = 0 ]; then
    say "already at $VER ($PREFIX/$BIN); use --force to reinstall"
    exit 0
fi
if [ -n "$HAVE" ] && [ "$HAVE" = "$VER" ]; then
    say "reinstalling $VER"
elif [ -n "$HAVE" ]; then
    say "upgrading $HAVE → $VER"
fi

# --- Download (step 9) ------------------------------------------------------
ASSET="salat-$VER-$TRIPLE.tar.gz"
TOP="salat-$VER-$TRIPLE"
ASSET_URL="$DOWNLOAD_BASE/$TAG/$ASSET"
SUMS_URL="$DOWNLOAD_BASE/$TAG/SHA256SUMS"
say "downloading $ASSET_URL"
fetch "$ASSET_URL" "$TMP/$ASSET" || die "download failed: $ASSET_URL"
fetch "$SUMS_URL" "$TMP/SHA256SUMS" || die "download failed: $SUMS_URL"
size=$(wc -c <"$TMP/$ASSET" | tr -d ' ')
if [ "$size" -le 0 ] || [ "$size" -gt "$MAX_TARBALL_BYTES" ]; then die "unexpected download size: $size bytes"; fi

# --- Verify BEFORE extracting (step 10) -------------------------------------
want=$(awk -v a="$ASSET" '$2 == a || $2 == "*" a { print $1 }' "$TMP/SHA256SUMS")
case "$want" in
*'
'*) die "SHA256SUMS lists $ASSET more than once" ;;
esac
printf '%s\n' "$want" | grep -Eq '^[0-9a-f]{64}$' || die "SHA256SUMS has no valid entry for $ASSET"
got=$(sha256_of "$TMP/$ASSET")
printf '%s\n' "$got" | grep -Eq '^[0-9a-f]{64}$' || die "could not compute the SHA-256 of the download"
if [ "$got" != "$want" ]; then
    warn "error: checksum mismatch for $ASSET: refusing to install"
    warn "  expected: $want"
    warn "  actual:   $got"
    exit 1
fi
say "verified sha256 $got"

# --- Validate the member list BEFORE extracting (step 11) --------------------
tar -tzf "$TMP/$ASSET" >"$TMP/names" 2>/dev/null || die "cannot read the archive"
tar -tzvf "$TMP/$ASSET" >"$TMP/verbose" 2>/dev/null || die "cannot read the archive"
n1=$(wc -l <"$TMP/names" | tr -d ' ')
n2=$(wc -l <"$TMP/verbose" | tr -d ' ')
if [ "$n1" -le 0 ] || [ "$n1" -ne "$n2" ]; then die "the archive listing is inconsistent: refusing to extract"; fi
have_bin=0
while IFS= read -r name; do
    case "$name" in
    '' | /*) die "unsafe archive member (empty or absolute path): '$name'" ;;
    esac
    case "/$name/" in
    */../*) die "unsafe archive member (contains '..'): '$name'" ;;
    esac
    case "$name" in
    "$TOP" | "$TOP/" | "$TOP"/*) ;;
    *) die "unexpected archive member outside $TOP/: '$name'" ;;
    esac
    if [ "$name" = "$TOP/$BIN" ]; then have_bin=1; fi
done <"$TMP/names"
[ "$have_bin" = 1 ] || die "the archive has no $TOP/$BIN"
while IFS= read -r line; do
    case "$line" in
    -* | d*) ;;
    *) die "unsafe archive member (not a regular file or directory): $line" ;;
    esac
done <"$TMP/verbose"

mkdir "$TMP/x"
tar -xzf "$TMP/$ASSET" -C "$TMP/x" || die "extraction failed"
SRC="$TMP/x/$TOP/$BIN"
if [ ! -f "$SRC" ] || [ -L "$SRC" ]; then die "$TOP/$BIN is not a regular file"; fi

# --- Install (steps 12-13) ---------------------------------------------------
if ! install -d -m 0755 "$PREFIX" 2>/dev/null; then
    die "cannot create $PREFIX; choose another directory with --prefix DIR"
fi
STAGED="$PREFIX/.$BIN.new.$$"
if ! install -m 0755 "$SRC" "$STAGED" 2>/dev/null; then
    rm -f "$STAGED"
    die "cannot write to $PREFIX; choose another directory with --prefix DIR"
fi
# Smoke-test the staged file first, so a bad download can never replace a
# working installation.
out=$("$STAGED" --version 2>/dev/null) || out=""
if [ "$out" != "salat $VER" ]; then
    rm -f "$STAGED"
    die "the downloaded binary failed its smoke test (--version printed '$out', expected 'salat $VER'); nothing was installed"
fi
mv -f "$STAGED" "$PREFIX/$BIN" || {
    rm -f "$STAGED"
    die "cannot move the binary into place in $PREFIX"
}

# --- Tell the user (steps 14-15) ---------------------------------------------
case ":${PATH:-}:" in
*":$PREFIX:"*) ;;
*)
    say "$PREFIX is not on your PATH. Add it for this shell and future ones:"
    case "$(basename "${SHELL:-sh}")" in
    fish) say "  fish_add_path \"$PREFIX\"" ;;
    *) say "  export PATH=\"$PREFIX:\$PATH\"    # add this line to your shell's startup file" ;;
    esac
    ;;
esac
say "installed salat $VER to $PREFIX/$BIN"
say "  from:   $ASSET_URL"
say "  sha256: $got"
say "run: salat"
