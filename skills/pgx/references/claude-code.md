# Claude Code

Status: **verified live** (Claude Code 2.1.310, macOS): the SessionStart
hook's exports reached Bash tool commands, the session id matched
`CLAUDE_CODE_SESSION_ID`, PreToolUse and SessionEnd fired, SessionEnd fired on
a normal headless exit.

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
