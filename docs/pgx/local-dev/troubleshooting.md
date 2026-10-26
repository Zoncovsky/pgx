# Troubleshooting

Run `pgx doctor` first; it names most problems and the fix.

| Symptom | Cause / fix |
|---|---|
| `pgx init` → exit 2, "socket path would be … bytes" | `$PGX_HOME` is too deep for a Unix socket (limit 103 bytes). Set `PGX_RUN_DIR=/tmp/pgx` and re-run, or use a shorter `$PGX_HOME`. |
| `pgx start` → exit 4, "port … is in use" | Another process holds the port. Stop it, or `pgx init --force --port N`. |
| `pgx start` → exit 3, "not initialised" | Run `pgx init`. |
| exit 7, "incomplete PGX installation" | A file is missing from the distribution; reinstall. `pgx doctor` lists which. |
| `pgx drop` → exit 6, "in use" | Clients are connected. Disconnect them, or `pgx drop <db> --force`. |
| XML functions raise `0A000 unsupported XML feature` | The host has no libxml2. Install it (`libxml2` on Debian/Ubuntu); macOS and the Docker image include it. `pgx doctor` shows library status. |
| `pgx ls` shows a database as `invalid (interrupted drop)` | A `DROP DATABASE` was interrupted. Run `pgx drop <db> --force` again. |
| `pgx doctor` warns about orphan `base/<oid>` directories | A CREATE DATABASE or clone was interrupted by a crash. PGX never deletes them automatically; after confirming nothing is creating a database, remove them by hand from `$PGX_HOME/data/base`. |
| In a container, `pgx stop` seems slow or restart fails after a kill | The container has no init to reap processes. Run it with `docker run --init`, or use `pgx serve` (the engine is PID 1). |
| disk full → `53100` then the runtime stops | Expected: PGX is one crash domain. Free space and `pgx start`; committed data is recovered. |
| `pgx copy` → exit 1, "pg_dump and pg_restore were not found" | `pgx copy` runs the PostgreSQL client tools, which PGX does not bundle. Install them (`brew install libpq`, `apt install postgresql-client-18`, …) or set `PGX_PG_BINDIR`. `pgx doctor` shows what it finds. See [copy.md](copy.md). |
| `pgx copy` → exit 1, "unsupported extension: …" | The source uses an extension PGX does not have (PostGIS, TimescaleDB, …). PGX cannot load C extensions; keep that database on PostgreSQL or drop the extension in a copy of the source. |
| `pgx copy` → exit 1, "restore into PGX failed … role … does not exist" | A policy or similar object names a role; roles are not part of a database dump. `psql "$(pgx url postgres)" -c 'CREATE ROLE <name>'` and retry. |
| `pgx copy` → exit 4, "database … already exists" | `pgx copy` never overwrites. Use another `--to` name or `pgx drop` the old one first. |
| `pgx ls` shows `_pgxcopy_… abandoned` | A `pgx copy` process was killed. The next `pgx copy` removes it, or `pgx drop _pgxcopy_… --force`. |

Logs: `$PGX_HOME/log/pgx.log`. Status as JSON: `pgx --json status`.

## Coding agents

`pgx agent doctor` checks the project (`.pgx.toml`, hooks per host, the
source template) and, inside a session, whether `DATABASE_URL` names the
session's branch. The Skill's own list is in
[skills/pgx/references/troubleshooting.md](../../../skills/pgx/references/troubleshooting.md).

| Symptom | Cause / fix |
|---|---|
| `PGX_ISOLATION=failed`, `DATABASE_URL` points at port 1 | The SessionStart hook could not create a branch and failed closed; `PGX_ISOLATION_REASON` says why (missing source, busy source, PGX not initialised). Fix it, start a new session. |
| No "PGX: this session's database is …" line at session start | Hooks not installed, or the hook's shell cannot find `pgx`. `pgx agent doctor` → `hooks:<host>`; `pgx agent install <host>` reports the self-test and the `pgx` it resolved; link the binary into `~/.local/bin` for Dock-launched editors. |
| `pgx agent up` → exit 6, "a branch source must be idle" | Something is connected to the template. Keep `<app>_template` for branching only. |
| Rails prints `ActiveRecord::EnvironmentMismatchError` after a green test run | The branch's `ar_internal_metadata` says `development`; the spawned `db:test:prepare` refuses to purge. Harmless. Never run `db:environment:set` in a branch. |
| Rails re-ran every migration on a fresh branch | The template was imported schema-only by a `pgx copy` older than 0.1.1. Re-import (bookkeeping rows are now kept). |
| Codex commands cannot connect; `CODEX_SANDBOX_NETWORK_DISABLED=1` | The `workspace-write` sandbox blocks loopback TCP. `sandbox_workspace_write.network_access = true` in `.codex/config.toml` (the user's decision). |
| Cursor: variables missing after reopening a stopped chat | Cursor does not re-run `sessionStart` on reopen. Start a new chat, or `pgx agent exec -- <command>`. |
| A branch outlived its session | Grace period (10 min) or idle TTL (2 h) not yet reached; the supervisor removes it. `pgx agent down --now` drops it at once. |
