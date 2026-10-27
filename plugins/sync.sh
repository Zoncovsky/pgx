#!/usr/bin/env bash
# Package the canonical Skill (skills/pgx) for Claude Code.
#
#   plugins/sync.sh                         skills/pgx -> plugins/pgx/skills/pgx
#   plugins/sync.sh --check                 exit 1 if plugins/pgx/skills/pgx drifted
#   plugins/sync.sh --standalone <dir>      assemble the standalone plugin repository
#                                           (github.com/pgrundev/pgx-plugin) into <dir>
#   plugins/sync.sh --check-standalone <dir>  exit 1 if a pgx-plugin checkout drifted
#
# The plugin copy of the skill is generated: edit skills/pgx, run this script,
# commit both. CI runs `--check` so the two copies cannot diverge.
#
# The standalone repository is generated too. It is exactly:
#   plugins/pgx/*            the plugin package (manifest, README, skills/pgx)
#   LICENSE                  the repository licence (AGPL-3.0), required by the directory
#   plugins/standalone/*     the overlay only that repository needs: its one-entry
#                            marketplace manifest and its validation workflow
# `--standalone` replaces everything in <dir> except .git, so deletions propagate.
# .github/workflows/publish-plugin.yml publishes the result from main; the
# standalone repository's own workflow runs `--check-standalone` against main.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
src="$repo/skills/pgx"
dst="$repo/plugins/pgx/skills/pgx"
overlay="$repo/plugins/standalone"

assemble_standalone() { # $1: destination (created; contents other than .git replaced)
  local out=$1
  [ "$out" != "/" ] && [ "$out" != "$HOME" ] && [ "$out" != "$repo" ] || { echo "refusing to assemble into $out" >&2; exit 1; }
  if [ -d "$out" ]; then
    # Only replace a directory that is empty, a git checkout, or a previous assembly.
    if [ -n "$(ls -A "$out")" ] && [ ! -d "$out/.git" ] && [ ! -f "$out/.claude-plugin/plugin.json" ]; then
      echo "refusing to replace the contents of $out (not empty, not a git checkout, not a plugin)" >&2; exit 1
    fi
    find "$out" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
  else
    mkdir -p "$out"
  fi
  cp -R "$repo/plugins/pgx/." "$out/"
  cp "$repo/LICENSE" "$out/LICENSE"
  cp -R "$overlay/." "$out/"
  find "$out" -name .DS_Store -delete
  if find "$out" -path "$out/.git" -prune -o -type l -print | grep -q .; then echo "the standalone tree must not contain symlinks" >&2; exit 1; fi
}

case "${1:-}" in
  --check)
    if [ -d "$dst" ] && diff -r "$src" "$dst" >/dev/null; then exit 0; fi
    echo "plugins/pgx/skills/pgx is out of date with skills/pgx; run plugins/sync.sh" >&2
    diff -r "$src" "$dst" 2>&1 | head -20 >&2 || true
    exit 1 ;;
  --standalone)
    out=${2:?usage: plugins/sync.sh --standalone <dir>}
    "$0" --check
    assemble_standalone "$out"
    files=$(find "$out" -path "$out/.git" -prune -o -type f -print | wc -l | tr -d ' ')
    bytes=$(find "$out" -path "$out/.git" -prune -o -type f -print0 | xargs -0 cat | wc -c | tr -d ' ')
    echo "assembled the standalone plugin repository in $out ($files files, $((bytes / 1024)) KB)" ;;
  --check-standalone)
    dir=${2:?usage: plugins/sync.sh --check-standalone <dir>}
    [ -d "$dir" ] || { echo "no such directory: $dir" >&2; exit 1; }
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/pgx-standalone.XXXX"); trap 'rm -rf "$tmp"' EXIT
    assemble_standalone "$tmp/expected" >/dev/null
    if diff -r --exclude=.git "$tmp/expected" "$dir" >"$tmp/diff" 2>&1; then
      echo "standalone plugin repository matches plugins/pgx + LICENSE + plugins/standalone"; exit 0
    fi
    echo "standalone plugin repository drifted from the canonical plugin (run plugins/sync.sh --standalone $dir):" >&2
    head -40 "$tmp/diff" >&2
    exit 1 ;;
  "")
    if find "$src" -type l | grep -q .; then echo "skills/pgx must not contain symlinks" >&2; exit 1; fi
    rm -rf "$dst"
    mkdir -p "$(dirname "$dst")"
    cp -R "$src" "$dst"
    find "$dst" -name .DS_Store -delete
    echo "synced skills/pgx -> plugins/pgx/skills/pgx ($(find "$dst" -type f | wc -l | tr -d ' ') files)" ;;
  *) echo "usage: plugins/sync.sh [--check | --standalone <dir> | --check-standalone <dir>]" >&2; exit 2 ;;
esac
