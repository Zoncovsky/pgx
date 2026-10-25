# `pgx` CLI reference

`pgx [--json] <command> [arguments]`. `--json` makes a command print one JSON
object (or array) on stdout; errors print `{"error":{...}}`. Data lives under
`$PGX_HOME` (default `~/.pgx`).

## Quick start

| Command | What it does |
|---|---|
| `pgx run` | One command to a ready database. Initialise `$PGX_HOME` if needed, start the runtime if it is not already up, ensure a default `app` database, and print its connection URL. Idempotent and safe to repeat; it never resets or overwrites an existing database. `--json` prints one object (for agents). The first run reads `PGX_PORT` / `PGX_DURABILITY` / `PGX_PASSWORD`; afterwards they come from the saved config. |

```console
$ pgx run
PGX is ready.

  postgres://postgres:…@127.0.0.1:5432/app

  database   app (created)
  runtime    started (pid 23469, port 5432)
  home       /home/you/.pgx

Connect:
  psql "postgres://postgres:…@127.0.0.1:5432/app"
```

## Runtime

| Command | What it does |
|---|---|
| `pgx init [--port N] [--durability ephemeral\|full] [--force]` | Create the cluster under `$PGX_HOME` from the bundled base cluster, pick a free port, set a random superuser password, write the config. `--force` recreates (deletes every database). |
| `pgx start` | Start the runtime in the background (idempotent: a second start reports "already running"). |
| `pgx serve` | Run the engine in the **foreground** (init first if needed); the Docker/systemd entrypoint. |
| `pgx stop [--force]` | Fast shutdown (SIGINT); `--force` escalates to SIGKILL after the timeout. |
| `pgx restart` | Stop then start. |
| `pgx status` | Running?, pid, port, databases, connections, uptime, memory, libraries. Exit 3 when stopped. |

## Databases

| Command | What it does |
|---|---|
| `pgx create <db> [--ttl <dur>]` | Create an empty database. `--ttl` makes it temporary (auto-expires); default is permanent. |
| `pgx drop <db> [--force]` | Drop it; without `--force`, exit 6 if sessions are connected (data untouched); `--force` terminates them first. |
| `pgx clone <src> <dst> [--ttl <dur>]` | Copy-on-write clone (`CREATE DATABASE … STRATEGY = FILE_COPY`); shares blocks on APFS/ZFS/reflink filesystems. TTL is never inherited from the source. |
| `pgx branch <source> <name> [--ttl <dur>]` | A disposable database for one agent: the same clone, with a **2h default TTL** (`--ttl 30m`, or `--ttl unlimited` for a permanent branch) and the connection URL printed as the last line, so `DATABASE_URL="$(pgx branch myapp agent-1 \| tail -1)"` works. Refuses an existing name (exit 4) and a missing source (exit 5) without changing anything. `--json` returns `branch`, `source`, `url`, `socket_url`, `ttl`, `ttl_seconds`, `expires_at`, `expires_in_seconds`, `method`, `elapsed_ms`. |
| `pgx ls` | List databases with status, TTL, remaining time and size. |
| `pgx ttl <db> [<dur>\|unlimited]` | Show a database's TTL, or set/extend/disable it. See [ttl.md](ttl.md). |
| `pgx url <db> [--socket]` | Print the connection URL (TCP with password, or Unix socket). |

```console
$ pgx branch myapp agent-1
✓ Branch created: agent-1
✓ Source: myapp
✓ Expires in: 2 hours

DATABASE_URL:
postgres://postgres:…@127.0.0.1:5433/agent-1
```

## Import from PostgreSQL

| Command | What it does |
|---|---|
| `pgx copy --from <postgres-url> --to <db> [--schema-only] [--jobs N] [--verbose] [--no-optimize]` | Copy an existing PostgreSQL database into PGX as a template to clone from: `pg_dump` (read-only on the source) → compatibility gate → `pg_restore --exit-on-error` into a staging database → validate → `VACUUM (FREEZE, ANALYZE)` → atomic rename to `<db>`. Refuses an existing destination (exit 4) and an incompatible source (exit 1) without creating anything. Needs `pg_dump`/`pg_restore` (`PGX_PG_BINDIR`, `PATH`, or the usual install dirs). See [copy.md](copy.md). |

Temporary databases (`--ttl`) are deleted automatically by a background worker;
permanent databases are never auto-deleted. Full semantics: [ttl.md](ttl.md).

## Tools

`pgx doctor` checks the install, data directory, config mode, socket-path
length, port, runtime, disk, stack limit, filesystem/block-cloning, libxml2/ICU,
the PostgreSQL client tools `pgx copy` would use, and orphan `base/<oid>`
directories (read-only; never deletes). `pgx version`
reports the engine commit and PostgreSQL compatibility. `pgx help`.

## Database names

1–63 bytes, `^[A-Za-z_][A-Za-z0-9_-]*$`, always sent quoted; `postgres`,
`template0`, `template1` cannot be created, dropped or cloned onto.

## Exit codes

`0` ok · `1` engine/runtime error · `2` usage · `3` not initialised / not
running · `4` conflict (exists, running, port in use) · `5` not found · `6`
database in use · `7` corrupted or incomplete installation.

## Environment

`PGX_HOME` (data home), `PGX_RUN_DIR` (socket directory, for a long `$PGX_HOME`),
`PGX_PORT` / `PGX_DURABILITY` / `PGX_LISTEN` / `PGX_PASSWORD` (read by `init` and
`serve`, used by the Docker entrypoint), `PGX_PG_BINDIR` (where `pgx copy` finds
`pg_dump` and `pg_restore`).
