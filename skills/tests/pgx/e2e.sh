#!/usr/bin/env bash
# End-to-end evaluation of the pgx Skill's promises against a real PGX runtime
# (and a real scratch PostgreSQL for the import scenario). No paid agent calls:
# this drives the same commands the Skill tells an agent to run.
#
#   PGX_DIST=~/.pgx/dist/0.1.0 PGX_BIN=target/release/pgx PG_BINDIR=/opt/homebrew/opt/postgresql@18/bin skills/tests/pgx/e2e.sh
#
# PGX_DIST is an assembled distribution (the engine and share tree); the pgx
# binary under test is copied next to it, exactly like the Rust test harness.
# Scenarios: A fresh, B existing-postgres, C pgx-only, D concurrent,
# E lifecycle, F security, G installation-ux. SKIP_NPX=1 skips the skills-CLI
# part of G (offline CI). Each scenario uses its own PGX_HOME and HOME under a
# short scratch root; runtimes are stopped at exit, directories are kept.
set -uo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
skill="$repo/skills/pgx"
PGX_BIN=${PGX_BIN:-$repo/target/release/pgx}
PGX_BIN=$(cd "$(dirname "$PGX_BIN")" && pwd)/$(basename "$PGX_BIN")
PG_BINDIR=${PG_BINDIR:-${PGX_TEST_PG_BINDIR:-${PG18_BINDIR:-}}}
[ -x "$PGX_BIN" ] || { echo "no pgx binary at $PGX_BIN (build with cargo build -p pgx --release)"; exit 2; }
PGX_DIST=${PGX_DIST:-$(ls -d "$HOME"/.pgx/dist/*/ 2>/dev/null | sort -V | tail -1)}
[ -n "$PGX_DIST" ] && [ -x "$PGX_DIST/bin/postgres" ] || { echo "PGX_DIST must point at an assembled distribution (bin/postgres, share/)"; exit 2; }
root=$(mktemp -d /tmp/pgxe2e.XXXX)
echo "scratch: $root"
# A prefix with the binary under test beside the distribution's engine and share tree.
mkdir -p "$root/prefix/bin"; cp "$PGX_BIN" "$root/prefix/bin/pgx"; ln -s "$PGX_DIST/bin/postgres" "$root/prefix/bin/postgres"; ln -s "${PGX_DIST%/}/share" "$root/prefix/share"
PGX_BIN="$root/prefix/bin/pgx"
ORIG_PATH=$PATH
fails=0; passes=0
pass() { echo "pass  $1"; passes=$((passes+1)); }
fail() { echo "FAIL  $1"; fails=$((fails+1)); }
cleanup() {
  for h in "$root"/*/home; do [ -d "$h" ] && PGX_HOME=$h "$PGX_BIN" stop --force >/dev/null 2>&1; done
  [ -n "${PGDATA_SCRATCH:-}" ] && "$PG_BINDIR/pg_ctl" -D "$PGDATA_SCRATCH" stop -m fast >/dev/null 2>&1
  true
}
trap cleanup EXIT
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }
json() { python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {"d": d}))' "$1"; }
# scenario <name>: sets PGX_HOME, HOME, PROJ, PGX_PORT for a fresh environment and cds into PROJ.
scenario() {
  export PGX_HOME="$root/$1/home" HOME="$root/$1/userhome" PGX_PORT; PGX_PORT=$(free_port)
  PROJ="$root/$1/proj"; mkdir -p "$PGX_HOME" "$HOME" "$PROJ/.git"; cd "$PROJ"
  export PATH="$(dirname "$PGX_BIN"):$PG_BINDIR:$ORIG_PATH"
  unset CLAUDE_CODE_SESSION_ID CODEX_SESSION_ID CLAUDE_ENV_FILE CLAUDE_PROJECT_DIR CURSOR_VERSION PGX_SOURCE DATABASE_URL PGHOST PGPORT PGUSER PGPASSWORD PGDATABASE
}
pgx() { "$PGX_BIN" "$@"; }

# ---- A. fresh machine: no pgx on PATH, then install path present --------------
scenario fresh
out=$(PATH=/usr/bin:/bin bash "$skill/scripts/detect-project.sh" .)
[ "$(printf '%s' "$out" | json "d['pgx']['installed']")" = False ] && pass "A detect: pgx not installed is reported" || fail "A detect: $out"
mkdir -p "$HOME/.claude"      # a machine with Claude Code
out=$(pgx --json agent setup) && pass "A setup on an empty project exits 0" || fail "A setup: $out"
[ "$(printf '%s' "$out" | json "d['runtime']['initialized']")" = True ] && pass "A setup initialised PGX" || fail "A setup did not initialise: $out"
[ "$(printf '%s' "$out" | json "d['source']['name']+':'+str(d['source']['created'])")" = "app:True" ] && pass "A setup created the empty app template" || fail "A source: $out"
[ "$(printf '%s' "$out" | json "[h['host'] for h in d['hooks']]")" = "['claude']" ] && pass "A setup detected Claude Code and installed hooks" || fail "A hooks: $out"
[ -f .pgx.toml ] && grep -q 'source = "app"' .pgx.toml && pass "A .pgx.toml written" || fail "A .pgx.toml"
pgx --json agent doctor >/dev/null && pass "A agent doctor is clean" || fail "A agent doctor: $(pgx agent doctor)"
pgx stop >/dev/null

