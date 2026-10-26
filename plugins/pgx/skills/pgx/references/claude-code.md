# Claude Code

Status: **verified live** (Claude Code 2.1.310, macOS): the SessionStart
hook's exports reached Bash tool commands, the session id matched
`CLAUDE_CODE_SESSION_ID`, PreToolUse and SessionEnd fired, SessionEnd fired on
a normal headless exit.

## Two ways to install the skill in Claude Code

- **Plugin (native)**: `/plugin marketplace add pgrundev/pgx` then
  `/plugin install pgx@pgrundev` inside a session (shell:
  `claude plugin marketplace add pgrundev/pgx && claude plugin install
  pgx@pgrundev`). The skill is namespaced `/pgx:pgx`; `claude plugin list`
  shows `pgx@pgrundev`, `claude plugin update pgx@pgrundev` upgrades it,
  `/plugin uninstall pgx@pgrundev` removes it (the skill only: databases,
  templates, `.pgx.toml` and project hooks are untouched).
- **Skills CLI**: `npx skills add pgrundev/pgx --skill pgx` copies the skill
  into `.claude/skills/pgx` (plus `.agents/skills/pgx` for Cursor and Codex).
  It loads as the plain skill `/pgx`.

Both copies are the same skill. With both installed, Claude Code lists `/pgx`
and `/pgx:pgx` side by side and either runs the same workflow; nothing is
duplicated at the database level because hooks and branches come from `pgx
agent setup`, never from the skill files. To keep one copy, remove the Skills
CLI one with `npx skills remove pgx` or the plugin with `/plugin uninstall
pgx@pgrundev`.

The plugin contains no hooks: `pgx agent setup` installs the project's hooks
into `.claude/settings.json` (idempotent, ownership-checked, see
[agent-lifecycle.md](agent-lifecycle.md)), so a plugin installed at user scope
does nothing in projects that have not opted in with `.pgx.toml`.

## How it works

`pgx agent setup` (or `pgx agent install claude`) adds four entries to
`.claude/settings.json`:

- `SessionStart` → `pgx agent up` for `session_id`, appends the nine
  `export` lines to `$CLAUDE_ENV_FILE` (Claude runs that file before every
  Bash command), prints the context line you see at the top of the session.
  Fires on startup, resume, `/clear`, compaction and fork; resume and
  compaction find the same branch; `/clear` and fork are new session ids and
  get new branches.
- `UserPromptSubmit` and `PreToolUse` (matcher `Bash`) → `pgx agent touch`.
- `SessionEnd` → `pgx agent down` (grace period). `claude --continue` within
  10 minutes finds the branch intact.

Subagents run inside the same session and share its branch. `claude
--worktree <name>` starts a new session id in a new worktree and therefore
gets its own branch; `.pgx.toml` must be committed so the worktree sees it.

## In the session that ran setup

That session had no SessionStart after the hooks were installed, so its Bash
environment has no `DATABASE_URL` from PGX. Use `pgx agent exec -- <cmd>`:
it attaches to this session (`CLAUDE_CODE_SESSION_ID`), creates the branch
on first use, and keeps it. Or tell the user to start a new session.

## Scope choices

`--scope project` (default) writes `.claude/settings.json`, shared through
git; teammates without PGX get a silent no-op. `--scope local` writes
`.claude/settings.local.json` (personal, gitignored by Claude Code).
`--scope user` writes `~/.claude/settings.json` for every project; then the
source comes from each project's `.pgx.toml`.

## Trust and headless runs

Project hooks run in trusted folders; `claude -p` runs them regardless
(unless `--bare`). The hook touches only the local PGX home.

## Checking

`pgx agent doctor` inside the session: the `session` line must say that
`DATABASE_URL` is the branch. `echo $PGX_BRANCH` in Bash shows the branch.
