# PGX for AI coding agents: automatic database isolation

*Design, 2026-10-25. Status: draft for review. Builds on `pgx copy` (PR #28)
and `pgx branch` (PR #29).*

## Intent

A developer should be able to run several coding agents at once against a
real PostgreSQL-shaped application and never have them trample each other's
database. Today that takes two commands per agent, and the second one is
manual:

```sh
pgx copy --from "$DATABASE_URL" --to myapp     # once
pgx branch myapp agent-1                       # per agent, by hand
```

This milestone makes the second command disappear. When Claude Code, Codex
or Cursor starts a coding session, PGX creates a branch for that session,
hands that session (and only that session) a `DATABASE_URL`, keeps the branch
alive while the agent is working, and removes it when the session is over.

Two facts shape everything below (both stated by the product owner):

- **Each agent gets its own `DATABASE_URL`.** Not a global shell variable.
  Two sessions in the same terminal, the same editor or the same worktree
  must see different URLs.
- **A child cannot set its parent's environment.** So the variable has to be
  placed by whoever *spawns* the agent's shell commands: the host tool via a
  hook mechanism that it provides for exactly this purpose, or a wrapper
  that is the parent of the agent process.

## Goals

1. One reusable core in the `pgx` binary; adapters for Claude Code, Codex and
   Cursor that are thin enough to read in one screen.
2. Zero per-task effort after a one-time `pgx agent install <host>`.
3. Per-session isolation that survives the common host behaviours: resume,
   compaction, `/clear`, worktrees, subagents, long autonomous turns.
4. No leaks: every branch has a TTL, and the TTL is an *idle* timeout that
   the host's activity keeps extending, so a three-hour session is safe and a
   forgotten one still disappears.
5. The agent is told what it was given (one line of context), so it does not
   go looking for a database or try to point `DATABASE_URL` elsewhere.
6. Everything is scriptable: `--json` everywhere, stable exit codes, nothing
   interactive.

## Non-goals (this release)

- Resetting or refreshing a branch from its source mid-session (`pgx refresh`
  stays out, as decided for v0.1.x).
- One branch per *subagent*. Subagents share their parent session's branch.
- Cursor cloud/background agents and Codex cloud: they run elsewhere and
  cannot reach a local PGX. The hook detects and skips them.
- Windows. PGX itself is macOS and Linux only.
- Multiple source databases per project, or branching from another branch.
- MCP servers. The integration is environment-based on purpose: every
  framework reads `DATABASE_URL`; nothing needs a tool call.

## Anti-goals

- A hook that can block or slow a coding session. Hooks never exit 2, finish
  in well under a second in the steady state, and degrade to "no database for
  this session, here is why" rather than failing the session.
- Dropping anything that is not an agent branch. The source database and
  hand-made branches are never touched by `down`, `exec` or expiry.
- Printing the password-bearing URL into transcripts. Context lines name the
  branch; the URL goes only into the environment.
- Editing the project's own config (`database.yml`, `.env`) to point at a
  branch. The environment is the only channel.

## What the integrations can actually do (research, 2026-10-25)

| Host | Session start | Per-session env | Session end | Session id | Caveats |
|---|---|---|---|---|---|
| Claude Code 2.1.x | `SessionStart` hook (`source`: startup/resume/clear/compact/fork) | hook appends `export …` lines to `$CLAUDE_ENV_FILE`; every later Bash tool call sources it | `SessionEnd` hook (`reason`: clear/resume/logout/prompt_input_exit/other); default budget 1.5 s, per-hook `timeout` raises it to 60 s | `session_id` in every hook payload; `CLAUDE_CODE_SESSION_ID` in Bash, hooks and MCP subprocesses | Subagents share the session id. `--worktree` sessions get their own id. Not documented whether `SessionEnd` runs on crash/SIGKILL (assume not). `claude -p` runs project hooks even in untrusted folders. |
| Cursor 2.4+ (IDE and `agent` CLI) | `sessionStart` hook; input has `conversation_id` (= `session_id`), `is_background_agent` | hook prints `{"env": {...}}`; Cursor injects it into the agent's shell tool for that conversation | `sessionEnd` hook (`reason`: completed/aborted/error/window_close/user_close); community reports it mostly fires on close | `conversation_id` in every payload | `sessionStart` is fire-and-forget. Env is not inherited by Cursor subagents and is lost when a stopped chat is reopened (no new `sessionStart`). Lifecycle hooks do not fire for cloud agents. Cursor also reads Claude Code hook files (`.claude/settings*.json`) and maps `SessionStart` to `sessionStart`. |
| Codex CLI 0.124+ (hooks stable), 0.145+ (`SessionEnd`) | `SessionStart` hook (`source`: startup/resume/clear/compact/fork), `UserPromptSubmit`, `PreToolUse` | **no hook can set env.** Shell commands do get `CODEX_SESSION_ID` and `CODEX_THREAD_ID`; `PreToolUse` may return `updatedInput` that rewrites the shell command; a wrapper's process env is inherited (`shell_environment_policy.inherit = "all"` is the default) | `SessionEnd` hook (close, archive, or 30 min idle; 1 s default timeout, 3 s max) | `session_id` in every hook payload; `CODEX_SESSION_ID` in commands | Project hooks (`.codex/hooks.json`) load only in trusted projects and must be trust-pinned once via `/hooks`. `workspace-write` sandbox blocks **all** network including 127.0.0.1 unless `sandbox_workspace_write.network_access = true`; `codex exec` defaults to read-only. Hooks also run under `codex exec`. |

Sources: code.claude.com/docs/en/hooks (#persist-environment-variables,
#sessionend), code.claude.com/docs/en/env-vars, cursor.com/docs/hooks,
cursor.com/docs/reference/third-party-hooks, cursor.com/docs/cli/*, plus
string inspection of the installed Claude Code 2.1.310 and `agent`
2026.01.23 binaries, which confirmed `CLAUDE_ENV_FILE`, the Cursor
`sessionStart` → `env` response and the Cursor Claude-hook compatibility
loader.

Spikes run on 2026-10-25 on this machine (throwaway hook configs in a
scratch project, no product code):

- **Claude Code 2.1.310: verified.** A `SessionStart` hook that appended
  `export PGX_SPIKE=…` to `$CLAUDE_ENV_FILE` made the value visible to a
  Bash tool command in a `claude -p` session; `CLAUDE_CODE_SESSION_ID` in
  that command equalled the hook's `session_id`; `PreToolUse` and
  `SessionEnd` both fired, `SessionEnd` on the normal headless exit. The env
  file lives at `~/.claude/session-env/<session_id>/sessionstart-hook-1.sh`.
- **Cursor `agent` 2026.01.23: not yet run.** The CLI is installed but not
  logged in here (`agent login`), so whether the `sessionStart` `env` reaches
  the Shell tool rests on the binary inspection (`setSessionEnvironment`) and
  Cursor staff statements for now. The spike is the first task of the Cursor
  adapter; if it fails, Cursor's environment goes through `preToolUse`
  `updated_input` exactly like Codex's layer 2.
- **Codex: not installed here** (0.164.0 is one `npm i -g @openai/codex`
  away, plus a login).

So: Claude Code and Cursor have a native, documented per-session environment
channel driven by hooks. Codex (and any tool without one) gets a wrapper. The
same core serves both shapes.

## The design

### Vocabulary

- **Agent session**: one conversation in one host, identified by
  `(host, session_id)`. `host` is `claude`, `cursor`, `codex`, or a free
  label for `exec`.
- **Agent branch**: the PGX database created for that session. Name:
  `agent-<host>-<id8>` where `id8` is the first 8 characters of the session
  id when it is already `[a-z0-9]` (UUIDs qualify), otherwise the 16 hex digits of
  FNV-1a 64 of the id (a stable, dependency-free hash; `std`'s
  `DefaultHasher` is not stable across Rust versions and must not be used,
  because a renamed branch would break resume). On a name collision with a *different*
  session the suffix grows to 16. Names always satisfy `names::validate`.
- **Source**: the database the branch is cloned from. Default `app`.
- **Idle TTL**: the branch expires `ttl` after the *last activity*, not after
  creation. Default `2h`, the same default as `pgx branch`.

### Core commands (one new command group)

```
pgx agent up      --id <session> [--host <h>] [--source <db>] [--ttl <dur>] [--env]
pgx agent touch   --id <session> [--host <h>]
pgx agent down    --id <session> [--host <h>] [--now | --grace <dur>]
pgx agent exec    [--source <db>] [--ttl <dur>] [--host <label>] [--keep] -- <cmd> [args…]
pgx agent ls
pgx agent env     [--id <session>] [--host <h>]          # export lines; --id defaults to the host's own session var
pgx agent hook    [--host claude|cursor|codex] [--source <db>] [--ttl <dur>] [--grace <dur>]
pgx agent install   <claude|cursor|codex|all> [--source <db>] [--ttl <dur>] [--user|--local]
pgx agent uninstall <claude|cursor|codex|all> [--user|--local]
```

**`up`** is the idempotent primitive. If the branch for this session exists
and its record carries the same session key, it extends the TTL and returns
the existing URL (`created: false`). Otherwise it clones `source` with
`clone_database()` (the same code path as `clone` and `branch`), records the
TTL with an extra `agent` object in the TTL record, and returns the URL
(`created: true`). If the runtime is initialised but stopped it starts it,
like `pgx run` does; if PGX was never initialised it fails with exit 3 and
the hint `pgx run`. Output: the `pgx branch` block in human mode; with
`--json` the `branch` object plus `session_id`, `host`, `created`; with
`--env` the lines

```sh
export DATABASE_URL='postgres://postgres:…@127.0.0.1:5433/agent-claude-9b1f3c2d'
export PGX_BRANCH='agent-claude-9b1f3c2d'
export PGX_SOURCE='myapp'
```

**`touch`** rewrites the record's `expires_at` to now + ttl. It never opens a
database connection (the supervisor reads the record file), and it skips the
write when the last bump was under 30 seconds ago, so hosts can call it on
every prompt and every shell command.

**`down`** retires the session's branch. Default: shorten the TTL to a grace
period (`--grace`, default `10m`) so `claude --continue` or a reopened Cursor
chat minutes later finds its database intact. `--now` drops it immediately
with force. `down` first checks that the record's `agent` object matches the
session key; it will not drop a database that merely has the right name.
Unknown session: exit 5 from the CLI, silently ignored from `hook`.

**`exec`** is the universal adapter: generate a session id (or take `--id`),
`up`, spawn the command with `DATABASE_URL`, `PGX_BRANCH`, `PGX_SOURCE` and
`PGX_AGENT_SESSION` added to the inherited environment, forward SIGINT,
SIGTERM and SIGHUP to the child, run `touch` every `ttl/4` (at least every
30 s) while the child lives, then `down --now` (or `down` with grace under
`--keep`) and exit with the child's status (128+signal if it was killed).
Nothing about the branch is printed unless `--json` or `--verbose`; the URL
never reaches stdout.

**`env`** prints the same `export` lines as `up --env` for an existing session
without creating or extending anything. Without `--id` it takes the session
from the host variable in its own environment (`CLAUDE_CODE_SESSION_ID`,
`CODEX_SESSION_ID`), which is what lets a command *inside* a session find
its own branch.

**`ls`** lists agent branches only: NAME, HOST, SESSION (shortened), SOURCE,
AGE, EXPIRES IN, SIZE, and `--json` with the full record.

**`hook`** is the one entry point every host config points at. It reads the
host's JSON payload from stdin, detects the host (explicit `--host` wins;
otherwise `CLAUDE_ENV_FILE`/`CLAUDE_CODE_SESSION_ID` in the environment or a
`hook_event_name` in Claude's PascalCase vocabulary means Claude;
`CURSOR_VERSION` in the environment or `conversation_id` in the payload
means Cursor), and dispatches:

| Host | Event | Action | Output |
|---|---|---|---|
| Claude | `SessionStart` (any source) | `up` | appends the three `export` lines to `$CLAUDE_ENV_FILE`; prints one context line to stdout: `PGX: this session's database is branch agent-claude-9b1f3c2d (a copy of myapp). DATABASE_URL is already set in your shell. It is disposable and expires 2h after your last activity.` |
| Claude | `UserPromptSubmit`, `PreToolUse` (matcher `Bash`) | `touch` | nothing |
| Claude | `SessionEnd` | `down`; reason `clear` or `logout` → `--now`, otherwise grace | nothing |
| Cursor | `sessionStart` (skip when `is_background_agent`) | `up` | `{"env":{"DATABASE_URL":…,"PGX_BRANCH":…,"PGX_SOURCE":…},"additional_context":"PGX: …same sentence…"}` |
| Cursor | `beforeSubmitPrompt`, `afterShellExecution` | `touch` | `{}` (Cursor's response validator treats `permission` as optional, so `{}` is valid for every Cursor hook; the touch hooks are nevertheless placed on non-permission events so they can never influence approvals) |
| Cursor | `sessionEnd` | `down` with grace | nothing |
| Codex | `SessionStart` | `up` | `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"PGX: …same sentence…"}}` |
| Codex | `UserPromptSubmit` | `touch` | nothing |
| Codex | `PreToolUse` (shell) | look up URL, rewrite command | `{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{…command with the export prefix…}}}` or nothing when the shape is unknown |
| Codex | `SessionEnd` | `down` with grace | nothing |

`hook` exits 0 on success and on "nothing to do". When PGX is unusable (not
initialised, source missing, clone failed) it prints one line to stderr
(`pgx: no database for this session: source "myapp" does not exist (run pgx
copy --from … --to myapp)`) and exits 1, which both hosts treat as a
non-blocking notice. It never exits 2.

The Cursor compatibility loader means a project with Claude hooks installed
may also deliver those hooks to Cursor with Cursor-shaped payloads. Host
detection makes that harmless: the same command does the right thing under
either host, and installing both is not required for correctness.

**`install`** writes the hook entries for a host into its config file,
merging with whatever is there (parse JSON, append entries whose `command`
starts with `pgx agent hook`, write back pretty-printed, keep every other
hook untouched, refuse to touch a file that is not valid JSON). It also
writes `.pgx.toml` in the project root (see below) when it does not exist.
`uninstall` removes exactly the entries `install` added. Both print the
files they changed and are idempotent.

Scopes: Claude Code → `.claude/settings.json` by default, `--local` for
`.claude/settings.local.json` (per-developer, gitignored), `--user` for
`~/.claude/settings.json`. Cursor → `.cursor/hooks.json` (project) or
`~/.cursor/hooks.json` (`--user`). Codex → `.codex/hooks.json` (project) or
`~/.codex/hooks.json` (`--user`). The hook command written is
a guarded one-liner so a committed hook file is harmless for a teammate (or
a Dock-launched editor) that does not have `pgx` on its PATH:

```sh
/bin/sh -c 'PATH="$PATH:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin"; command -v pgx >/dev/null 2>&1 && exec pgx agent hook; exit 0'
```

No `pgx` means a silent no-op, not a notice on every prompt. The PATH suffix
covers the installer's `~/.local/bin` and the usual Homebrew and npm global
locations without hard-coding a machine-specific absolute path. `uninstall`
removes entries whose command *contains* `pgx agent hook`. `install` runs the
exact command it wrote once, with a synthetic payload, and reports the
`pgx` it resolved to, so a PATH problem shows up at install time rather than
in an invisible fire-and-forget hook. There is no `--source` flag in the
command; the source comes from `.pgx.toml`, so the same committed hook file
works on every clone of the repo. Timeouts:
start 30 s (a cold runtime start may take a second or two), end 10 s, the
`touch` hooks 5 s.

### Project configuration: `.pgx.toml`

PGX is one runtime per user, so database names are global across projects.
Two projects cannot both call their source `app`. The branch source therefore
has to be a per-project fact, and hooks installed at user level have no flag
to carry it. A tiny file in the project root settles it:

```toml
# .pgx.toml — written by `pgx agent install`, safe to commit
[agent]
source = "myapp"   # the database `pgx branch`es from; default "app"
ttl = "2h"         # idle TTL for agent branches
```

Resolution order for the source, everywhere in the core: `--source` flag >
`PGX_SOURCE` environment variable > `.pgx.toml` found by walking up from the
hook's `cwd`, then from the host's project root when that differs
(`CLAUDE_PROJECT_DIR`, `CURSOR_PROJECT_DIR`, Cursor's `workspace_roots[0]`;
`exec` uses the current directory) > `app`. `install` writes the file at the
git top level when inside a repository, otherwise in the current directory,
and tells the user to commit it. When hooks are installed but resolution
still lands on the default, `hook` says so on stderr and in the context line
(`a copy of app, which was created empty because no .pgx.toml names a
source`), so an agent working against an unexpectedly empty database has the
explanation in front of it. When the resolved source is the default `app` and it does
not exist, `up` creates it empty (exactly what `pgx run` does) so a project
that never imported anything still gets a working, isolated database; an
explicitly named source that is missing is an error with the `pgx copy` hint.

### Codex

Codex has the same lifecycle hooks as the other two (`SessionStart`,
`UserPromptSubmit`, `SessionEnd`, `PreToolUse`), keyed by `session_id`, but
its hook protocol has no way to publish environment variables to later
commands (the `SessionStart` output schema has only `additionalContext`).
Two things it does offer: every shell command runs with `CODEX_SESSION_ID`
in its environment, and a `PreToolUse` hook may return `updatedInput` that
rewrites the command about to run. The adapter therefore has two layers:

1. **Lifecycle through hooks, the same as Claude and Cursor.** `pgx agent
   install codex` writes `.codex/hooks.json` with `SessionStart` → `up`,
   `UserPromptSubmit` → `touch`, `SessionEnd` → `down`. `hook` detects Codex
   by `CODEX_THREAD_ID`/`CODEX_SESSION_ID` in its environment or by the
   payload shape (`permission_mode` and `transcript_path` next to a
   PascalCase `hook_event_name`, with no `CLAUDE_ENV_FILE`). `SessionStart`
   answers with `hookSpecificOutput.additionalContext` carrying the same
   context sentence as the other hosts. `SessionEnd` fires on close, archive
   or 30 minutes idle, with a 1 s default timeout, which the grace-only
   `down` fits.

2. **Environment through command rewriting.** A `PreToolUse` hook matched on
   the shell tool returns `updatedInput` with the command prefixed by
   `export DATABASE_URL='…' PGX_BRANCH='…' PGX_SOURCE='…'; ` when the
   command is in Codex's standard `["bash","-lc","…"]` (or `zsh`/`sh -c`)
   shape, and leaves anything else untouched. The URL is looked up from the
   session's record (no connection, same cost as `touch`), so this adds well
   under 10 ms per command. This is the part that makes Codex *automatic*,
   and it is the part that rests on an inference from the docs' own
   `updatedInput` example rather than on a documented guarantee, so it is
   built behind a spike: the first implementation task is a real
   `codex exec` run that proves `echo $DATABASE_URL` inside the session
   prints the branch URL. If the spike fails, Codex ships with layer 1 plus
   the wrapper below, and the context sentence tells the model to prefix
   database commands with `$(pgx agent env)`; `pgx agent env` reads
   `CODEX_SESSION_ID` from its own environment when `--id` is absent and
   prints the `export` lines.

3. **The wrapper, for every tool without hooks and for Codex users who do
   not want hook trust prompts:** `pgx agent exec -- codex [args]` and
   `pgx agent exec -- codex exec "…"`. Codex inherits the wrapper's
   environment by default and forwards it to shell commands.

Sandboxing is the Codex-specific wrinkle. In `workspace-write` the sandbox
has no network at all, loopback included (Seatbelt denies network-outbound;
on Linux the sandbox runs in a fresh network namespace with `connect`
denied by seccomp), so the TCP URL fails with `CODEX_SANDBOX_NETWORK_DISABLED=1`
set in the command's environment. `install codex` does **not** change the
sandbox policy on its own: it prints the one setting the user needs,
`-c sandbox_workspace_write.network_access=true` (or the same key in
`.codex/config.toml`), and `hook` includes the same line in the context
sentence when it sees `CODEX_SANDBOX_NETWORK_DISABLED=1`. The Unix-socket
URL is a possible future alternative on macOS (`network.allow_unix_sockets`)
but is not part of this release.

Project hooks in Codex load only for trusted projects and must be
trust-pinned once (the `/hooks` TUI, or `--dangerously-bypass-hook-trust`
for one run); the docs say so and `install codex` says so too.

### Record format

The TTL record gains one optional object; the schema number stays 1 because
older binaries read only the keys they know and never delete on the strength
of unknown ones:

```json
"agent": { "host": "claude", "session_id": "9b1f3c2d-…", "source": "myapp",
           "cwd": "/Users/alex/code/myapp", "last_touch": 1761420000 }
```

`pgx ls` shows agent branches like any other TTL database; `pgx agent ls`
filters on the object. The supervisor is unchanged: expiry is still
`expires_at`, which `touch` moves forward.

### What the agent experiences

Claude Code, after `pgx agent install claude` in the project:

```
$ claude
… session starts; a context line arrives:
PGX: this session's database is branch agent-claude-9b1f3c2d (a copy of myapp).
DATABASE_URL is already set in your shell. It is disposable and expires 2h
after your last activity.
> bin/rails db:migrate            # runs against agent-claude-9b1f3c2d
```

A second `claude` in another terminal, or `claude --worktree fix-123`, gets
`agent-claude-4e77a0c1`. `/clear` drops the old branch and the new session
gets a fresh one. `claude --continue` within ten minutes of quitting finds
the same branch.

Cursor: the same, through `.cursor/hooks.json`; each chat tab is its own
branch. Codex and everything else: `pgx agent exec -- codex`.

## Constraints

- Rust, in `crates/bin/pgx` (new `cmd/agent.rs`, small additions to
  `cli.rs`, `ttlstore.rs`, `cmd/db.rs`). No new crates beyond what is already
  in the tree (`serde_json` is present; a TOML reader for `.pgx.toml` can be
  the twenty-line subset we need, since the file has two keys).
- Hook commands must be fast: `touch` is a file rewrite, no connection.
- Follows the existing CLI conventions: hand-rolled parser, `Out`, `PgxError`
  with hints, stable exit codes 0/1/2/3/4/5/6/7, `--json` one object.
- Black-box tests in `crates/bin/pgx/tests/agent.rs` drive the real binary
  with synthetic host payloads and a fake `CLAUDE_ENV_FILE`; they do not
  require Claude Code, Cursor or Codex to be installed. CI gains
  `--test agent`.
- Manual validation before the PR: a real `claude -p` session and a real
  Cursor `agent -p` session in a scratch project (both binaries are on this
  machine), with the resulting `DATABASE_URL` observed from inside the
  session. Codex needs a binary (`npm i -g @openai/codex`, 0.164.0) and a
  login, neither of which is on this machine today; the Codex spike cannot
  run without them.
- No deletions of the user's environments or data as part of the work.

### The source must be idle

`CREATE DATABASE … TEMPLATE` refuses a template that other sessions are
connected to (PostgreSQL's own rule, both clone strategies). `up` therefore
fails fast with exit 6 and a specific hint when the source has live
connections: `source "myapp" has 2 active connections; a branch source must
be idle. Keep it as a template nobody connects to (pgx copy --to
myapp_template) or disconnect first.` No retry loop: connections that block
a clone are long-lived by nature. The documentation presents the source as
exactly that, a template that only `pgx agent` and `pgx branch` read,
matching the `pgx copy` guidance already in `copy.md`. The developer's own
editable database stays in PostgreSQL (the product story) or in a PGX
database that is not the branch source.

### Settled details the reviewer asked about

- `up` checks that the recorded database still exists with the recorded OID;
  if it is gone it re-clones, reports `created: true`, and the context line
  says "fresh".
- `pgx ttl <agent-branch> <dur>` preserves the `agent` object (`cmd/ttl.rs`
  updates the record instead of rebuilding it), so a manual TTL change never
  detaches a branch from its session.
- A database with the derived name but no matching `agent` record is
  foreign: the suffix grows, and it is never dropped.
- `up` calls `spawn_supervisor` opportunistically (single-instance, cheap) so
  a dead supervisor cannot turn the idle TTL into a leak.
- `exec` labels the host with the child's basename unless `--host` says
  otherwise; its heartbeat interval is `max(ttl/4, 30s)`, clamped below the
  TTL.
- `ls` lists agent branches of all hosts; `pgx ls` keeps showing them as
  ordinary TTL databases.

## Risks and how the design meets them

- **Host end-of-session hooks are not guaranteed** (Claude on SIGKILL, Cursor
  in general). The idle TTL is the real cleanup; `down` is an optimisation.
- **Cursor env lost on chat reopen, and not yet observed reaching the
  Shell tool.** The Cursor spike (needs `agent login`) comes first; if `env`
  does not reach the shell, the Cursor adapter uses `preToolUse`
  `updated_input` to prefix Shell commands, the same layer-2 mechanism as
  Codex, and the reopen gap closes with it. Otherwise reopen stays a
  documented limitation and the context line tells the agent to start a new
  chat when `DATABASE_URL` is unset.
- **Claude's 1.5 s SessionEnd budget.** `down` with grace is a file write;
  `--now` is one `DROP DATABASE`; both fit, and `install` sets `timeout: 10`.
- **Project hooks run for teammates without PGX.** The guarded command is a
  silent no-op without `pgx`; `--local` keeps hooks out of the repo
  entirely for those who prefer that.
- **Codex command rewriting is inferred, not promised.** It is gated on a
  spike; the wrapper and the hook lifecycle do not depend on it.
- **Codex sandbox has no loopback network.** Documented, printed by
  `install`, and repeated in the agent's context when detected.
- **Name collisions across hosts or projects.** The session key, not the
  name, is the identity; collisions grow the suffix.

## Left to the builder

- Exact struct shapes, the TOML subset reader, signal forwarding details in
  `exec` (reuse `process.rs`), and the heartbeat thread.
- Wording of help text and docs, following `cli.md` and `ttl.md` style.
- Whether `pgx doctor` should report agent branches (nice, not required).

## Decisions the product owner has not made yet

1. The command group's name. `pgx agent` reads naturally but PGRun already
   has a host component called `pgx-agent`; `pgx session` or `pgx attach`
   are the alternatives.
2. Whether a committed `.pgx.toml` in the project root is acceptable as the
   place that names the branch source (the design depends on some
   per-project fact that survives user-level hook installs).
3. Validation access: `agent login` for the Cursor spike, and a Codex
   install plus login for the Codex spike; or ship those two adapters
   validated only by black-box tests with synthetic payloads.
4. `down` default on session end: a 10-minute grace (resume keeps the data)
   versus an immediate drop.
