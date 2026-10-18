#!/bin/sh
# ZFS storage checks for PGX: block cloning through `pgx clone`, and snapshot
# consistency (a ZFS snapshot of the data dataset taken mid-workload, cloned,
# and opened by a second runtime). Runs ONLY on a host with a real ZFS dataset
# the current user may snapshot/clone; it never mocks ZFS.
#
#   PGX_DIST=<prefix> tests/pgx/storage_zfs.sh <pool/dataset>
#
# Exit 0 pass, 1 fail, 2 COVERAGE GAP (no dataset given or zfs unavailable).
set -eu
DATASET="${1:-}"
[ -n "${PGX_DIST:-}" ] || { echo "PGX_DIST is not set" >&2; exit 2; }
if [ -z "$DATASET" ] || ! command -v zfs >/dev/null 2>&1; then
  echo "COVERAGE GAP: ZFS — no dataset given or zfs(8) not installed; ZFS behaviour is UNVERIFIED on this host"; exit 2
fi
MNT="$(zfs get -H -o value mountpoint "$DATASET")"
[ -d "$MNT" ] || { echo "dataset $DATASET is not mounted" >&2; exit 2; }
PGX="$PGX_DIST/bin/pgx"
export PGX_HOME="$MNT/pgx-zfs-$$"
export PGX_RUN_DIR="/tmp/pgxz$$"
SNAP="$DATASET@pgx-test-$$"
CLONE="$DATASET-pgx-clone-$$"
cleanup() {
  "$PGX" stop --force >/dev/null 2>&1 || true
  [ -n "${PGX2_HOME:-}" ] && PGX_HOME="$PGX2_HOME" PGX_RUN_DIR="$PGX_RUN_DIR" "$PGX" stop --force >/dev/null 2>&1 || true
  zfs destroy -f "$CLONE" >/dev/null 2>&1 || true
  zfs destroy "$SNAP" >/dev/null 2>&1 || true
  rm -rf "$PGX_HOME" "$PGX_RUN_DIR"
}
trap cleanup EXIT INT TERM
fail() { echo "FAIL: $*" >&2; exit 1; }
psqlq() { python3 - "$@" <<'PY'
import sys, os
sys.path.insert(0, os.path.join(os.environ["REPO"], "tests", "pgx"))
from lib import pgclient
dir_, port, db, sql = sys.argv[1:5]
with pgclient.connect_unix(dir_, int(port), db) as c:
    r = c.query(sql)
    print("\n".join("|".join("" if v is None else v for v in row) for row in r.rows))
PY
}
export REPO="$(cd "$(dirname "$0")/../.." && pwd)"

echo "== init on $DATASET ($MNT), filesystem $(zfs get -H -o value type "$DATASET")"
"$PGX" init >/dev/null
"$PGX" start >/dev/null
PORT="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$PGX_HOME/config.json")"
"$PGX" doctor --json | python3 -c 'import json,sys; d=json.load(sys.stdin); fs=[c for c in d["checks"] if c["name"]=="filesystem"][0]; print("filesystem check:", fs["status"], fs["detail"]); sys.exit(0 if "block cloning" in fs["detail"] and fs["status"]=="ok" else 1)' || fail "pgx doctor does not see block cloning on ZFS (needs the block_cloning feature enabled on the pool)"

echo "== block cloning through pgx clone"
"$PGX" create src >/dev/null
psqlq "$PGX_RUN_DIR" "$PORT" src "CREATE TABLE t AS SELECT g id, md5(g::text) v FROM generate_series(1, 2000000) g; CHECKPOINT" >/dev/null
SRC_MD5="$(psqlq "$PGX_RUN_DIR" "$PORT" src "SELECT md5(string_agg(id || v, ',' ORDER BY id)) FROM t")"
USED_BEFORE="$(zfs get -Hp -o value used "$DATASET")"
"$PGX" clone src c1 >/dev/null
sync; sleep 2
USED_AFTER="$(zfs get -Hp -o value used "$DATASET")"
SRC_BYTES="$(psqlq "$PGX_RUN_DIR" "$PORT" postgres "SELECT pg_database_size('src')")"
echo "   source $((SRC_BYTES / 1048576)) MB; dataset grew $(( (USED_AFTER - USED_BEFORE) / 1048576 )) MB on clone"
[ $((USED_AFTER - USED_BEFORE)) -lt $((SRC_BYTES / 4)) ] || fail "clone consumed more than a quarter of the source: block cloning not effective"
[ "$(psqlq "$PGX_RUN_DIR" "$PORT" c1 "SELECT md5(string_agg(id || v, ',' ORDER BY id)) FROM t")" = "$SRC_MD5" ] || fail "clone content differs from source"

echo "== ZFS snapshot consistency under load"
python3 - "$PGX_RUN_DIR" "$PORT" <<'PY' &
import sys, os, time
sys.path.insert(0, os.path.join(os.environ["REPO"], "tests", "pgx"))
from lib import pgclient
with pgclient.connect_unix(sys.argv[1], int(sys.argv[2]), "src") as c:
    c.query("CREATE TABLE w (id int PRIMARY KEY, b int)")
    b = 0
    end = time.time() + 20
    while time.time() < end:
        b += 1
        c.query("BEGIN"); c.query("INSERT INTO w SELECT g, %d FROM generate_series(%d, %d) g" % (b, (b-1)*500+1, b*500)); c.query("COMMIT")
PY
WRITER=$!
sleep 5
zfs snapshot "$SNAP"
zfs clone "$CLONE" >/dev/null 2>&1 || zfs clone "$SNAP" "$CLONE"
wait $WRITER || true
CMNT="$(zfs get -H -o value mountpoint "$CLONE")"
export PGX2_HOME="$CMNT/$(basename "$PGX_HOME")"
rm -f "$PGX2_HOME/data/postmaster.pid"
# second runtime on the snapshot clone: new port, same run dir
python3 - "$PGX2_HOME/config.json" <<'PY'
import json, sys, socket
p = sys.argv[1]; d = json.load(open(p)); s = socket.socket(); s.bind(("127.0.0.1", 0)); d["port"] = s.getsockname()[1]; s.close()
json.dump(d, open(p, "w"))
PY
PORT2="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$PGX2_HOME/config.json")"
PGX_HOME="$PGX2_HOME" "$PGX" start >/dev/null || fail "runtime on the ZFS snapshot clone did not start (recovery failed)"
ROWS="$(psqlq "$PGX_RUN_DIR" "$PORT2" src "SELECT count(*), count(DISTINCT b), coalesce(max(b),0) FROM w")"
echo "   snapshot clone: rows|batches|maxbatch = $ROWS"
PARTIAL="$(psqlq "$PGX_RUN_DIR" "$PORT2" src "SELECT count(*) FROM (SELECT b, count(*) n FROM w GROUP BY b HAVING count(*) <> 500) x")"
[ "$PARTIAL" = "0" ] || fail "$PARTIAL partial batch(es) visible in the snapshot clone: snapshot is not transactionally consistent"
[ "$(psqlq "$PGX_RUN_DIR" "$PORT2" src "SELECT md5(string_agg(id || v, ',' ORDER BY id)) FROM t")" = "$SRC_MD5" ] || fail "table t differs in the snapshot clone"
echo "PASS: ZFS block cloning and snapshot consistency verified on $DATASET"