# ---- B. existing local PostgreSQL ---------------------------------------------
scenario existing
if [ -n "$PG_BINDIR" ] && [ -x "$PG_BINDIR/initdb" ]; then
  PGDATA_SCRATCH="$root/existing/pgdata"; pgport=$(free_port)
  "$PG_BINDIR/initdb" -D "$PGDATA_SCRATCH" -A trust -U postgres >/dev/null 2>&1
  "$PG_BINDIR/pg_ctl" -D "$PGDATA_SCRATCH" -o "-p $pgport -k '' -c listen_addresses=127.0.0.1" -l "$root/existing/pg.log" start -w >/dev/null 2>&1
  psql "postgres://postgres@127.0.0.1:$pgport/postgres" -X -q -c "CREATE DATABASE myapp" >/dev/null
  DEV_URL="postgres://postgres@127.0.0.1:$pgport/myapp"
  psql "$DEV_URL" -X -q -c "CREATE TABLE users(id serial PRIMARY KEY, email text UNIQUE); INSERT INTO users(email) VALUES ('a@x'),('b@x'),('c@x')" >/dev/null
  before=$(psql "$DEV_URL" -X -Atc "SELECT count(*)||':'||md5(string_agg(email,',' ORDER BY id)) FROM users")
  printf 'DATABASE_URL=%s\n' "$DEV_URL" > .env; printf 'source "https://rubygems.org"\ngem "rails"\ngem "pg"\n' > Gemfile; mkdir -p config && printf 'development:\n  adapter: postgresql\n  url: <%%= ENV["DATABASE_URL"] %%>\n' > config/database.yml
  cls=$(bash "$skill/scripts/detect-project.sh" . | json "d['database_config']['classification']+':'+d['framework']")
  [ "$cls" = "local:rails" ] && pass "B detect: local PostgreSQL, Rails" || fail "B detect: $cls"
  pgx run >/dev/null 2>&1
  pgx copy --from "$DEV_URL" --to myapp_template --schema-only >/dev/null 2>&1 && pass "B pgx copy --schema-only" || fail "B pgx copy failed"
  [ "$(pgx --json agent setup --source myapp_template --hosts claude | json "d['source']['connections']")" = 0 ] && pass "B setup on the template" || fail "B setup"
  a=$(pgx --json agent up --id sessA --host claude); b=$(pgx --json agent up --id sessB --host claude)
  ua=$(printf '%s' "$a" | json "d['url']"); ub=$(printf '%s' "$b" | json "d['url']")
  [ "$ua" != "$ub" ] && pass "B two sessions, two branches" || fail "B same branch"
  psql "$ua" -X -q -c "ALTER TABLE users ADD COLUMN last_login_at timestamptz" >/dev/null
  [ "$(psql "$ub" -X -Atc "SELECT count(*) FROM information_schema.columns WHERE table_name='users' AND column_name='last_login_at'")" = 0 ] && pass "B migration in A invisible in B" || fail "B isolation"
  [ "$(psql "$(pgx url myapp_template)" -X -Atc "SELECT count(*) FROM information_schema.columns WHERE column_name='last_login_at'")" = 0 ] && pass "B template unchanged" || fail "B template changed"
  [ "$(psql "$ua" -X -Atc "SELECT count(*) FROM users")" = 0 ] && pass "B schema-only: no rows copied" || fail "B rows were copied"
  after=$(psql "$DEV_URL" -X -Atc "SELECT count(*)||':'||md5(string_agg(email,',' ORDER BY id)) FROM users")
  [ "$before" = "$after" ] && pass "B the developer's PostgreSQL is unchanged" || fail "B PostgreSQL changed"
  "$PG_BINDIR/pg_ctl" -D "$PGDATA_SCRATCH" stop -m fast >/dev/null 2>&1; PGDATA_SCRATCH=""
  pgx stop >/dev/null
else
  echo "skip  B (no PostgreSQL bindir: set PG_BINDIR)"
fi

