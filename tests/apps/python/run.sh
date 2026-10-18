#!/bin/sh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
command -v python3 >/dev/null || { echo "APP python: COVERAGE GAP (python3 not found)"; exit 2; }
[ -n "${DATABASE_URL:-}" ] || { echo "APP python: COVERAGE GAP (DATABASE_URL unset)"; exit 2; }
VENV="$HERE/.venv"
if [ ! -x "$VENV/bin/python" ]; then
  python3 -m venv "$VENV" >/dev/null 2>&1 || { echo "APP python: COVERAGE GAP (venv creation failed)"; exit 2; }
fi
if ! "$VENV/bin/python" -c "import psycopg" 2>/dev/null; then
  "$VENV/bin/pip" install --quiet "psycopg[binary]" >"$HERE/pip-install.log" 2>&1 || { echo "APP python: COVERAGE GAP (pip install psycopg failed; see tests/apps/python/pip-install.log)"; exit 2; }
fi
"$VENV/bin/python" "$HERE/app.py"
