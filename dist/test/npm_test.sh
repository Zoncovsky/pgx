#!/bin/sh
# End-to-end test of the npm wrapper (dist/npm, published as `pgxdb`) against a
# REAL assembled PGX distribution served over local HTTP as a release would be:
# `node dist/npm/bin/pgxdb.js` with no arguments must download + verify the
# tarball, cache it, start PGX and print the `app` connection URL; further
# invocations must reuse the cache and pass every CLI command through.
#
#   PGX_TARBALL_DIR=<dir with pgx-<v>-<os>-<arch>.tar.gz(.sha256)> dist/test/npm_test.sh
#   (without PGX_TARBALL_DIR it builds a debug pgx and assembles one; needs
#    PG18_BINDIR or initdb on PATH, and target/release/postgres)
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/pgx-npm-test.XXXXXX")"
SRV_PID=""
cleanup() {
  [ -n "$SRV_PID" ] && kill "$SRV_PID" 2>/dev/null || true
  PGX_HOME="$TMP/home" "$TMP/opt/bin/pgx" stop >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

OS="$(uname -s | tr 'A-Z' 'a-z')"; [ "$OS" = darwin ] || OS=linux
ARCH="$(uname -m)"; case "$ARCH" in x86_64|amd64) ARCH=amd64;; aarch64|arm64) ARCH=arm64;; esac

if [ -n "${PGX_TARBALL_DIR:-}" ]; then
  TGZ="$(find "$PGX_TARBALL_DIR" -maxdepth 1 -name "pgx-*-$OS-$ARCH.tar.gz" | head -1)"
  [ -n "$TGZ" ] || fail "no pgx-*-$OS-$ARCH.tar.gz in $PGX_TARBALL_DIR"
  VERSION="$(basename "$TGZ" | sed -n "s/^pgx-\(.*\)-$OS-$ARCH\.tar\.gz$/\1/p")"
else
  VERSION="0.0.0-npm-test"
  ( cd "$ROOT" && CC=/usr/bin/cc cargo build -q -p pgx ) || fail "cargo build -p pgx"
  "$ROOT/dist/assemble.sh" --out "$TMP/stage" --version "$VERSION" \
    --pgx "$ROOT/target/debug/pgx" ${PG18_BINDIR:+--pg-bindir "$PG18_BINDIR"} >/dev/null || fail "assemble"
  TGZ="$TMP/stage/pgx-$VERSION-$OS-$ARCH.tar.gz"
fi
NAME="pgx-$VERSION-$OS-$ARCH"
[ -f "$TGZ" ] && [ -f "$TGZ.sha256" ] || fail "missing $TGZ or its .sha256"

# Release layout as GitHub serves downloads: <base>/v<version>/<name>.tar.gz
mkdir -p "$TMP/srv/v$VERSION"
cp "$TGZ" "$TGZ.sha256" "$TMP/srv/v$VERSION/"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
( cd "$TMP/srv" && python3 -m http.server "$PORT" --bind 127.0.0.1 >"$TMP/srv.log" 2>&1 ) &
SRV_PID=$!
sleep 1

export PGX_VERSION="$VERSION" PGX_BASE_URL="http://127.0.0.1:$PORT" PGX_INSTALL_DIR="$TMP/opt"
export PGX_HOME="$TMP/home"
# No system postgres/psql on PATH: the wrapper and PGX must not need them.
NODE="$(command -v node)"; PATH="$(dirname "$NODE"):/usr/bin:/bin:/usr/sbin:/sbin"; export PATH
PGXDB="$ROOT/dist/npm/bin/pgxdb.js"

echo "## 1. no arguments == pgx run: downloads, verifies, starts, prints the URL"
node "$PGXDB" >"$TMP/run.out" 2>"$TMP/run.err" || { cat "$TMP/run.out" "$TMP/run.err"; fail "pgxdb (no args) exited non-zero"; }
grep -q "downloading PGX $VERSION" "$TMP/run.err" || { cat "$TMP/run.err"; fail "did not download"; }
grep -q "PGX is ready" "$TMP/run.out" || { cat "$TMP/run.out"; fail "no 'PGX is ready'"; }
URL="$(grep -o 'postgres://[^ ]*' "$TMP/run.out" | head -1)"
[ -n "$URL" ] || fail "no connection URL printed"
[ -x "$TMP/opt/bin/pgx" ] || fail "distribution not cached under PGX_INSTALL_DIR"
ok "started; URL $(echo "$URL" | sed 's/:[^:@]*@/:***@/')"

echo "## 2. commands pass through, --json works, cache is reused (no second download)"
node "$PGXDB" --json ls >"$TMP/ls.json" 2>"$TMP/ls.err" || fail "pgxdb --json ls"
grep -q '"app"' "$TMP/ls.json" || { cat "$TMP/ls.json"; fail "ls --json lacks app"; }
[ ! -s "$TMP/ls.err" ] || { cat "$TMP/ls.err"; fail "cache hit still wrote to stderr"; }
[ "$(grep -c "$NAME.tar.gz HTTP" "$TMP/srv.log" || true)" = 1 ] || { cat "$TMP/srv.log"; fail "tarball fetched more than once"; }
node "$PGXDB" version | grep -q "$VERSION" || fail "pgxdb version != $VERSION"
node "$PGXDB" url app | grep -q '^postgres://' || fail "pgxdb url app"
ok "ls/version/url pass through; one download total"

echo "## 3. exit codes propagate (drop of a missing database -> 5)"
set +e; node "$PGXDB" drop no-such-db >/dev/null 2>&1; rc=$?; set -e
[ "$rc" = 5 ] || fail "expected exit 5 for a missing database, got $rc"
ok "exit code 5 propagated"

echo "## 4. stop through the wrapper"
node "$PGXDB" stop >/dev/null || fail "pgxdb stop"
set +e; node "$PGXDB" status >/dev/null 2>&1; rc=$?; set -e
[ "$rc" = 3 ] || fail "expected status exit 3 after stop, got $rc"
echo "PASS: npm wrapper downloads, verifies, caches, runs PGX and passes commands through ($OS $ARCH)"
