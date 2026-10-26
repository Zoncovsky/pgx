#!/usr/bin/env bash
# Tests for scripts/check-pgx.sh: missing, too old, current, malformed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); chk="$here/../../pgx/scripts/check-pgx.sh"
fail=0; ok() { echo "ok   $1"; }; bad() { echo "FAIL $1"; fail=1; }
mk() { # name version-json-or-garbage
  d=$(mktemp -d); printf '#!/bin/sh\ncase "$*" in *version*) printf %%s %s ;; *agent*--help*) echo "pgx help"; exit 0 ;; esac\n' "'$2'" > "$d/pgx"; chmod +x "$d/pgx"; echo "$d"
}
run() { PATH="$1:/usr/bin:/bin" bash "$chk"; }
set +e
out=$(PATH=/usr/bin:/bin bash "$chk"); rc=$?; [ $rc = 3 ] && [[ $out == *'"reason":"missing"'* ]] && ok "missing → 3" || bad "missing: rc=$rc $out"
d=$(mk old '{"pgx_version":"0.1.0","os":"darwin"}'); out=$(run "$d"); rc=$?; [ $rc = 2 ] && [[ $out == *'"too_old"'* ]] && [[ $out == *"no pgx agent"* ]] && ok "0.1.0 → 2 (too old) even though agent --help exits 0" || bad "old: rc=$rc $out"
d=$(mk sess '{"pgx_version":"0.1.1","os":"darwin"}'); out=$(run "$d"); rc=$?; [ $rc = 2 ] && [[ $out == *'"too_old"'* ]] && [[ $out == *subagents* ]] && ok "0.1.1 → 2 (session isolation only; the hint names subagents)" || bad "0.1.1: rc=$rc $out"
d=$(mk cur '{"pgx_version":"0.1.2","os":"darwin"}'); out=$(run "$d"); rc=$?; [ $rc = 0 ] && [[ $out == *'"ok":true'* ]] && ok "0.1.2 → 0" || bad "current: rc=$rc $out"
d=$(mk env '{"pgx_version":"0.1.1","os":"darwin"}'); out=$(PGX_MIN_VERSION=0.1.1 PATH="$d:/usr/bin:/bin" bash "$chk"); rc=$?; [ $rc = 0 ] && ok "PGX_MIN_VERSION override still works" || bad "override: rc=$rc $out"
d=$(mk new '{"pgx_version":"0.2.0","os":"linux"}'); out=$(run "$d"); rc=$?; [ $rc = 0 ] && ok "0.2.0 → 0" || bad "newer: rc=$rc $out"
d=$(mk ci '{"pgx_version":"0.0.0-ci.abc","os":"linux"}'); out=$(run "$d"); rc=$?; [ $rc = 2 ] && ok "0.0.0-ci → 2" || bad "ci build: rc=$rc $out"
d=$(mk bad 'not json at all'); out=$(run "$d"); rc=$?; [ $rc = 4 ] && [[ $out == *'"malformed"'* ]] && ok "garbage → 4 (malformed)" || bad "malformed: rc=$rc $out"
d=$(mk empty ''); out=$(run "$d"); rc=$?; [ $rc = 4 ] && ok "empty → 4" || bad "empty: rc=$rc $out"
for o in "$(PATH=/usr/bin:/bin bash "$chk")" "$(run "$(mk x '{"pgx_version":"0.1.0"}')")"; do python3 -I -c 'import json,sys; json.loads(sys.argv[1])' "$o" && ok "output is JSON" || bad "not JSON: $o"; done
set -e
[ $fail = 0 ] && echo "CHECK-PGX TESTS PASSED" || { echo "CHECK-PGX TESTS FAILED"; exit 1; }
