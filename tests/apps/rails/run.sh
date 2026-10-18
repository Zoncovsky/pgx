#!/bin/sh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
command -v ruby >/dev/null || { echo "APP rails: COVERAGE GAP (ruby not found)"; exit 2; }
[ -n "${DATABASE_URL:-}" ] || { echo "APP rails: COVERAGE GAP (DATABASE_URL unset)"; exit 2; }
ruby -e 'require "active_record"; require "pg"' 2>/dev/null || { echo "APP rails: COVERAGE GAP (activerecord/pg gems not installed)"; exit 2; }
ruby "$HERE/app.rb"
