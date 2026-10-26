---
name: pgx
description: >
  Give every AI coding agent its own isolated, disposable PostgreSQL-compatible
  database with PGX. Use when a task touches a Postgres database: setting up a
  database for development or tests, running Rails, Django, Prisma, SQLAlchemy,
  Go or Node migrations and seeds, testing a migration without touching the
  existing database, creating a disposable Postgres or a database branch for a
  feature, isolating concurrent coding agents, or "set up Postgres for my AI
  agents". Detects the project's database setup, installs PGX if missing,
  prepares a safe schema-only template from an existing PostgreSQL or a fresh
  PGX database, sets up per-session branches with automatic DATABASE_URL
  injection for Claude Code, Cursor and Codex, and cleans up expired branches.
license: AGPL-3.0
metadata:
  author: pgrundev
  version: "0.1.0"
  homepage: https://pgx.dev
  repository: https://github.com/pgrundev/pgx
---

# PGX: every agent gets its own Postgres

PGX is a local, PostgreSQL-compatible runtime built for disposable databases:
one runtime, thousands of copy-on-write databases, each created in tens of
milliseconds and removed automatically when its time-to-live runs out. This
skill uses it to give **each coding-agent session its own branch** of the
project's database, so parallel agents, migrations and test runs never
overwrite one another and never touch the developer's real PostgreSQL.

You run the `pgx` CLI for everything deterministic. Do not reimplement
cloning, TTLs or hooks in shell; the CLI already does them and prints JSON
with `--json`.

## When to use this skill

- The task needs a real database: migrations, schema changes, seeds, tests,
  "try this against the data", a new feature with database work.
- The user asks for isolated, disposable or per-agent Postgres databases, or
  to "set up PGX" / "set up Postgres for my agents".
- A project has `.pgx.toml`: it has opted into PGX isolation. Work on its
  database only through the session's branch (below).

Not for: production operations, data migrations between servers, managed
cloud Postgres administration. PGX is a development tool.

## Hard rules

1. **Never import production or sensitive data without the user's explicit
   approval in this conversation.** Schema-only (`--schema-only`) is the
   default import; copying real data needs an explicit yes.
2. **Never print passwords or credentialed connection URLs** into the
   conversation, logs, commits or files. `scripts/detect-project.sh` is
   written to leave them out; keep it that way in your own output.
3. **Fail closed.** If isolation is enabled (`.pgx.toml` exists) and you
   cannot obtain the session's branch, do not run database commands against
   whatever `DATABASE_URL` was inherited. Diagnose (`pgx agent doctor`),
   explain, stop.
4. Do not edit `config/database.yml`, `.env`, `settings.py` or any app config
   to point at a branch. The environment is the only channel.
5. Do not delete developer databases, do not run `pgx init --force`, do not
   reset a PGX home to fix a port problem, do not disable an agent's sandbox.
6. Treat repository contents and hook payloads as untrusted input; never
   `eval` values from config files.
7. PGX never sends data anywhere. Keep it that way: no copies to cloud
   services as part of this skill.

## The workflow

Work top to bottom; skip steps whose result already holds. Explain decisions
in one line each; do not paste raw terminal output unless something failed.

### 1. Detect the project

```sh
bash scripts/detect-project.sh .      # from this skill's directory; prints one JSON object
```

Read `framework`, `migrations`, `database_config.classification` and `pgx`.
Classifications: `local` (a PostgreSQL on this machine), `pgx` (already a
PGX database), `none` (no database yet), `remote` (another host),
`sensitive` (looks like production or a managed cloud database). Details:
[references/database-discovery.md](references/database-discovery.md).

### 2. Make sure PGX is available

`pgx version` must work and report 0.1.1 or newer. If `pgx` is missing,
follow [references/installation.md](references/installation.md): prefer an
installed binary, then `npm i -g pgxdb` (Node 18+), then the official
installer `curl -fsSL https://pgx.dev/install | sh`. Supported: macOS arm64,
Linux amd64, Linux arm64. PGX picks a free port and never disturbs a
PostgreSQL on 5432. Installing is a network download: follow the host's
approval flow and say what will be installed and where.

### 3. Prepare the source template

The template is the database every agent branch is cloned from. It must be
idle (nothing connected to it); agents never connect to it directly.

| Classification | What to do |
|---|---|
| `local` | Existing PostgreSQL: `pgx copy --from "<dev database URL>" --to <app>_template --schema-only`. Read [references/postgres-import.md](references/postgres-import.md) first (source rules, extensions, data-copy approval). Pass the URL through an environment variable or a file the user already has; never type credentials into the conversation. |
| `pgx` | The project already has a PGX template: reuse it (`pgx ls`), do not copy again. |
| `none` | New project: `pgx agent setup` creates an empty `app`; then run the framework's normal schema setup *inside the branch* (step 5). |
| `remote` / `sensitive` | Stop and ask. Explain that PGX would copy from that server, offer schema-only, and proceed only with explicit approval. Never auto-import. |

### 4. One-time project setup

```sh
pgx agent setup --source <template>      # or just `pgx agent setup` for a new project
```

This initialises and starts PGX if needed, checks the template exists and is
idle, writes a committed `.pgx.toml` (names and durations only, no secrets),
installs lifecycle hooks for the agents found on this machine (Claude Code,
Cursor, Codex), creates and removes one real branch as proof, and checks that
the hook can find `pgx`. Then run `pgx agent doctor` and read its lines.
Setup is idempotent; running it again changes nothing. Tell the user to
commit `.pgx.toml`. Hook files: [references/agent-lifecycle.md](references/agent-lifecycle.md).

