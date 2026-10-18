# `pgx` CLI reference

`pgx [--json] <command> [arguments]`. `--json` makes a command print one JSON
object (or array) on stdout; errors print `{"error":{...}}`. Data lives under
`$PGX_HOME` (default `~/.pgx`).

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
| `pgx create <db>` | Create an empty database. |
| `pgx drop <db> [--force]` | Drop it; without `--force`, exit 6 if sessions are connected (data untouched); `--force` terminates them first. |
| `pgx clone <src> <dst>` | Copy-on-write clone (`CREATE DATABASE … STRATEGY = FILE_COPY`); shares blocks on APFS/ZFS/reflink filesystems. |
| `pgx ls` | List databases with size, connections and state (invalid databases from an interrupted drop are flagged). |
| `pgx url <db> [--socket]` | Print the connection URL (TCP with password, or Unix socket). |

## Tools

`pgx doctor` checks the install, data directory, config mode, socket-path
length, port, runtime, disk, stack limit, filesystem/block-cloning, libxml2/ICU,
and orphan `base/<oid>` directories (read-only; never deletes). `pgx version`
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
`serve`, used by the Docker entrypoint).
