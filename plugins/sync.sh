#!/usr/bin/env bash
# Package the canonical Skill (skills/pgx) into the Claude Code plugin
# (plugins/pgx/skills/pgx). The plugin copy is generated: edit skills/pgx,
# run this script, commit both. `plugins/sync.sh --check` only reports drift
# (exit 1), which CI runs so the two copies cannot diverge.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
src="$repo/skills/pgx"
dst="$repo/plugins/pgx/skills/pgx"
if [ "${1:-}" = "--check" ]; then
  if [ -d "$dst" ] && diff -r "$src" "$dst" >/dev/null; then exit 0; fi
  echo "plugins/pgx/skills/pgx is out of date with skills/pgx; run plugins/sync.sh" >&2
  diff -r "$src" "$dst" 2>&1 | head -20 >&2 || true
  exit 1
fi
if find "$src" -type l | grep -q .; then echo "skills/pgx must not contain symlinks" >&2; exit 1; fi
rm -rf "$dst"
mkdir -p "$(dirname "$dst")"
cp -R "$src" "$dst"
find "$dst" -name .DS_Store -delete
echo "synced skills/pgx -> plugins/pgx/skills/pgx ($(find "$dst" -type f | wc -l | tr -d ' ') files)"
