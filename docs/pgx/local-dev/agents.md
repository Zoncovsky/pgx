# PGX for AI coding agents

**Every AI agent gets its own Postgres.** Install one Skill, and each Claude
Code, Cursor or Codex session that touches your database works in its own
disposable branch of a template, created in milliseconds, never shared, and
cleaned up on its own. Your real PostgreSQL is read once and never written.

```sh
npx skills add pgrundev/pgx --skill pgx     # the Skill (recommended)
npx pgxdb@latest                            # PGX itself, if you prefer to install it by hand
```

Then open your agent in the project and ask it to set PGX up:

> Use PGX to give this project isolated databases for coding agents. Set everything up.

The agent (through the Skill) detects the project's database setup, installs
PGX if it is missing, imports your development database **schema-only** into
a template, runs `pgx agent setup`, and verifies the result. From the next
session on, isolation is automatic on the hosts marked verified below. A
Skill installation on its own executes nothing; the agent performs setup when
the Skill is activated and the host's permissions allow it.

## Versions

`pgx agent` and the Skill need **PGX 0.1.1 or newer**
([release](https://github.com/pgrundev/pgx/releases/tag/v0.1.1): linux-amd64,
linux-arm64, darwin-arm64 with SHA-256 checksums). The Skill's
`scripts/check-pgx.sh` verifies the installed binary from `pgx --json version`
and refuses older ones (the published 0.1.0 has no `pgx agent`). npm
`pgxdb@0.1.1` follows the GitHub release through the repository's staged
publishing process.

## What you get

| | |
|---|---|
| Template | `<app>_template`, a schema-only (or, with your approval, full) copy of your PostgreSQL database, or an empty `app` for a new project. Nothing connects to it; it only gets branched. |
| Branch per session | `agent-<host>-<8 chars of the session id>`, a copy-on-write clone. Two sessions, two branches; a `claude --worktree` session gets its own. |
| Environment | `DATABASE_URL`, `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `PGX_BRANCH`, `PGX_SOURCE`, `PGX_ISOLATION=ok`, present before the agent's first command. |
| Lifetime | Idle TTL of 2 hours, extended by every prompt and shell command; a 10-minute grace period after the session ends, so a resume finds its data. |
| Cleanup | The PGX supervisor removes expired branches. `pgx agent ls` shows them; `pgx agent down --now` drops the current one. |
| Fail closed | If a project has `.pgx.toml` and a branch cannot be made, the session's `DATABASE_URL` is pointed at an unreachable sentinel (`PGX_ISOLATION=failed`) instead of silently inheriting another database. |

## Host compatibility

| Host | Automatic isolation | How | Verified |
|---|---|---|---|
| **Claude Code** | Yes | `SessionStart` hook writes the exports to `CLAUDE_ENV_FILE`; `UserPromptSubmit`/`PreToolUse` extend the TTL; `SessionEnd` retires the branch. | **Live, 2026-10-25**: setup from an existing PostgreSQL, feature work in a fresh session, resume within grace, two concurrent sessions. [validation note](validation/2026-10-25-agent-live-claude.md) |
| **Cursor** (IDE agent, `agent` CLI) | Hooks installed; env injection pending live verification | `sessionStart` hook answers `{"env": …}`, which Cursor applies to that conversation's shell per its docs. Env is not re-applied to a reopened chat and not inherited by Cursor subagents; cloud agents are skipped. | Payload-level tests only; live run needs `agent login`. [note](validation/2026-10-25-agent-live-cursor.md). Use `pgx agent exec -- <command>` whenever `PGX_ISOLATION` is unset. |
| **Codex** | With the wrapper | Codex hooks cannot set environment. `pgx agent exec -- codex` puts the branch in Codex's environment; inside any Codex session `pgx agent exec -- <command>` attaches to the session's branch. Lifecycle hooks still create the branch and tell the model. The `workspace-write` sandbox blocks loopback TCP until `sandbox_workspace_write.network_access = true`. | Wrapper verified by tests; native hooks pending a logged-in Codex. [note](validation/2026-10-25-agent-live-codex.md) |
| Anything else | With the wrapper | `pgx agent exec -- <command>` | `tests/agent_exec.rs`, e2e scenarios C and D |

## The pieces

- **The Skill** (`skills/pgx/`, [SKILL.md](../../../skills/pgx/SKILL.md)):
  what an agent reads. It decides when a task needs a database, classifies
  the project's existing setup (`local`, `pgx`, `none`, `remote`,
  `sensitive`), chooses schema-only import by default, never imports
  production data without an explicit yes, runs the commands below, and
  explains failures. Installed with the `skills` CLI into `.claude/skills/pgx`
  (Claude Code) and `.agents/skills/pgx` (Codex, Cursor and others).
- **`pgx agent`** (the CLI, [cli.md](cli.md#agents-one-isolated-database-per-coding-agent-session)):
  the deterministic part. `setup`, `doctor`, `exec`, `ls`, and the
  primitives `up`, `touch`, `down`, `env`, `hook`, `install`, `uninstall`.
  Everything takes `--json`.
- **`.pgx.toml`** in the project root, written by `setup`, safe to commit:
  `source`, `idle_ttl`, `grace_period`. Names and durations only; URLs are
  rejected. PGX is one runtime per user, so the project is part of a
  session's identity and two projects can both call their template `app`.
- **Hook files**, merged into `.claude/settings.json`, `.cursor/hooks.json`,
  `.codex/hooks.json` (or the `--scope local`/`user` equivalents). The
  command is guarded (`command -v pgx … || exit 0`), so a committed hook is a
  silent no-op for a teammate without PGX. Existing entries are never
  touched; `pgx agent uninstall` removes only ours.

## Manual use

```sh
pgx copy --from "$DATABASE_URL" --to myapp_template --schema-only   # import once
pgx agent setup --source myapp_template                            # config + hooks + proof
pgx agent doctor                                                   # is this project ready? is this session isolated?
pgx agent exec -- bin/rails test                                   # any command in its own branch
pgx agent ls                                                       # who has a branch, until when
pgx agent down --now                                               # drop this session's branch
pgx agent uninstall claude cursor codex                            # remove the hooks (keeps .pgx.toml)
pgx branch myapp_template feature-123                              # a hand-made 2h branch, as before
```

## Semantics worth knowing

- **Identity.** A session is `(project, host, session id)`; the branch name
  is derived from it. The same session id in another project is another
  branch. Hand-picked ids (`pgx agent exec --id`) shorter than 8 characters
  or with other characters are hashed into the name.
- **The source must be idle.** `CREATE DATABASE … TEMPLATE` refuses a busy
  template; `up` fails with exit 6 and says so. Keep `<app>_template` for
  branching only.
- **Schema-only keeps migration bookkeeping.** `pgx copy --schema-only`
  carries the rows of `schema_migrations`, `ar_internal_metadata`,
  `django_migrations`, `_prisma_migrations`, `alembic_version` and the other
  common bookkeeping tables, so a framework sees its migrations as applied.
- **Framework purges are survived.** If a session drops and recreates its
  branch under the same name (Rails `db:reset`), `pgx agent` adopts the new
  incarnation: same TTL, same cleanup. Hand-made databases keep the strict
  rule that a reused name is never touched.
- **Concurrency.** A per-session lock serialises duplicate hook deliveries
  (Cursor also loads Claude hook files); two simultaneous starts for one
  session produce one branch.
- **Crash recovery.** Every operation is safe to repeat; a vanished branch is
  replaced by a fresh one and the context line says so.

## Security

The Skill's full rules are in [skills/pgx/references/security.md](../../../skills/pgx/references/security.md).
In short: no credentials in the conversation (the detection script prints
host and database name only, and the Skill is told not to read credential
files); schema-only by default and no production data without explicit
approval; nothing leaves the machine; no edits to application config; no
deletion of developer databases; official installers only; existing hooks
preserved; sandboxes never weakened; fail closed.

## Troubleshooting

`pgx agent doctor` first. The common cases are in the Skill's
[troubleshooting reference](../../../skills/pgx/references/troubleshooting.md)
and in [troubleshooting.md](troubleshooting.md#coding-agents).