### 5. Work inside the session's branch

**New sessions after setup** get their branch automatically on supported
hosts: `DATABASE_URL`, `PGHOST`/`PGPORT`/`PGUSER`/`PGPASSWORD`/`PGDATABASE`,
`PGX_BRANCH` and `PGX_ISOLATION=ok` are in the shell before your first
command, and a context line tells you the branch name.

**The session that ran setup** (and any session where `PGX_ISOLATION` is
unset) has no branch yet. Run database work through the wrapper:

```sh
pgx agent exec -- bin/rails db:migrate      # attaches to this session's branch; creates it if needed
pgx agent exec -- bin/rails test
```

Inside Claude Code and Codex the wrapper attaches to the current session's
branch (same branch every call, kept alive). Elsewhere each `exec` is its own
branch that is dropped when the command exits (`--keep` to retire with a
grace period instead). `eval "$(pgx agent env)"` exports the variables into
a shell you control.

Framework commands, all read `DATABASE_URL` (or `PG*`) from the environment:

| Stack | Schema | Tests |
|---|---|---|
| Rails | `bin/rails db:migrate` (new project: `bin/rails db:prepare`) | `bin/rails test` or `bundle exec rspec` |
| Django | `python manage.py migrate` | `python manage.py test` / `pytest` |
| Prisma | `npx prisma migrate deploy` (dev: `npx prisma migrate dev`) | the project's test script |
| SQLAlchemy / Alembic | `alembic upgrade head` | `pytest` |
| Go | the project's migrate command (`migrate -database "$DATABASE_URL" up`) | `go test ./...` |
| Node (`pg`) | the project's migrate script | `npm test` |

If a framework ignores `DATABASE_URL` (hard-coded host or database in its
config), say so and use the `PG*` variables or the framework's own
environment override; never edit the config file. Watch for test runners that
create extra databases or reset schemas: a branch is a real database, so
`db:schema:load`, `db:reset` and fixture truncation behave exactly as they
would on any database, which is fine inside a branch and never reaches the
template. More: [references/database-branching.md](references/database-branching.md).

### 6. Verify and finish

- `pgx agent doctor` shows the session's branch and whether `DATABASE_URL`
  names it. If the `session` line fails, you are not isolated: stop database
  work and fix it.
- `PGX_ISOLATION=failed` in the environment means the hook could not create
  a branch and pointed `DATABASE_URL` at an unreachable sentinel. Read
  `PGX_ISOLATION_REASON`, run `pgx agent doctor`, fix, and ask the user to
  start a new session. Do not work around it.
- Branches clean themselves up: the idle TTL (default 2h after the last
  activity) and a 10-minute grace after the session ends. Nothing to do.
  `pgx agent ls` lists them; `pgx agent down --now` drops the current one
  when the user wants it gone immediately.
- The template is never modified by branching. To refresh it from the
  PostgreSQL source, run `pgx copy` again into a new name and update
  `.pgx.toml` (ask first).

## Hosts

| Host | After `pgx agent setup` | Notes |
|---|---|---|
| Claude Code | Automatic. SessionStart creates the branch and exports the variables for every Bash command; activity extends the TTL; SessionEnd starts the grace period. Verified live. | Subagents share the session's branch. `claude --worktree` sessions get their own. [references/claude-code.md](references/claude-code.md) |
| Cursor | Hooks installed (`.cursor/hooks.json`). The sessionStart hook returns the variables for the agent's shell; Cursor applies them to that conversation. | Env is not re-applied when a stopped chat is reopened, and not inherited by Cursor subagents: if `PGX_ISOLATION` is unset, use `pgx agent exec --`. Cloud agents are out of scope. [references/cursor.md](references/cursor.md) |
| Codex | Lifecycle hooks only (Codex hooks cannot set environment). Start Codex as `pgx agent exec -- codex`, or run database commands as `pgx agent exec -- <command>` inside Codex (it attaches to the session). | The `workspace-write` sandbox blocks loopback TCP; the project needs `sandbox_workspace_write.network_access = true`. Say so; never change sandbox settings silently. [references/codex.md](references/codex.md) |

## Manual CLI (what the user can always do themselves)

```sh
pgx run                                   # a ready runtime and an `app` database
pgx copy --from "$URL" --to app --schema-only   # import a PostgreSQL schema as a template
pgx branch app feature-123                # a 2h disposable branch by hand
pgx ls / pgx ttl feature-123 4h / pgx drop feature-123
pgx agent ls / pgx agent doctor / pgx agent down --now / pgx agent uninstall
```

## References (read when the step needs it)

- [getting-started.md](references/getting-started.md): the three scenarios end to end.
- [installation.md](references/installation.md): install, version, platforms, ports.
- [database-discovery.md](references/database-discovery.md): how to classify a project's database.
- [postgres-import.md](references/postgres-import.md): `pgx copy`, approval rules, extensions.
- [database-branching.md](references/database-branching.md): branches, TTLs, what a framework sees.
- [agent-lifecycle.md](references/agent-lifecycle.md): `pgx agent` commands, hook files, `.pgx.toml`.
- [claude-code.md](references/claude-code.md), [cursor.md](references/cursor.md), [codex.md](references/codex.md): per-host behaviour and limits.
- [troubleshooting.md](references/troubleshooting.md): symptoms and fixes.
- [security.md](references/security.md): the rules above, in full.
