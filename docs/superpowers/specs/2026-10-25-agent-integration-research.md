# Host research notes for the agent integration (2026-10-25)

Companion to `2026-10-25-agent-integration-design.md`: the facts the
adapters are built on, with the exact payload and response shapes the
builder needs. Sources are the official docs plus string inspection of the
binaries installed on the design machine (Claude Code 2.1.310, Cursor
`agent` 2026.01.23). [unverified] marks what the docs do not state.

## Claude Code

- Docs: code.claude.com/docs/en/hooks, /env-vars, /settings, /headless,
  /worktrees, /sub-agents, /cli-reference.
- Hook config shape (`.claude/settings.json`, `.claude/settings.local.json`,
  `~/.claude/settings.json`; entries merge across files):
  ```json
  {"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"pgx agent hook","timeout":30}]}],
            "UserPromptSubmit":[{"hooks":[{"type":"command","command":"pgx agent hook","timeout":5}]}],
            "PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"pgx agent hook","timeout":5}]}],
            "SessionEnd":[{"hooks":[{"type":"command","command":"pgx agent hook","timeout":10}]}]}}
  ```
- Common stdin fields: `session_id`, `transcript_path`, `cwd`,
  `hook_event_name`, `permission_mode`; `SessionStart` adds `source`
  (`startup|resume|clear|compact|fork`); `SessionEnd` adds `reason`
  (`clear|resume|logout|prompt_input_exit|other`); subagent hooks add
  `agent_id`, `agent_type` (same `session_id` as the parent).
- Env for the session: `CLAUDE_ENV_FILE` is set for `SessionStart` (and
  Setup/CwdChanged/FileChanged) hooks only. Append shell lines
  (`export NAME=value`); Claude runs the file before every Bash tool command.
  Always append (`>>`), other hooks share the file. [unverified] whether
  subagent Bash and MCP servers see it; whether a `cd` clears SessionStart
  values (docs say CwdChanged values are cleared on the next CwdChanged).
- `SessionStart` stdout (exit 0) is added to Claude's context.
- `SessionEnd`: default total budget 1.5 s; a per-hook `timeout` raises it
  to at most 60 s; `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS` overrides. Runs
  on SIGTERM; [unverified] on crash/SIGKILL and on a normal `claude -p`
  exit.
- Env vars in Bash/hook subprocesses: `CLAUDE_CODE_SESSION_ID` (matches the
  hook `session_id`; changes on `/clear`), `CLAUDECODE=1`,
  `CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDE_PROJECT_DIR` (hooks).
- Exit codes: 0 = ok (JSON parsed), 2 = block (never use), other =
  non-blocking, stderr shown.
- `claude --worktree <name>` gives a new session id; `--session-id <uuid>`
  lets a wrapper choose it; `--settings '{"env":{…}}'` is per-process env for
  `-p` runs. Project hooks need folder trust interactively; `-p` runs them
  regardless unless `--bare`.

## Cursor (IDE and `agent` CLI)

