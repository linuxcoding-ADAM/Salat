#!/bin/sh
# `package` job: verify the binary that `build-musl` produced (dist/salat,
# passed as an artifact — never rebuilt here), then assemble the release
# tarball and SHA256SUMS exactly as docs/ARCHITECTURE.md §22.2 specifies.
#
# Every check fails the job on a mismatch. Reports go to dist/reports/.
set -eu

TARGET=x86_64-unknown-linux-musl
BIN=dist/salat
R=dist/reports

fail() {
    echo "package: FAIL: $*" >&2
    exit 1
}
ok() { echo "package: ok   $*"; }

mkdir -p "$R"
[ -s "$BIN" ] || fail "$BIN is missing or empty — the build-musl artifact did not arrive"
chmod 0755 "$BIN"

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' salat/Cargo.toml | head -n 1)
[ -n "$VERSION" ] || fail "could not read the version from salat/Cargo.toml"
ok "version from Cargo.toml: $VERSION"

# --- Architecture ---------------------------------------------------------
file "$BIN" | tee "$R/file.txt"
readelf -h "$BIN" >"$R/readelf-header.txt"
grep -Eq 'Class:[[:space:]]+ELF64' "$R/readelf-header.txt" || fail "not a 64-bit ELF"
grep -Eq 'Machine:[[:space:]]+Advanced Micro Devices X86-64' "$R/readelf-header.txt" ||
    fail "not an x86-64 binary: $(grep Machine: "$R/readelf-header.txt")"
grep -q 'x86-64' "$R/file.txt" || fail "file(1) does not report x86-64"
ok "architecture: ELF64 x86-64"

# --- Static linking -------------------------------------------------------
readelf -l "$BIN" >"$R/readelf-program-headers.txt"
if grep -q 'INTERP' "$R/readelf-program-headers.txt"; then
    fail "has a program interpreter: dynamically linked"
fi
readelf -d "$BIN" >"$R/readelf-dynamic.txt" 2>&1 || true
if grep -q '(NEEDED)' "$R/readelf-dynamic.txt"; then
    fail "depends on shared libraries: $(grep '(NEEDED)' "$R/readelf-dynamic.txt" | tr -s ' ')"
fi
ldd "$BIN" >"$R/ldd.txt" 2>&1 || true # ldd exits non-zero for static binaries
grep -Eq 'not a dynamic executable|statically linked' "$R/ldd.txt" ||
    fail "ldd does not report a static binary: $(cat "$R/ldd.txt")"
grep -Eq 'static(-pie)? linked|statically linked' "$R/file.txt" || fail "file(1) does not report static linking"
ok "static: no interpreter, no NEEDED libraries, ldd: $(head -n 1 "$R/ldd.txt" | tr -s ' \t' ' ')"

# --- Stripped -------------------------------------------------------------
readelf -S "$BIN" >"$R/sections.txt"
if grep -q '\.symtab' "$R/sections.txt"; then
    fail "symbol table present: not stripped"
fi
grep -q ', stripped' "$R/file.txt" || fail "file(1) does not report stripped"
ok "stripped: no .symtab"

# --- Smoke tests (isolated HOME/XDG, no network) ---------------------------
SMOKE=$(mktemp -d)
export HOME="$SMOKE/home" XDG_CONFIG_HOME="$SMOKE/config" XDG_DATA_HOME="$SMOKE/data" \
    XDG_STATE_HOME="$SMOKE/state" XDG_RUNTIME_DIR="$SMOKE/run" NO_COLOR=1
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR"
chmod 0700 "$XDG_RUNTIME_DIR"

got=$("$BIN" --version)
[ "$got" = "salat $VERSION" ] || fail "--version printed '$got', expected 'salat $VERSION'"
ok "--version: $got"

"$BIN" --help >"$R/smoke-help.txt" || fail "--help exited non-zero"
grep -q -- '--full' "$R/smoke-help.txt" || fail "--help does not list --full"
ok "--help: exit 0"

# `salat config` is load-or-create: on a fresh machine it writes the default
# configuration, then prints the §8.1 resolution lines and the TOML.
"$BIN" config >"$R/smoke-config.txt" || fail "'salat config' exited non-zero"
for key in scope: config: database: lock:; do
    grep -q "^$key" "$R/smoke-config.txt" || fail "'salat config' output lacks '$key'"
done
grep -q '^\[location\]' "$R/smoke-config.txt" || fail "'salat config' output lacks [location]"
[ -s "$XDG_CONFIG_HOME/salat/config.toml" ] || fail "'salat config' did not create the default config"
ok "salat config: started, created the default config, printed resolution + TOML"

# --- Tarball (§22.2 layout) ------------------------------------------------
NAME="salat-$VERSION-$TARGET"
STAGE=$(mktemp -d)
mkdir "$STAGE/$NAME"
install -m 0755 "$BIN" "$STAGE/$NAME/salat"
install -m 0644 README.md LICENSE CHANGELOG.md "$STAGE/$NAME/"
install -m 0644 packaging/systemd/salat.service "$STAGE/$NAME/salat.service"

# Reproducible: fixed order, owner and mtime (the commit time), no gzip name.
EPOCH=$(date -d "${CI_COMMIT_TIMESTAMP:?}" +%s)
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$EPOCH" \
    -C "$STAGE" -cf - "$NAME" | gzip -9n >"dist/$NAME.tar.gz"

tar -tzvf "dist/$NAME.tar.gz" | tee "$R/tarball-listing.txt"
expected="$NAME/
$NAME/CHANGELOG.md
$NAME/LICENSE
$NAME/README.md
$NAME/salat
$NAME/salat.service"
members=$(tar -tzf "dist/$NAME.tar.gz")
[ "$members" = "$expected" ] || fail "tarball members differ from §22.2:
$members"
# Only regular files and the one directory; the binary is mode 0755.
if awk '{print substr($1,1,1)}' "$R/tarball-listing.txt" | grep -qv '[-d]'; then
    fail "tarball contains a member that is not a regular file or directory"
fi
grep -Eq '^-rwxr-xr-x .* '"$NAME"'/salat$' "$R/tarball-listing.txt" || fail "salat is not mode 0755 in the tarball"
ok "tarball: one top-level directory, the five §22.2 members, salat 0755"

# --- SHA256SUMS -----------------------------------------------------------
(cd dist && sha256sum "$NAME.tar.gz" >SHA256SUMS && sha256sum -c SHA256SUMS) || fail "SHA256SUMS self-check failed"
cp dist/SHA256SUMS "$R/SHA256SUMS.txt"
ok "SHA256SUMS: $(cat dist/SHA256SUMS)"

# --- Sizes ----------------------------------------------------------------
bin_bytes=$(stat -c %s "$BIN")
tgz_bytes=$(stat -c %s "dist/$NAME.tar.gz")
[ "$tgz_bytes" -ge 100000 ] || fail "tarball is implausibly small ($tgz_bytes bytes)"
{
    echo "version:          $VERSION"
    echo "binary (stripped): $bin_bytes bytes ($(awk "BEGIN{printf \"%.2f\", $bin_bytes/1048576}") MiB)"
    echo "tarball (gzip -9): $tgz_bytes bytes ($(awk "BEGIN{printf \"%.2f\", $tgz_bytes/1048576}") MiB)"
    echo "plan target:       ~4-5 MB gzipped (Phase 14 expected results)"
} | tee "$R/sizes.txt"

echo "package: OK — dist/$NAME.tar.gz and dist/SHA256SUMS"