# ---- C. PGX-only application ---------------------------------------------------
scenario pgxonly
pgx agent setup --no-hooks >/dev/null 2>&1
out=$(pgx agent exec --host psql -- sh -c 'psql "$DATABASE_URL" -X -Atc "select current_database()"' 2>&1)
case "$out" in agent-psql-*) pass "C exec runs psql in its own branch ($out)" ;; *) fail "C exec: $out" ;; esac
out=$(pgx agent exec -- sh -c 'psql "$DATABASE_URL" -X -q -c "create table t(x int)" && psql "$DATABASE_URL" -X -Atc "select count(*) from t"')
[ "$out" = "0" ] && pass "C schema work inside the branch" || fail "C: $out"
[ "$(pgx --json agent ls | json "len(d)")" = 0 ] && pass "C branch dropped after exec" || fail "C branch leaked"
[ "$(psql "$(pgx url app)" -X -Atc "select count(*) from pg_tables where tablename='t'")" = 0 ] && pass "C template untouched" || fail "C template touched"
pgx stop >/dev/null

# ---- D. concurrent agents -----------------------------------------------------
scenario concurrent
pgx agent setup --no-hooks >/dev/null 2>&1
pids=(); for i in 1 2 3 4 5 6; do
  ( pgx agent exec --keep -- sh -c "psql \"\$DATABASE_URL\" -X -q -c 'create table only_$i(x int); insert into only_$i values($i)' && psql \"\$DATABASE_URL\" -X -Atc \"select count(*) from pg_tables where tablename like 'only_%'\"" > "$root/concurrent/out.$i" 2>&1 ) & pids+=($!)
done
okc=0; for p in "${pids[@]}"; do wait "$p" && okc=$((okc+1)); done
[ "$okc" = 6 ] && pass "D six parallel wrappers exit 0" || fail "D only $okc exited 0: $(cat "$root"/concurrent/out.*)"
[ "$(cat "$root"/concurrent/out.* | sort -u | tr '\n' ' ')" = "1 " ] && pass "D each branch sees only its own table" || fail "D counts: $(cat "$root"/concurrent/out.* | tr '\n' ' ')"
[ "$(pgx --json agent ls | json "len({r['name'] for r in d})")" = 6 ] && pass "D six distinct branches" || fail "D branches: $(pgx agent ls)"
[ "$(psql "$(pgx url app)" -X -Atc "select count(*) from pg_tables where tablename like 'only_%'")" = 0 ] && pass "D template unchanged" || fail "D template changed"
pgx stop >/dev/null

# ---- E. lifecycle -------------------------------------------------------------
scenario lifecycle
pgx agent setup --no-hooks >/dev/null 2>&1
b1=$(pgx --json agent up --id life1aaa --host claude | json "d['branch']+':'+str(d['created'])")
b2=$(pgx --json agent up --id life1aaa --host claude | json "d['branch']+':'+str(d['created'])")
[ "$b1" = "agent-claude-life1aaa:True" ] && [ "$b2" = "agent-claude-life1aaa:False" ] && pass "E repeated activation reuses the branch" || fail "E $b1 $b2"
pgx --json agent down --id life1aaa --host claude | json "d['dropped']" | grep -q False && pass "E session end retires (grace)" || fail "E down"
[ "$(pgx --json agent ls | json "d[0]['state']")" = retired ] && pass "E state retired" || fail "E state"
[ "$(pgx --json agent up --id life1aaa --host claude | json "d['created']")" = False ] && pass "E resume within grace revives the same branch" || fail "E revive"
pgx --json agent up --id life2bbb --host claude --ttl 3s >/dev/null
t0=$(date +%s); while pgx --json agent ls | json "'agent-claude-life2bbb' in [r['name'] for r in d]" | grep -q True; do sleep 1; [ $(( $(date +%s) - t0 )) -gt 30 ] && break; done
pgx --json agent ls | json "'agent-claude-life2bbb' in [r['name'] for r in d]" | grep -q False && pass "E expired branch removed by the supervisor" || fail "E expiry"
pgx stop >/dev/null; pgx start >/dev/null 2>&1
[ "$(pgx --json agent up --id life1aaa --host claude | json "d['created']")" = False ] && pass "E branch survives a PGX restart" || fail "E restart"
pgx stop >/dev/null

