#!/bin/sh
# Behavioural compatibility suite: every step runs on PGX and, when a
# PostgreSQL 18 control is available, on PostgreSQL too; rows and SQLSTATEs
# must match. Minitest + the pg gem (ruby, libpq).
#
#   PGX_DIST=<prefix> [PG18_BINDIR=<dir with initdb,pg_ctl>] tests/compat/run.sh
#
# Without PG18_BINDIR the suite asserts PostgreSQL's known behaviour only and
# prints `COVERAGE GAP: no PostgreSQL control`.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
[ -n "${PGX_DIST:-}" ] || { echo "PGX_DIST is not set (dist/assemble.sh --out ...)" >&2; exit 2; }
command -v ruby >/dev/null || { echo "COVERAGE GAP: ruby not installed" >&2; exit 2; }
ruby -e 'require "pg"; require "minitest"' 2>/dev/null || { echo "COVERAGE GAP: ruby gems pg/minitest missing (gem install pg minitest)" >&2; exit 2; }

SCRATCH="${PGX_TEST_SCRATCH:-/tmp/pgxt}"; mkdir -p "$SCRATCH"
export PGX_HOME="$SCRATCH/compat-$$"
CONTROL_DIR="$SCRATCH/compat-ctl-$$"
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
PORT="$(free_port)"
cleanup() {
  "$PGX_DIST/bin/pgx" stop --force >/dev/null 2>&1 || true
  [ -n "${PG18_BINDIR:-}" ] && [ -d "$CONTROL_DIR" ] && "$PG18_BINDIR/pg_ctl" -D "$CONTROL_DIR" -m immediate stop >/dev/null 2>&1 || true
  [ -n "${PGX_TEST_KEEP:-}" ] || rm -rf "$PGX_HOME" "$CONTROL_DIR"
}
trap cleanup EXIT INT TERM

"$PGX_DIST/bin/pgx" init --port "$PORT" >/dev/null
"$PGX_DIST/bin/pgx" start >/dev/null
"$PGX_DIST/bin/pgx" create compat >/dev/null
PGX_COMPAT_URL="$("$PGX_DIST/bin/pgx" url compat)"; export PGX_COMPAT_URL

if [ -n "${PG18_BINDIR:-}" ]; then
  CPORT="$(free_port)"
  "$PG18_BINDIR/initdb" -D "$CONTROL_DIR" --no-locale -E UTF8 -U postgres -A trust >/dev/null
  "$PG18_BINDIR/pg_ctl" -D "$CONTROL_DIR" -o "-p $CPORT -k $CONTROL_DIR -c listen_addresses=127.0.0.1" -l "$CONTROL_DIR/log" -w start >/dev/null
  "$PG18_BINDIR/createdb" -h 127.0.0.1 -p "$CPORT" -U postgres pgx_compat_control
  PGX_COMPAT_CONTROL_URL="postgres://postgres@127.0.0.1:$CPORT/pgx_compat_control"; export PGX_COMPAT_CONTROL_URL
  echo "control: PostgreSQL $("$PG18_BINDIR/postgres" --version | awk '{print $3}') on port $CPORT"
else
  echo "COVERAGE GAP: no PostgreSQL control (set PG18_BINDIR); asserting known PostgreSQL behaviour only"
fi
echo "target:  $("$PGX_DIST/bin/pgx" version | head -1)"

cd "$HERE"
set +e
ruby -I. -e 'require "minitest/autorun"; Dir["./*_test.rb"].sort.each { |f| require f }' -- "$@"
status=$?
set -e
exit $status
