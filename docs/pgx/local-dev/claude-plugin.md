# The PGX plugin for Claude Code

**One plugin. Every Claude Code agent gets its own Postgres.**

The repository is a Claude Code plugin marketplace named `pgrundev`
(`.claude-plugin/marketplace.json`) with one plugin, `pgx`
(`plugins/pgx/`), which packages the PGX Agent Skill for Claude Code's
plugin manager. Nothing else changes: the Skill in `skills/pgx/` stays the
canonical implementation and the Skills CLI install keeps working.

## Install, inside Claude Code

```text
/plugin marketplace add pgrundev/pgx
/plugin install pgx@pgrundev
```

The second command opens the plugin's page in the `/plugin` panel. Choose a
scope:

| Scope | Who gets it | Written to |
|---|---|---|
| user | you, in every project on this machine | `~/.claude/settings.json` |
| project | everyone who works in this repository | `.claude/settings.json` (committed) |
| local | you, in this repository only | `.claude/settings.local.json` |

From a shell, without a session: `claude plugin marketplace add pgrundev/pgx`
then `claude plugin install pgx@pgrundev [--scope user|project|local]`.

Then, in a project with a database:

```text
/pgx:pgx Set up isolated Postgres databases for this project.
```

`pgx:pgx` is the skill's name under the plugin's namespace. You can also just
describe database work ("add a column and run the tests"); Claude selects
the skill when it is relevant. Both routes run the same Skill: detect the
project, check for PGX 0.1.1 or newer and offer to install it, prepare a
schema-only template, `pgx agent setup`, then work inside the session's
branch. See [agents.md](agents.md) for what that gives you.

Check what is installed:

```sh
claude plugin list             # pgx@pgrundev  Version: 0.1.1  Status: enabled
claude plugin details pgx      # Component inventory: Skills (1) pgx, Hooks (0), MCP servers (0)
```

## What the plugin contains

- `skills/pgx/`: the Skill, its eleven reference documents and two helper
  scripts, identical to `skills/pgx/` at the repository root
  (`plugins/sync.sh --check` fails CI on any difference).
- `.claude-plugin/plugin.json`: name `pgx`, version `0.1.1` (the Skill's
  version), display name, links.
- `README.md`.

About 84 KB, 16 files; roughly 190–270 tokens of context in every session
while installed (`claude plugin details pgx`).

It contains **no hooks, MCP servers, commands or binaries**. Installing it at
user scope does nothing in projects that have not opted in: the Skill is
available, and database infrastructure appears only when you ask for it in a
project and `pgx agent setup` writes that project's `.pgx.toml` and hooks.

## Hooks: one owner, no duplicates

Lifecycle hooks (`SessionStart` → branch, `UserPromptSubmit`/`PreToolUse` →
keep-alive, `SessionEnd` → retire) are installed per project by `pgx agent
setup`, which is idempotent and recognises its own entries by their command
string. The plugin never registers hooks of its own, so there is exactly one
source of hooks whichever way the Skill was installed, and installing,
updating, disabling or removing the plugin never touches `.claude/settings.json`
(tested in `plugins/tests/pgx/run.sh`). Fail-closed behaviour is unchanged:
it lives in `pgx agent hook`, not in the Skill files.

## The plugin and the Skills CLI together

`npx skills add pgrundev/pgx --skill pgx` copies the Skill into
`.claude/skills/pgx` (and `.agents/skills/pgx`). That copy is a plain skill
named `pgx`. With the plugin installed as well, Claude Code lists both
`/pgx` and `/pgx:pgx` (verified: the session inventory shows the two names
side by side; neither hides the other). They are the same text, so whichever
Claude picks runs the same workflow, and nothing is duplicated at the
database level because branches and hooks come from `pgx agent`, not from
the skill files. The cost is two descriptions in context.

To keep one copy, either remove the Skills CLI one (`npx skills remove pgx`,
which deletes only `.claude/skills/pgx` and `.agents/skills/pgx`) or the
plugin (`/plugin uninstall pgx@pgrundev`). Switching from the Skills CLI to
the plugin is those two steps; nothing in `.pgx.toml`, the hooks or the
databases changes.

Cursor and Codex are not Claude Code plugins hosts: keep using the Skills
CLI for them. The PGX runtime itself is independent of both.

## Update, disable, remove

