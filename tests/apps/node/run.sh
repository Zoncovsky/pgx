#!/bin/sh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
command -v node >/dev/null || { echo "APP node: COVERAGE GAP (node not found)"; exit 2; }
[ -n "${DATABASE_URL:-}" ] || { echo "APP node: COVERAGE GAP (DATABASE_URL unset)"; exit 2; }
if [ ! -d "$HERE/node_modules/pg" ]; then
  ( cd "$HERE" && npm install --no-audit --no-fund --loglevel=error ) >/"$HERE/npm-install.log" 2>&1 || { echo "APP node: COVERAGE GAP (npm install pg failed; see tests/apps/node/npm-install.log)"; exit 2; }
fi
node "$HERE/app.js"