- Docs: cursor.com/docs/hooks, /docs/reference/third-party-hooks,
  /docs/configuration/worktrees, /docs/cli/*, /docs/cloud-agent/setup.
- Hook config: `.cursor/hooks.json` (project), `~/.cursor/hooks.json`
  (user), enterprise paths; Cursor also loads `.claude/settings*.json` hooks
  and maps `SessionStart→sessionStart`, `PreToolUse→preToolUse`,
  `Bash→Shell` (the CLI has a `claudeCodeHooksEnabled` switch, default off in
  the 2026.01 bundle).
  ```json
  {"version":1,"hooks":{"sessionStart":[{"command":"pgx agent hook","timeout":30}],
                         "beforeSubmitPrompt":[{"command":"pgx agent hook","timeout":5}],
                         "beforeShellExecution":[{"command":"pgx agent hook","timeout":5}],
                         "sessionEnd":[{"command":"pgx agent hook","timeout":10}]}}
  ```
- Common stdin: `conversation_id`, `generation_id`, `model`,
  `hook_event_name` (camelCase), `cursor_version`, `workspace_roots`,
  `user_email`, `transcript_path`. `sessionStart` adds `session_id`
  (= `conversation_id`), `is_background_agent`, `composer_mode`.
  `sessionEnd` adds `reason` (`completed|aborted|error|window_close|user_close`),
  `duration_ms`, `final_status`. `beforeShellExecution` adds `command`,
  `cwd`, `sandbox`.
- Hook process env: `CURSOR_PROJECT_DIR`, `CURSOR_VERSION`,
  `CURSOR_USER_EMAIL`, `CURSOR_TRANSCRIPT_PATH`, `CLAUDE_PROJECT_DIR`.
- `sessionStart` response: `{"env": {"K":"V"}, "additional_context": "…"}`.
  `env` is injected into the agent's shell tool for that conversation
  (confirmed in the bundle: `setSessionEnvironment`). Fire-and-forget: the
  first turn does not wait for it. Known gaps (staff-acknowledged): not
  inherited by Cursor subagents; not re-applied when a stopped chat is
  reopened (no new `sessionStart`); lifecycle hooks do not fire for cloud
  agents.
- `beforeShellExecution` response: `permission` (`allow|deny|ask`),
  `user_message`, `agent_message`; it cannot rewrite the command.
  `preToolUse` can (`updated_input`).
- `sessionEnd` output is ignored; community reports it fires mainly on
  window/tab close. Exit 0 = JSON used, 2 = block, other = fail open.
- CLI: `agent -p "…" --output-format json|stream-json` emits `session_id`;
  `--resume [chatId]`, `--continue`; the shell tool inherits the launching
  process's environment, runs `zsh -c` sourcing only `~/.zshenv`.
- Worktrees: `.cursor/worktrees.json` setup commands run in a separate
  process (`ROOT_WORKTREE_PATH`); they cannot export into the agent; no
  teardown hook.

## Codex CLI (rust-v0.164.0)

- Docs now live at learn.chatgpt.com/docs/* (hooks, config-file/*,
  non-interactive-mode, agent-approvals-security, environments/*).
- Hooks: `SessionStart` + `Stop` since 0.114.0 (flag), stable with inline
  `[hooks]` in 0.124.0, `SessionEnd` in 0.145.0, `updatedInput` on
  `PreToolUse` in 0.131.0. Config: `~/.codex/hooks.json`, `~/.codex/config.toml`
  `[hooks]`, `<repo>/.codex/hooks.json`, `<repo>/.codex/config.toml`.
  Project hooks load only for trusted projects and must be trust-pinned
  once (`/hooks`, or `--dangerously-bypass-hook-trust`). Same JSON shape as
  Claude Code (`hooks.<Event>[] = {matcher, hooks:[{type:"command", command,
  timeout, async, statusMessage}]}`).
- Stdin: `SessionStart` = `{session_id, cwd, hook_event_name, model,
  permission_mode, source, transcript_path}`; `SessionEnd` =
  `{session_id, cwd, hook_event_name, reason:"other", transcript_path}`;
  fires on close, archive or 30 min idle, default timeout 1 s, max 3 s, not
  for subagents.
- Stdout: `{continue, stopReason, systemMessage, suppressOutput,
  hookSpecificOutput:{hookEventName, additionalContext}}`; `PreToolUse`
  adds `permissionDecision`, `permissionDecisionReason`, `updatedInput`.
  **No `env` field anywhere**: hooks cannot set session environment.
- Shell command env: `CODEX_THREAD_ID` (0.95.0+), `CODEX_SESSION_ID`
  (0.148.0+, shared with subagents), `CODEX_TOOL_CALL_ID`, `CODEX_VERSION`,
  `CODEX_SANDBOX=seatbelt` (macOS), `CODEX_SANDBOX_NETWORK_DISABLED=1` when
  network is off. These override `shell_environment_policy.set` entries of
  the same name. Not passed to stdio MCP servers (issue #19937).
- `shell_environment_policy`: `inherit = "all"` default, `set` literal
  strings only, `-c 'shell_environment_policy.set.DATABASE_URL="…"'`
  works through the generic dotted override [verify once by running].
- Sandbox: `workspace-write` has **no network, loopback included**
  (Seatbelt `(deny default)` with no network rules; Linux `bwrap
  --unshare-net` plus seccomp denying `connect`). Enable with
  `[sandbox_workspace_write] network_access = true` or
  `-c sandbox_workspace_write.network_access=true`. `codex exec` defaults to
  read-only. macOS can allowlist Unix sockets (`network.allow_unix_sockets`).
- `codex exec --json` emits `thread.started {thread_id}` first;
  `codex exec resume <id>`, `codex exec fork <id>`, `--ephemeral`,
  `--worktree` (managed worktrees, default on since 0.156.0).
- Profiles changed in 0.134.0: `--profile NAME` reads
  `$CODEX_HOME/NAME.config.toml`; `[profiles.X]` tables are no longer read.
  Precedence: CLI `-c` > project `.codex/config.toml` > profile > user >
  managed > `/etc/codex/config.toml`.
