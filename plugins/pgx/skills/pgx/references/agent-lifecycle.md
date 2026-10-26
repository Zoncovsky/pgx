# The `pgx agent` lifecycle and `.pgx.toml`

## Identity

A session is `(project, host, session id)`. The project is the canonical root
holding `.pgx.toml` (or the git top level, or the directory itself); the host
is `claude`, `cursor`, `codex` or a label for `exec`; the session id comes
from the host (`session_id` in hook payloads, `CLAUDE_CODE_SESSION_ID` /
`CODEX_SESSION_ID` in commands) or from `--id`. Two projects on one machine
can both use a source called `app` without their sessions colliding, because
the project is part of the identity.

## Commands

| Command | Role |
|---|---|
| `pgx agent setup [--source <db>] [--hosts …] [--scope project\|local\|user] [--no-hooks]` | One-time: runtime, template check, `.pgx.toml`, hooks, proof branch, hook self-test. Idempotent. |
| `pgx agent doctor` | Config, runtime, supervisor, source idle, hooks per host (missing/partial/duplicate), the current session's branch and `DATABASE_URL`, branch counts. Exit 1 on failure. |
| `pgx agent up [--env]` | Ensure this session's branch (create or extend). The hook calls it. |
| `pgx agent touch` | Activity: deadline = now + idle TTL (max once per 30 s, no connection). |
| `pgx agent down [--now]` | Session end: retire (grace) or drop now. Only this session's branch. |
| `pgx agent env` | `export` lines for an existing branch: `eval "$(pgx agent env)"`. |
| `pgx agent exec -- cmd` | Wrapper: own branch for the command, or attach to the surrounding Claude/Codex session. |
| `pgx agent ls` | Agent branches. |
| `pgx agent hook` | What hook files run; detects the host from the payload. |
| `pgx agent install/uninstall [hosts] [--scope …]` | Merge/remove our hook entries; other entries untouched. |

All commands take `--json` (`pgx --json agent …`) and use the stable exit
codes: 2 usage, 3 not initialised/stopped, 4 conflict, 5 not found, 6 source
busy.

## `.pgx.toml`

```toml
# .pgx.toml — PGX agent configuration for this project (safe to commit).
[agent]
source = "myapp_template"   # the PGX database every agent session branches from
idle_ttl = "2h"             # a branch expires this long after the agent's last activity
grace_period = "10m"        # how long a branch survives after its session ends
```

Names and durations only. A connection URL in it is rejected. Resolution
order for the source everywhere: `--source` > `PGX_SOURCE` > `.pgx.toml`
(walking up from the session's directory, then the host's project root) >
`app`. The default `app` is created empty when missing; a named source that
is missing is an error with the `pgx copy` hint.

## Hook files `setup`/`install` write

The command is guarded, so a committed hook file is a silent no-op for a
teammate without PGX:

```
/bin/sh -c 'PATH="$PATH:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin"; command -v pgx >/dev/null 2>&1 && exec pgx agent hook; exit 0'
```

- Claude Code `.claude/settings.json` (`--scope local`: `settings.local.json`;
  `user`: `~/.claude/settings.json`): `SessionStart` (30 s), `UserPromptSubmit`
  (5 s), `PreToolUse` matcher `Bash` (5 s), `SessionEnd` (10 s).
- Cursor `.cursor/hooks.json` (`user`: `~/.cursor/hooks.json`): `sessionStart`,
  `beforeSubmitPrompt`, `afterShellExecution`, `sessionEnd`. Touch hooks sit on
  non-permission events so they never influence approvals.
- Codex `.codex/hooks.json` (`user`: `~/.codex/hooks.json`): `SessionStart`,
  `UserPromptSubmit`, `SessionEnd`. Codex project hooks load for trusted
  projects and must be trust-pinned once (`/hooks` in Codex).

Existing entries (other tools' hooks, the user's own) are preserved
byte-for-byte; `install` adds ours where missing, `uninstall` removes only
ours and never deletes `.pgx.toml`.

## Timing and failure

- Idle TTL default 2h, grace 10m. A session that runs for hours stays alive
  because every prompt and shell command touches it.
- Hooks never exit 2 (blocking). On failure they print one line to stderr
  and, when `.pgx.toml` exists, fail closed: `DATABASE_URL` becomes
  `postgres://pgx-isolation-failed@127.0.0.1:1/pgx_isolation_failed` and
  `PGX_ISOLATION=failed` with the reason in `PGX_ISOLATION_REASON`.
- Concurrency: a per-session lock serialises duplicate deliveries (Cursor
  loads Claude hook files too); the database's own uniqueness check is the
  last line. Two simultaneous starts for one session yield one branch.
- Crash recovery: every operation is safe to repeat. A record whose database
  vanished is forgotten and a fresh branch made (the context line says
  "fresh"); a database whose record is gone is just a database with no TTL
  owner and is never dropped by `down`.