# ---- F. security --------------------------------------------------------------
scenario security
out=$(bash "$skill/scripts/detect-project.sh" "$repo/skills/tests/pgx/fixtures/sensitive")
case "$out" in *topsecret*|*admin:*) fail "F detect leaked a secret" ;; *) pass "F detect prints no credentials" ;; esac
[ "$(printf '%s' "$out" | json "d['database_config']['classification']")" = sensitive ] && pass "F production-looking host is classified sensitive" || fail "F classification: $out"
pgx agent setup --no-hooks >/dev/null 2>&1
printf '[agent]\nsource = "postgres://u:p@h/db"\n' > .pgx.toml
pgx --json agent up --id sec1 --host claude >/dev/null 2>&1; rc=$?; [ $rc = 2 ] && pass "F .pgx.toml with a URL is rejected (exit 2)" || fail "F toml url rc=$rc"
printf '[agent]\nsource = "app"\n' > .pgx.toml
pgx --json agent up --id sec2 --host claude >/dev/null
pgx create agent-claude-foreign --ttl 1h >/dev/null
pgx --json agent down --id foreign-0000 --host claude --now >/dev/null 2>&1; rc=$?
[ $rc = 5 ] && pgx --json ls | json "'agent-claude-foreign' in [x['name'] for x in d]" | grep -q True && pass "F down never drops a database it does not own" || fail "F foreign drop rc=$rc"
printf '{"hook_event_name":"SessionStart","session_id":"../../etc/passwd","cwd":"%s"}' "$PROJ" | CLAUDE_ENV_FILE="$root/security/env.sh" pgx agent hook >/dev/null 2>&1; rc=$?
[ $rc != 2 ] && [ ! -s "$root/security/env.sh" ] && pass "F hostile session id rejected without side effects" || fail "F hostile id rc=$rc"
printf '[agent]\nsource = "nope"\n' > .pgx.toml
printf '{"hook_event_name":"SessionStart","session_id":"abcd1234","cwd":"%s"}' "$PROJ" | CLAUDE_ENV_FILE="$root/security/env2.sh" pgx agent hook >/dev/null 2>&1
grep -q "PGX_ISOLATION='failed'" "$root/security/env2.sh" && grep -q "127.0.0.1:1/" "$root/security/env2.sh" && pass "F missing source fails closed (sentinel DATABASE_URL)" || fail "F fail-closed: $(cat "$root/security/env2.sh" 2>/dev/null)"
pgx stop >/dev/null

# ---- G. installation UX --------------------------------------------------------
scenario installux
mkdir -p .claude; printf '{"env":{"KEEP":"1"},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"orca-hook"}]}]}}' > .claude/settings.json
pgx agent setup --hosts claude,cursor,codex --no-hooks >/dev/null 2>&1
pgx --json agent install claude cursor codex >/dev/null && pgx --json agent install claude cursor codex > "$root/installux/second.json"
[ "$(json "sum(h['added'] for h in d['hosts'])" < "$root/installux/second.json")" = 0 ] && pass "G repeated install adds nothing" || fail "G second install"
python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["env"]["KEEP"]=="1"; assert d["hooks"]["SessionStart"][0]["hooks"][0]["command"]=="orca-hook"' .claude/settings.json && pass "G existing hooks and settings intact after install" || fail "G foreign hooks damaged"
pgx --json agent uninstall claude cursor codex >/dev/null
python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["hooks"]["SessionStart"]==[{"hooks":[{"type":"command","command":"orca-hook"}]}]; assert "SessionEnd" not in d["hooks"]' .claude/settings.json && pass "G uninstall removes only PGX entries" || fail "G uninstall"
[ -f .pgx.toml ] && pass "G uninstall keeps .pgx.toml" || fail "G toml removed"
if [ "${SKIP_NPX:-0}" != 1 ] && command -v npx >/dev/null 2>&1; then
  sp="$root/installux/skillproj"; mkdir -p "$sp/.git"
  ( cd "$sp" && npx -y skills@latest add "$skill" -a claude-code cursor codex -y >"$root/installux/npx-install1.log" 2>&1 && npx -y skills@latest add "$skill" -a claude-code cursor codex -y >"$root/installux/npx-install2.log" 2>&1 )
  [ -f "$sp/.claude/skills/pgx/SKILL.md" ] && [ -f "$sp/.agents/skills/pgx/SKILL.md" ] && pass "G skills CLI installs for claude-code, cursor and codex (repeatable)" || fail "G skills install: $(tail -5 "$root/installux/npx-install1.log")"
  [ ! -e "$sp/.claude/skills/pgx/tests" ] && pass "G installed skill carries no test fixtures" || fail "G fixtures installed"
  ( cd "$sp" && npx -y skills@latest add "$skill" --list >"$root/installux/npx-list.log" 2>&1 ); grep -q 'pgx' "$root/installux/npx-list.log" && pass "G skills CLI lists the pgx skill" || fail "G skills --list: $(tail -5 "$root/installux/npx-list.log")"
  ( cd "$sp" && npx -y skills@latest remove pgx -y >"$root/installux/npx-remove.log" 2>&1 ); [ ! -e "$sp/.claude/skills/pgx" ] && pass "G skills CLI removes the skill cleanly" || fail "G skills remove: $(tail -5 "$root/installux/npx-remove.log")"
else
  echo "skip  G skills-CLI part (SKIP_NPX)"
fi
pgx stop >/dev/null

echo; echo "e2e: $passes passed, $fails failed (scratch kept at $root)"
[ "$fails" = 0 ]