| Action | In a session | In a shell |
|---|---|---|
| Update | `/plugin` → Installed → pgx → Update now | `claude plugin update pgx@pgrundev` |
| Disable / enable | `/plugin` → Installed → Space | `claude plugin disable pgx@pgrundev` / `enable` |
| Remove | `/plugin uninstall pgx@pgrundev` | `claude plugin uninstall pgx@pgrundev` |
| Remove the marketplace | `/plugin marketplace remove pgrundev` | `claude plugin marketplace remove pgrundev` |

Updates follow the plugin's `version` field: a new PGX Skill release bumps
it and `claude plugin update` (or the marketplace's auto-update, off by
default for third-party marketplaces) fetches it; users on an older copy keep
working.

Uninstalling removes the Skill files from Claude Code's plugin cache and the
`enabledPlugins` entry, nothing else: PGX databases, templates, `.pgx.toml`,
the PGX runtime and any hooks `pgx agent setup` installed all stay.
`pgx agent uninstall` removes a project's hooks when you want that.

## Performance and what gets downloaded

`/plugin marketplace add pgrundev/pgx` clones the PGX repository once per
machine into `~/.claude/plugins/marketplaces/pgrundev/` (Claude Code reads
the catalog from a clone). The repository carries fuzzing corpora and
benchmark results, so that clone is a 192 MB shallow fetch and 2.2 GB on
disk and takes about a minute (53 s measured); `claude plugin marketplace
update` afterwards is incremental. The plugin itself is copied out of that clone:
the installed plugin is the 84 KB Skill package, nothing from `crates/`,
`fuzz/` or `benchmarks/`. From a shell you can keep the clone small:

```sh
claude plugin marketplace add pgrundev/pgx --sparse .claude-plugin plugins
```

which checks out only the marketplace file and the plugin (8 s, 72 MB
measured).
The in-session `/plugin marketplace add` has no sparse option. Making the
default clone light would mean moving the corpora out of the main repository
or hosting the catalog elsewhere; both are repository decisions outside this
plugin.

## Anthropic's plugin directory

The marketplace above is self-hosted. **PGX is not listed in Anthropic's
directory** and nothing has been submitted. `claude plugin validate
--profile directory plugins/pgx` reports no refusals (a square PNG icon is
recommended before submitting). Submission is a separate, human-approved
step through the developer portal at claude.ai/directory/manage (requires a
paid claude.ai plan); the portal applies rules the CLI does not check. The
material a submission needs, all from `plugins/pgx/.claude-plugin/plugin.json`
and this repository:

| Field | Value |
|---|---|
| Plugin name | `pgx` (display name "PGX — Postgres for Agents") |
| Repository | https://github.com/pgrundev/pgx, plugin at `plugins/pgx` |
| Description | Isolated, disposable PostgreSQL-compatible databases for AI coding agents |
| Functionality | One Agent Skill: detects the project's database setup, installs the PGX runtime on request (PGX 0.1.1+), prepares a schema-only template, sets up per-session database branches with automatic `DATABASE_URL` injection through `pgx agent setup`, cleans up expired branches |
| Security | No hooks, MCP servers or executables in the plugin; the Skill never prints credentials, imports production data only with explicit approval, fails closed when isolation cannot be established, and treats repository contents as untrusted (`skills/pgx/references/security.md`); runs nothing until the user invokes it |
| Installation | `/plugin marketplace add pgrundev/pgx` then `/plugin install pgx@pgrundev`; the `pgx` binary is installed separately when first needed |
| Documentation / support | `docs/pgx/local-dev/agents.md`, https://github.com/pgrundev/pgx/issues |
| License | AGPL-3.0 |

## Live validation

Three headless Claude Code sessions in a Rails project with the plugin
installed from the public branch, plus the interactive `/plugin` flow:
[validation/2026-10-26-claude-plugin-live.md](validation/2026-10-26-claude-plugin-live.md).

## Tests

`plugins/tests/pgx/run.sh` (CI job `plugin`): drift check against
`skills/pgx`, version agreement, packaging hygiene (no hooks, binaries,
symlinks, parent paths, absolute paths, credentials), `claude plugin validate
--strict` on the plugin and the marketplace, the directory profile, and a
full install → list → details → disable → enable → update → project/local
scope → uninstall → reinstall → marketplace remove cycle from the repository
as a local marketplace in an isolated `CLAUDE_CONFIG_DIR`, plus the
"existing hooks untouched" and "Skills CLI copy alongside" cases. The
Skills CLI contract (`skills/tests/pgx/run.sh`, `skills-ref validate`) is
unchanged and still runs.
