# PGX release

A self-contained, relocatable, PostgreSQL 18 compatible runtime for local
development, CI and disposable databases. No system PostgreSQL required.

## What is new in 0.1.1: one Skill, every agent gets its own Postgres

```sh
npx skills add pgrundev/pgx --skill pgx     # the PGX Agent Skill for Claude Code, Cursor and Codex
```

Then ask your agent to set up isolated databases for coding agents. It
detects the project, installs PGX if needed, imports your development
database schema-only into a template, runs `pgx agent setup`, and from then
on every Claude Code session works in its own branch with `DATABASE_URL`
already set (Cursor: hooks installed, env propagation pending live
verification; Codex: `pgx agent exec -- codex`). Your PostgreSQL is read once
and never written.

- `pgx agent setup | doctor | exec -- <command> | ls` and the primitives
  `up | touch | down | env | hook | install | uninstall`; idle TTL 2h, 10-minute
  grace after a session, fail-closed sentinel when a branch cannot be made.
- `.pgx.toml`: the committed per-project source template and TTLs.
- `pgx copy --schema-only` keeps migration-bookkeeping rows
  (`schema_migrations`, `django_migrations`, `_prisma_migrations`, …).
- `pgx drop` removes the TTL record; `--json` keeps key order.
- Guide and compatibility matrix: `docs/pgx/local-dev/agents.md`.

## Install

```sh
curl -fsSL https://pgx.dev/install | sh
pgx run                             # a ready `app` database in one step
pgx branch app agent-001            # a disposable 2-hour branch
pgx agent exec -- bin/rails test    # any command in a branch of its own
```

Or download the tarball for your platform below, verify its SHA-256 against
`checksums.txt`, and extract it; `bin/pgx` is relocatable.

## Platforms

| Platform | Artifact |
|---|---|
| Linux x86-64 | `pgx-<version>-linux-amd64.tar.gz` |
| Linux arm64  | `pgx-<version>-linux-arm64.tar.gz` |
| macOS arm64  | `pgx-<version>-darwin-arm64.tar.gz` |

XML functions and ICU collations use the host's libxml2 and ICU when present.

## Verified in this release

See `docs/pgx/local-dev/validation-report.md` for the full matrix (unit,
lifecycle, SQL, compatibility vs PostgreSQL 18.6, crash recovery, isolation,
storage, concurrency, stress, and the application fixtures) and the
GO / NO-GO / BLOCKED decision.
