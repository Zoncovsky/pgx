#!/bin/sh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
command -v go >/dev/null || { echo "APP go: COVERAGE GAP (go not found)"; exit 2; }
[ -n "${DATABASE_URL:-}" ] || { echo "APP go: COVERAGE GAP (DATABASE_URL unset)"; exit 2; }
cd "$HERE"
if [ ! -f go.sum ]; then
  go get github.com/jackc/pgx/v5@latest >"$HERE/go-get.log" 2>&1 || { echo "APP go: COVERAGE GAP (go get pgx/v5 failed; see tests/apps/go/go-get.log)"; exit 2; }
fi
go run . 2>"$HERE/go-run.log" || { echo "APP go: FAIL (see tests/apps/go/go-run.log)"; cat "$HERE/go-run.log" >&2; exit 1; }
