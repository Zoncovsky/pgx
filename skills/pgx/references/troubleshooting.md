# Troubleshooting

Start with `pgx agent doctor` (project state, this session) and `pgx doctor`
(the runtime). Both print one line per check with a hint.

| Symptom | Cause | Fix |
|---|---|---|
| `PGX_ISOLATION=failed`, `DATABASE_URL` points at port 1 | The hook could not create a branch and failed closed. `PGX_ISOLATION_REASON` says why. | Fix the reason (below), then start a new session. Do not run database commands in this one. |
| `source "x" does not exist` | `.pgx.toml` names a template that is not in PGX (new machine, dropped). | `pgx copy --from … --to x --schema-only` (or `pgx create x`), or change `source`. |
| `source … has N active connections; a branch source must be idle` | Something connects to the template directly. | Disconnect it; keep a dedicated `<app>_template` nobody uses. |
| `PGX is not initialised` | PGX was never set up on this machine. | `pgx agent setup`. |
| Context line present but `echo $DATABASE_URL` is empty (Claude Code) | A Bash tool issue, or the env file was cleared by a `cd`-driven hook. | `pgx agent env` and `eval` it; new session. Check `~/.claude/session-env/<session>/`. |
| No context line at all | Hooks not installed, PATH problem in the hook, or an untrusted folder. | `pgx agent doctor` → `hooks:<host>`; `pgx agent install <host>` reports the self-test and the `pgx` it resolved. Trust the folder. |
| Hook reports `pgx not found on the hook's PATH` | Editor launched from the Dock without the shell's PATH, `pgx` in an unusual place. | Link it: `ln -s "$(command -v pgx)" ~/.local/bin/pgx`. |
| Two branches for one Cursor chat | Older PGX before the per-session lock, or two different project roots. | Upgrade; make sure `.pgx.toml` is at the repository root. |
| Branch disappeared mid-work | Idle TTL passed (no prompt or command for 2h) or the session ended more than 10 minutes ago. | The next session start makes a fresh one ("fresh" in the context line). Raise `idle_ttl` in `.pgx.toml` if needed. |
| `pgx copy` refuses an extension | The schema needs something PGX does not bundle. | See postgres-import.md; tell the user exactly what. |
| Codex commands cannot connect, `CODEX_SANDBOX_NETWORK_DISABLED=1` | Sandbox blocks loopback. | `sandbox_workspace_write.network_access = true` (user's decision). |
| Rails created `agent-…-0/1/2` databases | Parallel tests. | Harmless; `PARALLEL_WORKERS=1` or `pgx drop` them. |
| Port conflict on first run | Another service on the chosen port. | `pgx doctor`; `PGX_PORT=<free> pgx run` on a fresh home only; never `init --force`. |

Logs: `~/.pgx/log/` (runtime and supervisor). Records of agent branches:
`~/.pgx/ttl/*.json` (0600, no secrets).
