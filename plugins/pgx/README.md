# PGX — Postgres for Agents

One plugin. Every Claude Code agent gets its own Postgres.

PGX is a local, PostgreSQL-compatible runtime built for disposable databases:
copy-on-write branches created in milliseconds and removed automatically when
their time-to-live runs out. This plugin ships the `pgx` Agent Skill, which
gives each Claude Code session its own branch of your project's database, so
parallel agents, migrations and test runs never touch your real PostgreSQL and
never overwrite one another.

## Install inside Claude Code

```text
/plugin marketplace add pgrundev/pgx
/plugin install pgx@pgrundev
```

Pick the scope in the dialog: **user** makes the skill available in every
project on this machine, **project** or **local** limit it to one repository.
Shell equivalent: `claude plugin marketplace add pgrundev/pgx` then
`claude plugin install pgx@pgrundev`.

## Use it

Invoke the skill explicitly:

```text
/pgx:pgx Set up isolated Postgres databases for this project.
```

or just describe database work ("run the pending migrations and the tests",
"set up Postgres for my agents"); Claude selects the skill when it is relevant.

The first activation in a project walks through detection of the existing
database setup, a one-time `pgx agent setup`, and from then on every new
Claude Code session in that project starts with `DATABASE_URL` pointing at its
own branch.

## What it contains, and what it does not

- The skill (`skills/pgx/SKILL.md`), its reference documents and two helper
  scripts (project detection, version check). About 190 tokens of context
  while installed.
- No hooks, MCP servers or background processes. Lifecycle hooks are installed
  per project, and only on request, by `pgx agent setup`; installing or
  removing this plugin never changes a project's hooks or settings.
- No database engine. The `pgx` binary (PGX 0.1.2 or newer) is installed
  separately when the skill first needs it: `npm i -g pgxdb` or the official
  installer. The skill asks before downloading anything.

Removing the plugin (`/plugin uninstall pgx@pgrundev`) removes the skill only.
Databases, templates, `.pgx.toml` and the PGX runtime stay; `pgx agent
uninstall` removes a project's hooks when you want that.

The same skill is also installable with the Agent Skills CLI for Cursor and
Codex: `npx skills add pgrundev/pgx --skill pgx`. Documentation:
https://github.com/pgrundev/pgx/blob/main/docs/pgx/local-dev/agents.md
