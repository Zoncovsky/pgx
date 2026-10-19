#!/bin/sh
# Assert a macOS distribution is self-contained: no shipped Mach-O may load a
# non-system library by absolute path (e.g. Homebrew /opt/homebrew, /usr/local).
# Allowed: /usr/lib/* and /System/* (the macOS system), and bundled references
# via @rpath/@loader_path/@executable_path resolving inside the dist. Runs
# recursively over bin/* and any lib/*.dylib. Exits nonzero on the first stray.
#
#   dist/test/macos_selfcontained_test.sh <dist-dir>
set -eu
DIST="${1:?usage: macos_selfcontained_test.sh <dist-dir>}"
[ "$(uname -s)" = Darwin ] || { echo "skip: not macOS"; exit 0; }
fail=0
check() {
  f="$1"
  # Load-command dylib paths, minus the install id line (first otool -L row).
  otool -L "$f" | tail -n +2 | awk '{print $1}' | while IFS= read -r dep; do
    case "$dep" in
      /usr/lib/*|/System/*) ;;                         # macOS system: ok
      @rpath/*|@loader_path/*|@executable_path/*) ;;    # bundled/relocatable: ok
      "") ;;
      *) echo "STRAY: $f -> $dep"; echo stray > "$DIST/.stray" ;;
    esac
  done
}
rm -f "$DIST/.stray"
for f in "$DIST"/bin/*; do [ -f "$f" ] && check "$f"; done
if [ -d "$DIST/lib" ]; then
  for f in "$DIST"/lib/*.dylib; do [ -e "$f" ] && check "$f"; done
fi
if [ -f "$DIST/.stray" ]; then rm -f "$DIST/.stray"; echo "FAIL: dist links non-system libraries by absolute path (not self-contained)"; exit 1; fi
echo "PASS: all shipped Mach-O load only system (/usr/lib,/System) or bundled (@rpath) libraries"
