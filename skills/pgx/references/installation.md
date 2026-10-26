# Installing and checking PGX

## Is it there?

```sh
command -v pgx && pgx --json version      # {"pgx_version":"0.1.x", …}
pgx --json status                         # exit 3 when initialised but stopped; hooks start it on demand
```

The skill needs 0.1.1 or newer (`pgx agent` exists). An older binary:
upgrade with the same method it was installed with (`npm i -g pgxdb@latest`,
or the installer again).

## Install, in order of preference

1. **A binary that is already on the machine** (a previous install under
   `~/.local/bin/pgx`, Homebrew, or `~/.pgx/dist/<version>/bin/pgx`). Prefer
   it; do not download twice.
2. **npm**, when Node.js 18+ is present: `npm i -g pgxdb` installs the
   `pgxdb` and `pgx` commands; the first run downloads the native release
   tarball into `~/.pgx/dist/<version>`. `npx pgxdb@latest` works without a
   global install but gives you `npx pgxdb …` instead of `pgx …`.
3. **The official installer**: `curl -fsSL https://pgx.dev/install | sh`.
   Verified release assets from github.com/pgrundev/pgx; installs under
   `~/.pgx/dist/<version>` and links `pgx` into `~/.local/bin` (override with
   `PGX_BIN_DIR`). No root.

Nothing else. No third-party scripts, no building from source for a user who
just wants a database (that is for contributors).

Installing downloads from the network and writes to the user's home. Follow
the host's approval flow: say which command you will run, what it installs,
and where. If approval is denied or no method is possible, print the command
for the user to run themselves and stop.

## Platforms

macOS arm64 (Apple silicon), Linux amd64, Linux arm64. Not supported: Intel
macOS, Windows (WSL2 counts as Linux). Do not claim otherwise; on an
unsupported platform explain and stop.

Docker is not needed. PGX is a single runtime process started by `pgx start`
(or by the hooks on demand).

## Ports and an existing PostgreSQL

An existing PostgreSQL on 5432 is never touched. PGX picks a free port on
its first `pgx run`/`pgx agent setup` (5432-5500, or `PGX_PORT`) and saves it
in its config; afterwards every URL carries that port. Do not assume 5433.
Never re-initialise (`pgx init --force`) to change a port: that deletes every
PGX database. If a port conflict appears, `pgx doctor` explains it.

## Prerequisites for importing a PostgreSQL database

`pgx copy` needs `pg_dump` and `pg_restore` (PostgreSQL 16+ client tools) on
PATH or in `PGX_PG_BINDIR`. macOS: `brew install libpq` then
`PGX_PG_BINDIR=$(brew --prefix libpq)/bin`; Debian/Ubuntu:
`postgresql-client`. The tools must be at least as new as the source server.
