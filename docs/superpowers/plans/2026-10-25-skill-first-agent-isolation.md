# Skill-First Agent Database Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the public `pgx` Agent Skill plus the `pgx agent` lifecycle core so that one `npx skills add pgrundev/pgx --skill pgx` lets Claude Code, Cursor and Codex give every coding session its own isolated PGX branch.

**Architecture:** The Rust CLI gains one command group, `pgx agent`, that owns deterministic work: project-aware session identity, idempotent branch creation on top of `clone_database`, idle-TTL and grace handling in the existing TTL store, a host-agnostic `hook` entry point, a process wrapper, hook installation, `setup` and `doctor`. The Skill (`skills/pgx/`) owns judgement: when a task needs a database, project and framework discovery, choosing a safe template strategy, running `pgx agent setup`, and explaining failures. Hooks give lifecycle guarantees on Claude Code and Cursor; the wrapper covers Codex and any other tool.

**Tech Stack:** Rust (std + `libc` + `serde_json`, no new crates), POSIX sh for skill scripts, Node (`skills` CLI) and `skills-ref` (Python) for skill validation, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-25-agent-integration-design.md` (architecture, adapters, settled details) and `docs/superpowers/specs/2026-10-25-agent-integration-research.md` (exact host payloads). The product brief that re-prioritised the work skill-first is reproduced in the PR description.

## Global Constraints

- No new Cargo dependencies; the crate has `libc` and `serde_json` only.
- Database names satisfy `names::validate` (`^[A-Za-z_][A-Za-z0-9_-]{0,62}$`); never a reserved name.
- Exit codes keep their meaning: 0 ok, 1 failure, 2 usage, 3 not ready, 4 conflict, 5 not found, 6 in use, 7 corrupt. `pgx agent hook` never exits 2.
- Idle TTL default `2h`; grace default `10m`; `touch` writes at most once per 30 s.
- Secrets never reach stdout in hook mode, context lines, logs, `.pgx.toml`, or test output; the password-bearing URL goes only into environment variables and `--json`/`--env` output that the caller asked for.
- Hook installers merge JSON and never remove or reorder entries they did not write; the user's existing `~/.claude/settings.json` and `~/.cursor/hooks.json` hooks (an "orca" tool) must survive every install/uninstall test untouched.
- SKILL.md frontmatter uses only `name`, `description`, `license`, `metadata`, `allowed-tools`, `compatibility`; `name: pgx` equals the directory name; body under 500 lines.
- Supported platforms: macOS arm64, Linux amd64, Linux arm64. Nothing claims Windows or darwin-amd64.
- Nothing deletes a developer's database, PGX home, worktree or scratch data; tests use `Pgx::new()` homes under the scratch root.
- CI (`.github/workflows/ci.yml`) toolchain is Rust 1.96; `cargo fmt --check` and `cargo clippy -- -D warnings` must pass.

## Review Focus

1. Two Cursor hook deliveries for one `sessionStart` (native file plus Claude-compat loader) must yield one branch, not two: covered by the lock test in Task 4 and the duplicate-payload test in Task 6.
2. A `.pgx.toml` whose `source` is missing from PGX, with hooks installed, must fail closed: the session's `DATABASE_URL` is a sentinel that cannot connect, and the context line says why (Task 6).
3. A session id containing characters outside `[a-z0-9-]` (hand-chosen `exec --id`, or a hostile payload) must produce a valid, non-colliding name and never reach SQL unquoted (Task 3).
4. `pgx ttl agent-… 4h` on an agent branch must keep its `agent` object, so `down` still recognises it (Task 2).
5. `exec` killed by SIGTERM while its child runs must forward the signal, wait, retire the branch, and exit 143 (Task 7).

---

### Task 1: `pgx agent` argument parsing and dispatch skeleton

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/mod.rs`
- Modify: `crates/bin/pgx/src/cli.rs` (parse: route `"agent"` to the new parser; stop treating `--json` as global after `--`), `crates/bin/pgx/src/cmd/mod.rs` (`pub mod agent;`), `crates/bin/pgx/src/main.rs` (dispatch `Command::Agent(cmd)`)
- Test: `crates/bin/pgx/src/cli.rs` unit tests, `crates/bin/pgx/src/cmd/agent/mod.rs` unit tests

**Interfaces:**
- Produces: `pub enum AgentCmd { Up(UpOpts), Touch(IdOpts), Down(DownOpts), Exec(ExecOpts), Env(IdOpts), Ls, Hook(HookOpts), Install(InstallOpts), Uninstall(InstallOpts), Setup(SetupOpts), Doctor }` and `pub fn parse(tail: &[&str]) -> Result<AgentCmd>`; `Command::Agent(AgentCmd)` in `cli.rs`; `pub fn run(out: &Out, cmd: AgentCmd) -> Result<ExitCode>` in `cmd/agent/mod.rs`.
- Option structs: `IdOpts { id: Option<String>, host: Option<String> }`; `UpOpts { id: Option<String>, host: Option<String>, source: Option<String>, ttl: Option<String>, env: bool, cwd: Option<PathBuf> }`; `DownOpts { id, host, now: bool, grace: Option<String> }`; `ExecOpts { id, host, source, ttl, keep: bool, command: Vec<String> }`; `HookOpts { host: Option<String>, source, ttl, grace }`; `InstallOpts { hosts: Vec<String>, scope: Scope /* Project|Local|User */, source: Option<String>, ttl: Option<String> }`; `SetupOpts { source: Option<String>, hosts: Vec<String>, scope: Scope, ttl: Option<String>, no_hooks: bool }`.
- `--` ends option parsing for `exec`; everything after it is `command`.

- [ ] **Step 1: Write failing parser tests in `cli.rs`**

```rust
#[test]
fn agent_exec_keeps_everything_after_double_dash() {
    let (g, c) = parse(&args(&["agent", "exec", "--source", "tpl", "--", "codex", "--json", "x"])).unwrap();
    assert!(!g.json);
    match c { Command::Agent(AgentCmd::Exec(o)) => { assert_eq!(o.source.as_deref(), Some("tpl")); assert_eq!(o.command, vec!["codex", "--json", "x"]); } other => panic!("{other:?}") }
}
#[test]
fn agent_requires_a_subcommand_and_rejects_unknown_ones() {
    assert_eq!(parse(&args(&["agent"])).unwrap_err().code, ExitCode::Usage);
    assert_eq!(parse(&args(&["agent", "frobnicate"])).unwrap_err().code, ExitCode::Usage);
    assert_eq!(parse(&args(&["agent", "exec"])).unwrap_err().code, ExitCode::Usage); // no command
}
#[test]
fn help_mentions_agent() { assert!(help_text().contains("pgx agent setup")); }
```

- [ ] **Step 2: Run them, expect compile failure on `Command::Agent`**

Run: `cargo test -p pgx --bin pgx cli::tests::agent_ -- --nocapture`

- [ ] **Step 3: Implement the parser**

In `cli.rs::parse`, before the generic flag loop: if `name == "agent"`, return `Ok((global, Command::Agent(crate::cmd::agent::parse(tail)?)))` where `tail` still contains `--json`-free tokens except after `--`. Change the global loop to stop consuming `--json`/`-h`/`-V` once it has seen `--` (push `--` and everything after it verbatim). In `cmd/agent/mod.rs` write `parse` with its own small flag loop: value flags `id host source ttl grace hosts scope`; boolean flags `env now keep no-hooks json? (no)`; usage strings per subcommand exactly as the help block in Step 4.

- [ ] **Step 4: Add the help block to `help_text()`**

```
Agents (one isolated database per coding-agent session)
  pgx agent setup [--source <db>] [--hosts claude,cursor,codex] [--scope project|local|user]
                          one-time project setup: writes .pgx.toml, installs lifecycle
                          hooks for the detected hosts, verifies them
  pgx agent doctor        check hooks, .pgx.toml, the source template, the runtime
  pgx agent exec [--source <db>] [--ttl <dur>] [--keep] -- <command> [args…]
                          run any command (codex, tests, a script) with its own branch
                          in DATABASE_URL; retire the branch when it exits
  pgx agent ls            agent branches: host, session, source, expires-in
  pgx agent up|touch|down|env|hook|install|uninstall   lifecycle primitives (see docs/pgx/local-dev/agents.md)
```

- [ ] **Step 5: Dispatch in `main.rs`: `Command::Agent(cmd) => cmd::agent::run(&out, cmd)` inside the post-init arm; `run` returns `Err(PgxError::failure("not implemented"))` for each variant for now.**

- [ ] **Step 6: Run `cargo test -p pgx --bin pgx` and `cargo clippy -p pgx -- -D warnings`; expect pass.**

- [ ] **Step 7: Commit** `feat(pgx): parse the pgx agent command group`

---

### Task 2: Agent metadata in the TTL record

**Files:**
- Modify: `crates/bin/pgx/src/ttlstore.rs` (add `AgentMeta`, `agent: Option<AgentMeta>` on `TtlRecord`, JSON round trip), `crates/bin/pgx/src/cmd/ttl.rs` (preserve `agent` when a TTL is changed), `crates/bin/pgx/src/cmd/db.rs` (`record_ttl` builds records with `agent: None`), `crates/bin/pgx/src/cmd/supervise.rs` (no logic change; records with `agent` expire like any other)
- Test: unit tests in `ttlstore.rs`; black-box in `crates/bin/pgx/tests/ttl.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct AgentMeta { pub host: String, pub session_id: String, pub project_id: String, pub project_root: String, pub source: String, pub last_touch: u64, pub retired: bool }
  pub struct TtlRecord { /* existing */ pub agent: Option<AgentMeta> }
  ```
  Serialised under key `"agent"` with those field names; schema stays `1`; a record without the key loads with `agent: None`.

- [ ] **Step 1: Failing unit test**

```rust
#[test]
fn agent_metadata_round_trips_and_is_optional() {
    let mut r = rec("agent-claude-9b1f3c2d", Some(1_007_200));
    r.agent = Some(AgentMeta { host: "claude".into(), session_id: "9b1f3c2d-0000".into(), project_id: "a1b2c3d4e5f60718".into(), project_root: "/p".into(), source: "myapp".into(), last_touch: 1_000_000, retired: false });
    assert_eq!(TtlRecord::from_json(&r.to_json()).unwrap(), r);
    assert!(TtlRecord::from_json(&rec("x", Some(1)).to_json()).unwrap().agent.is_none());
}
```

- [ ] **Step 2: Run `cargo test -p pgx --bin pgx ttlstore` — expect compile error.**
- [ ] **Step 3: Implement the struct, `to_json`/`from_json` (missing or malformed `agent` object → `None`, never an error), update every `TtlRecord { … }` literal (`db.rs`, `ttl.rs`, tests) with `agent: None`.**
- [ ] **Step 4: In `cmd/ttl.rs`, when setting a finite TTL load the existing record first and copy its `agent` into the new one; write the black-box test `ttl_change_keeps_agent_metadata` in `tests/ttl.rs` (create a record through `pgx agent up` once Task 4 exists; until then, drive it by writing a record file with `agent` set through the store in a unit test in `cmd/ttl.rs` is not possible, so leave a `// TODO(task 4)` marker and add the black-box test in Task 4 Step 6).**
- [ ] **Step 5: Run `cargo test -p pgx --bin pgx` — pass. Commit** `feat(pgx): optional agent metadata on TTL records`

---

### Task 3: Session identity, naming and project configuration

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/identity.rs`, `crates/bin/pgx/src/cmd/agent/project.rs`
- Test: unit tests in both files

**Interfaces:**
- `identity.rs`:
  ```rust
  pub fn fnv1a64(bytes: &[u8]) -> u64;                       // offset 0xcbf29ce484222325, prime 0x100000001b3
  pub fn hex16(v: u64) -> String;
  pub struct SessionKey { pub host: String, pub session_id: String, pub project_id: String }
  pub fn validate_host(s: &str) -> Result<String>;          // ^[a-z][a-z0-9]{0,15}$ else usage error
  pub fn validate_session_id(s: &str) -> Result<String>;    // 1..=128 bytes, printable ASCII, no whitespace or '/' else usage error
  pub fn branch_name(key: &SessionKey, long: bool) -> String;  // "agent-{host}-{sid8}" or with 16 hex of fnv1a64(project_id|host|session_id)
  pub fn session_env_var(host: &str) -> Option<&'static str>; // claude→CLAUDE_CODE_SESSION_ID, codex→CODEX_SESSION_ID, cursor→None
  ```
  `sid8`: first 8 chars of `session_id` lowercased when all of them are in `[a-z0-9]`, else the first 8 hex of the FNV hash.
- `project.rs`:
  ```rust
  pub struct ProjectConfig { pub root: PathBuf, pub project_id: String, pub source: Option<String>, pub idle_ttl: Option<String>, pub grace_period: Option<String>, pub config_file: Option<PathBuf> }
  pub fn locate(start: &Path, fallbacks: &[PathBuf]) -> Result<ProjectConfig>;  // walk up for .pgx.toml, then .git (dir or file); else start; project_id = hex16(fnv1a64(canonical root))
  pub fn parse_pgx_toml(text: &str) -> Result<(Option<String>, Option<String>, Option<String>)>; // [agent] source / idle_ttl / grace_period; quoted strings only; unknown keys ignored; anything that is not a bare key = "value" line inside [agent] is ignored; a value containing "://" is rejected with a usage error ("connection URLs do not belong in .pgx.toml")
  pub fn render_pgx_toml(source: &str, idle_ttl: &str, grace: &str) -> String;  // exactly the format in the spec, with comments
  pub const DEFAULT_SOURCE: &str = "app"; pub const DEFAULT_IDLE_TTL: &str = "2h"; pub const DEFAULT_GRACE: &str = "10m";
  ```

- [ ] **Step 1: Failing tests**

```rust
#[test] fn fnv_matches_reference() { assert_eq!(fnv1a64(b""), 0xcbf29ce484222325); assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c); }
#[test] fn uuid_session_gives_readable_name() { let k = key("claude", "9B1F3C2D-aaaa-bbbb"); assert_eq!(branch_name(&k, false), "agent-claude-9b1f3c2d"); }
#[test] fn odd_session_ids_hash() { let k = key("exec", "my task!"); assert!(branch_name(&k, false).starts_with("agent-exec-")); assert!(crate::names::validate(&branch_name(&k, true)).is_ok()); }
#[test] fn hosts_and_ids_are_validated() { assert!(validate_host("Claude").is_err()); assert!(validate_session_id("a b").is_err()); assert!(validate_session_id(&"x".repeat(129)).is_err()); }
#[test] fn toml_subset() { let (s, t, g) = parse_pgx_toml("# c\n[agent]\nsource = \"myapp_template\"\nidle_ttl = \"3h\"\n").unwrap(); assert_eq!(s.as_deref(), Some("myapp_template")); assert_eq!(t.as_deref(), Some("3h")); assert!(g.is_none()); assert!(parse_pgx_toml("[agent]\nsource = \"postgres://u:p@h/db\"\n").is_err()); }
#[test] fn locate_prefers_pgx_toml_then_git() { /* tempdir: root/.git dir, root/sub/.pgx.toml, start root/sub/deep → root/sub; without the toml → root */ }
```

- [ ] **Step 2: Run, expect compile errors. Step 3: Implement. Step 4: Run `cargo test -p pgx --bin pgx agent::` — pass.**
- [ ] **Step 5: Commit** `feat(pgx): agent session identity and .pgx.toml`

---

### Task 4: `pgx agent up`, `touch`, `down`, `env`, `ls`

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/session.rs`
- Modify: `crates/bin/pgx/src/cmd/db.rs` (make `clone_database` and `exists` `pub(crate)`; add `pub fn active_connections(client, name) -> Result<u64>`), `crates/bin/pgx/src/cmd/agent/mod.rs` (dispatch)
- Test: `crates/bin/pgx/tests/agent.rs` (new; add to CI in Task 12)

**Interfaces:**
- ```rust
  pub struct Resolved { pub key: SessionKey, pub project: ProjectConfig, pub source: DbName, pub idle_ttl: Ttl /* finite */, pub grace: Ttl }
  pub fn resolve(opts: &UpOpts /* or IdOpts fields */, env: &HashMap<String,String>) -> Result<Resolved>;
      // host: --host > PGX_AGENT_HOST > "exec"; id: --id > the host's session env var > usage error "no session id: pass --id or run inside a supported agent"
      // source: --source > PGX_SOURCE > .pgx.toml > "app"; ttl: --ttl > .pgx.toml idle_ttl > "2h"; a ttl of unlimited → usage error "agent branches always expire; use pgx branch for a permanent copy"
  pub struct UpResult { pub branch: DbName, pub url: String, pub socket_url: String, pub created: bool, pub source: String, pub source_created: bool, pub expires_at: u64, pub idle_ttl: Ttl, pub key: SessionKey, pub fresh: bool }
  pub fn up(out: &Out, r: &Resolved) -> Result<UpResult>;
  pub fn touch(r: &Resolved) -> Result<Option<u64>>;        // new expires_at, None when skipped (<30 s since last_touch) or no record
  pub fn down(r: &Resolved, now: bool) -> Result<DownResult>; // DownResult { branch: Option<String>, dropped: bool, retired_until: Option<u64> }
  pub fn env_lines(res: &UpResult) -> String;                 // export DATABASE_URL, PGX_BRANCH, PGX_SOURCE, PGHOST, PGPORT, PGUSER, PGPASSWORD, PGDATABASE (single-quoted, ' escaped as '\'' )
  pub fn sentinel_env_lines(reason: &str) -> String;         // DATABASE_URL='postgres://pgx-isolation-failed@127.0.0.1:1/pgx_isolation_failed', PGX_ISOLATION='failed', PGX_ISOLATION_REASON='…', PGHOST=127.0.0.1 PGPORT=1 PGDATABASE=pgx_isolation_failed
  pub fn find_record(store: &MetaStore, key: &SessionKey) -> Result<Option<TtlRecord>>;  // by scanning records whose agent matches the key
  pub fn lock_session(home: &Home, key: &SessionKey) -> Result<File>;  // flock on $PGX_HOME/agent/locks/<hex16>.lock, created 0700/0600
  ```
- Behaviour of `up`: lock; `Ctx::load` (not initialised → exit 3, hint `pgx run`); `ensure_running` (quiet); `spawn_supervisor`; find record by key → if found and database exists with that OID: set `retired=false`, extend `expires_at = now + idle_ttl`, `last_touch = now`, return `created:false`; if found but database gone → fall through with `fresh: true`; else choose `branch_name(short)`; if a database with that name exists and no record with our key owns it → `branch_name(long)`; source missing: if source == "app" and no explicit `--source`/toml: `CREATE DATABASE app` (`source_created: true`), else exit 5 with hint `pgx copy --from "$DATABASE_URL" --to {source} --schema-only`; `active_connections(source) > 0` → exit 6 with the spec's message; `clone_database(ctx, client, &source, &branch, Some(ttl_label))`; then load the record just written by `record_ttl` and save it with `agent: Some(meta)`; on 42P04 during the clone (a racing twin) re-run the lookup once and return its result.
- `down`: find record; none → `Ok(DownResult{branch:None,..})`; `now` → terminate sessions on that database only, `DROP DATABASE IF EXISTS … WITH (FORCE)`, remove record; else `retired=true`, `expires_at = now + grace`, save.
- `ls`: table `NAME HOST SESSION SOURCE PROJECT AGE EXPIRES-IN STATE` (STATE active/retired/expired) and `--json` array of the full records minus nothing (no secrets live there).

- [ ] **Step 1: Failing black-box tests in `tests/agent.rs`** (helpers: `up_with_source()` as in `tests/branch.rs` creating `myapp`; `agent(&p, args, envs)` running `pgx --json agent …` with `PGX_SOURCE`, `CLAUDE_CODE_SESSION_ID` etc.)

```rust
#[test] fn up_is_idempotent_per_session_and_isolated_across_sessions()   // two ups same id → same branch, second created:false; different id → different branch; row inserted in A invisible in B; myapp fingerprint unchanged
#[test] fn up_without_source_uses_app_and_creates_it_empty()             // fresh runtime, no myapp: branch of app, source_created:true, json.source == "app"
#[test] fn up_refuses_a_busy_source_with_exit_6()                        // hold a psql session open on myapp (spawn psql reading stdin) → exit 6, message contains "must be idle"
#[test] fn touch_extends_and_rate_limits()                              // up with --ttl 90s; sleep 2; touch → expires_at moved (>= first + 1); immediate second touch → json.skipped == true
#[test] fn down_retires_with_grace_then_up_revives()                    // down → json.retired_until ≈ now+600, pgx ls shows the db still there; up → same branch, created:false; down --now → dropped:true and db gone
#[test] fn expired_agent_branch_is_removed_by_the_supervisor()          // --ttl 3s, wait ≤ 20s → gone, source intact (mirror tests/branch.rs)
#[test] fn env_prints_exports_for_the_current_session()                 // CLAUDE_CODE_SESSION_ID set, no --id: `pgx agent env` → lines start with export DATABASE_URL=' and PGDATABASE='agent-claude-
#[test] fn ttl_change_keeps_agent_metadata()                            // pgx ttl <branch> 4h → pgx --json agent ls still lists it with host claude (Review Focus 4)
#[test] fn concurrent_ups_for_one_session_make_one_branch()             // spawn 6 `pgx agent up` with the same id simultaneously → all exit 0, one distinct branch, exactly one created:true (Review Focus 1)
#[test] fn ls_lists_only_agent_branches()                               // pgx create plain --ttl 1h; agent ls json has no "plain"
```

- [ ] **Step 2: Run `cargo test -p pgx --release --test agent` — expect failures ("not implemented").**
- [ ] **Step 3: Implement `session.rs` and wire `AgentCmd::{Up,Touch,Down,Env,Ls}` in `mod.rs`.** Human output of `up`: the `pgx branch` block with `✓ Branch ready:`/`✓ Branch created:`, `✓ Source:`, `✓ Expires in: 2 hours after the last activity`, then `DATABASE_URL:` and the URL; `--env` prints only `env_lines`. JSON of `up`: `{branch, source, source_created, created, fresh, url, socket_url, host, session_id, project_id, project_root, idle_ttl, expires_at, expires_in_seconds, method}`.
- [ ] **Step 4: Run the suite — pass. Step 5: `cargo fmt`, `cargo clippy -p pgx -- -D warnings`.**
- [ ] **Step 6: Commit** `feat(pgx): agent up/touch/down/env/ls — one branch per coding-agent session`

---

### Task 5: Host payload parsing

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/hosts.rs`
- Test: unit tests in the file using the exact payloads from the research appendix

**Interfaces:**
- ```rust
  pub enum Host { Claude, Cursor, Codex }
  pub enum Event { Start, Activity, End, Ignore }
  pub struct HookInput { pub host: Host, pub event: Event, pub session_id: String, pub cwd: Option<PathBuf>, pub project_dir: Option<PathBuf>, pub reason: Option<String>, pub background: bool, pub raw_event: String }
  pub fn detect(payload: &serde_json::Value, env: &HashMap<String,String>, forced: Option<&str>) -> Result<HookInput>;
  ```
  Detection order: `forced`; env `CLAUDE_ENV_FILE` or `CLAUDE_CODE_SESSION_ID` → Claude; env `CURSOR_VERSION` or payload `conversation_id` → Cursor; env `CODEX_THREAD_ID`/`CODEX_SESSION_ID` or (`permission_mode` present and no Claude env) → Codex; a PascalCase `hook_event_name` with `session_id` and `transcript_path` but none of the above → Claude. Event mapping exactly as the spec's hook table (Claude: SessionStart→Start, UserPromptSubmit/PreToolUse/PostToolUse→Activity, SessionEnd→End; Cursor: sessionStart→Start, beforeSubmitPrompt/afterShellExecution/beforeShellExecution/preToolUse/postToolUse→Activity, sessionEnd→End, anything else→Ignore; Codex: SessionStart→Start, UserPromptSubmit/PreToolUse/PostToolUse→Activity, SessionEnd→End). Cursor `is_background_agent: true` → `background: true`. `cwd` from payload `cwd`, else `workspace_roots[0]`; `project_dir` from env `CLAUDE_PROJECT_DIR`/`CURSOR_PROJECT_DIR`. A payload without a usable session id → `Err(usage)`.

- [ ] **Step 1: Failing tests, one per row of the table plus `cursor_payload_delivered_through_claude_compat_is_detected_as_cursor` (Cursor-shaped payload with `CURSOR_VERSION` set and `hook_event_name: "sessionStart"` while `CLAUDE_PROJECT_DIR` is also set) and `hostile_session_id_is_rejected` (`"session_id": "x; rm -rf /"`).**
- [ ] **Step 2–4: run, implement, pass. Step 5: Commit** `feat(pgx): parse Claude Code, Cursor and Codex hook payloads`

---

### Task 6: `pgx agent hook`

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/hook.rs`
- Modify: `crates/bin/pgx/src/cmd/agent/mod.rs`
- Test: `crates/bin/pgx/tests/agent_hook.rs` (new)

**Interfaces:**
- `pub fn run(out: &Out, opts: HookOpts) -> Result<ExitCode>`: read stdin fully (empty stdin → exit 0 silently), `detect`, then:
  - `Start`: `resolve` with `cwd`/`project_dir` fallbacks and `up`. Claude: append `env_lines` to `$CLAUDE_ENV_FILE` (create if missing; if the variable is unset print the lines to stderr with a notice and still exit 0), print the context sentence on stdout. Cursor: print `{"env": {...9 vars...}, "additional_context": "<sentence>"}`. Codex: print `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"<sentence>"}}`.
  - On failure of `up` with isolation enabled (a `.pgx.toml` was found, or `PGX_SOURCE` set): Claude → append `sentinel_env_lines(reason)` and print `PGX isolation FAILED for this session: <reason>. DATABASE_URL now points at an unreachable sentinel so nothing runs against another database. Fix it (pgx agent doctor) and start a new session.`; Cursor → same in `env` + `additional_context`; Codex → context only. Exit 1 with the reason on stderr. Without isolation enabled (no `.pgx.toml`): stderr notice, exit 1, no env written.
  - `Activity`: `touch`; print `{}` for Cursor, nothing otherwise; exit 0 even when no record.
  - `End`: `down(now=false)`; exit 0.
  - `Ignore`/`background`: print `{}` for Cursor, exit 0.
- Context sentence (exact): `PGX: this session's database is branch {branch} (a copy of {source}{, created empty because no .pgx.toml names a source|, fresh}). DATABASE_URL and PG* are already set in your shell; do not point them elsewhere. The branch is disposable and expires {idle_ttl} after your last activity.` For Codex append ` Shell commands do not inherit it from this hook: run database commands as `pgx agent exec -- <command>`, or start Codex as `pgx agent exec -- codex`.` When `CODEX_SANDBOX_NETWORK_DISABLED=1` is in the hook's environment append ` Codex's sandbox blocks loopback TCP; set sandbox_workspace_write.network_access = true for this project.`

- [ ] **Step 1: Failing tests** (drive the real binary with payloads on stdin and `CLAUDE_ENV_FILE` pointing into the scratch dir):

```rust
#[test] fn claude_session_start_writes_env_file_and_context()   // file contains export DATABASE_URL='postgres://…/agent-claude-…'; stdout starts with "PGX: this session's database is branch agent-claude-"; stdout does not contain the password
#[test] fn claude_prompt_and_tool_events_touch_quietly()          // stdout empty, exit 0, expires_at moved
#[test] fn claude_session_end_retires_with_grace()                // ls shows retired, expires in ≤ 600
#[test] fn cursor_session_start_returns_env_json_and_background_is_skipped()
#[test] fn cursor_same_conversation_delivered_twice_makes_one_branch()   // native + compat delivery (Review Focus 1)
#[test] fn codex_session_start_returns_additional_context_with_exec_advice()
#[test] fn missing_source_with_pgx_toml_fails_closed()           // .pgx.toml source="nope": exit 1, env file has PGX_ISOLATION='failed' and DATABASE_URL on port 1; psql against it fails (Review Focus 2)
#[test] fn hook_never_exits_2_and_tolerates_garbage()             // stdin "not json" → exit 0 or 1, never 2; empty stdin → 0
#[test] fn hook_ignores_cursor_events_it_does_not_handle()       // afterFileEdit → "{}"
```

- [ ] **Step 2–4: run, implement, pass. Step 5: Commit** `feat(pgx): pgx agent hook — one entry point for every host`

---

### Task 7: `pgx agent exec`

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/exec.rs`
- Test: `crates/bin/pgx/tests/agent_exec.rs` (new)

**Interfaces:**
- `pub fn run(out: &Out, opts: ExecOpts) -> Result<ExitCode>`; attach mode when `--id` is absent and a host session var is in the environment (`CLAUDE_CODE_SESSION_ID` → host claude, `CODEX_SESSION_ID` → codex): reuse that session's branch and do **not** retire it on exit; otherwise fresh mode: `session_id = <16 hex random from /dev/urandom>`, host = `--host` or the command's basename sanitised through `validate_host` (fallback `exec`), retire on exit (`down --now`, or grace with `--keep`).
- Child env: inherited + the 9 variables + `PGX_AGENT_SESSION=<session_id>`; signals SIGINT/SIGTERM/SIGHUP forwarded with `libc::kill(child_pid, sig)` from a handler that sets an atomic (reuse the pattern in `pgtools.rs`); heartbeat thread calls `touch` every `max(ttl/4, 30s)`; exit status = child's code, or `128 + signal`; `--json` prints `{branch, session_id, host, exit_code, retired}` on stderr? No: stdout is the child's; `--json` summary goes to stdout only after the child exits.
- The branch URL is never printed in human mode.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn exec_gives_the_child_its_own_branch_and_retires_it()        // sh -c 'psql "$DATABASE_URL" -c "create table t(x int)"; echo $PGX_BRANCH' → exit 0; branch gone afterwards; source unchanged
#[test] fn exec_propagates_exit_code_and_still_cleans_up()             // sh -c 'exit 7' → 7, branch gone
#[test] fn exec_forwards_sigterm_and_exits_143()                      // child `sleep 30`; kill -TERM the pgx process; wait → status 143, branch gone (Review Focus 5)
#[test] fn exec_attaches_to_the_surrounding_session()                  // CLAUDE_CODE_SESSION_ID set: two sequential execs share one branch; branch survives
#[test] fn exec_keep_retires_with_grace()                               // --keep: branch still listed, retired
#[test] fn exec_two_parallel_wrappers_are_isolated()                    // Test D in miniature
```

- [ ] **Step 2–4: run, implement, pass. Step 5: Commit** `feat(pgx): pgx agent exec — a branch for any command's lifetime`

---

### Task 8: `install`, `uninstall`, `setup`, `doctor`

**Files:**
- Create: `crates/bin/pgx/src/cmd/agent/install.rs`, `crates/bin/pgx/src/cmd/agent/doctor.rs`
- Test: `crates/bin/pgx/tests/agent_install.rs` (new), unit tests for the JSON merge

**Interfaces:**
- `pub const HOOK_COMMAND: &str = "/bin/sh -c 'PATH=\"$PATH:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin\"; command -v pgx >/dev/null 2>&1 && exec pgx agent hook; exit 0'";` and `pub const MARKER: &str = "pgx agent hook";`
- `pub fn install(out, opts: InstallOpts, cwd) -> Result<Report>` where hosts default to the detected set (`~/.claude` or `.claude` → claude; `~/.cursor` or `.cursor` → cursor; `~/.codex` or `.codex` → codex), writing:
  - claude: `.claude/settings.json` (project) / `.claude/settings.local.json` (local) / `~/.claude/settings.json` (user): `hooks.SessionStart[]` (no matcher, timeout 30), `hooks.UserPromptSubmit[]` (timeout 5), `hooks.PreToolUse[]` matcher `Bash` (timeout 5), `hooks.SessionEnd[]` (timeout 10), each `{"hooks":[{"type":"command","command":HOOK_COMMAND,"timeout":N}]}`.
  - cursor: `.cursor/hooks.json` / `~/.cursor/hooks.json`: `version: 1`, `hooks.sessionStart[]`, `beforeSubmitPrompt[]`, `afterShellExecution[]`, `sessionEnd[]`, each `{"command":HOOK_COMMAND,"timeout":N}`.
  - codex: `.codex/hooks.json` / `~/.codex/hooks.json`: same shape as Claude for `SessionStart`, `UserPromptSubmit`, `SessionEnd`.
  - Merge rule: parse (invalid JSON → exit 1 "will not touch"), for each event append the entry when no existing entry's command contains `MARKER`; write pretty JSON atomically (tmp + rename); report `added`/`present` per file.
  - `.pgx.toml`: written at the project root when absent with `render_pgx_toml(source, idle_ttl, grace)`; when present and `--source` differs → exit 4 with hint `edit .pgx.toml`.
  - Self-test: run `HOOK_COMMAND` through `/bin/sh -c` with `PGX_AGENT_SELFTEST=1` and a synthetic Claude `SessionStart` payload for session id `selftest` and `CLAUDE_ENV_FILE` in a temp file; `hook` in self-test mode resolves everything but creates nothing and prints `pgx <version> at <path>`; report the path or "pgx not found on the hook's PATH".
- `uninstall`: remove entries whose command contains `MARKER`; leave files otherwise byte-identical in content; do not delete `.pgx.toml`.
- `setup(SetupOpts)`: `Ctx`/`ensure_running` (initialise through `cmd::run` if needed), resolve or create the source (`--source` or `.pgx.toml` or `app`), verify it exists and is idle, write `.pgx.toml`, `install` for the hosts, run one real `up`+`down --now` with session id `setup-check` to prove branching works, print the summary block:
  ```
  PGX agent setup complete.
    source     myapp_template (12 MB, idle)
    config     .pgx.toml (written)
    hooks      claude-code  .claude/settings.json  (4 added)
               cursor       .cursor/hooks.json     (4 added)
    runtime    running on port 5433
    check      branch created and removed in 38 ms
  Next: start a new agent session; it gets its own branch automatically.
  ```
  and `--json` with the same facts.
- `doctor`: checks `install`, `runtime`, `supervisor`, `.pgx.toml`, `source` (exists, idle, size), `hooks:<host>` per host (present/missing/partial at project, local, user scopes; duplicate entries flagged), `session` (if `CLAUDE_CODE_SESSION_ID`/`CODEX_SESSION_ID` present: whether a branch exists and whether `DATABASE_URL` in the environment names it), `branches` (count, retired, expired-awaiting-cleanup). Exit 0 when nothing failed, 1 otherwise; `--json` array like `pgx doctor`.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn install_merges_and_preserves_foreign_hooks()        // pre-seed .claude/settings.json with an orca-like SessionStart hook and env {"FOO":"1"}; install; both remain; our 4 events present; running twice adds nothing
#[test] fn install_cursor_and_codex_files()
#[test] fn install_refuses_invalid_json()
#[test] fn uninstall_removes_only_ours()
#[test] fn setup_writes_toml_installs_hooks_and_verifies_a_branch()   // json.check.elapsed_ms > 0; no agent branch left; .pgx.toml content == render_pgx_toml("myapp","2h","10m")
#[test] fn doctor_reports_missing_hooks_and_busy_source()
#[test] fn hook_command_self_test_resolves_pgx_on_the_installed_path()
```

- [ ] **Step 2–4: run, implement, pass. Step 5: Commit** `feat(pgx): pgx agent setup/install/uninstall/doctor`

---

### Task 9: `pgx doctor` and `pgx ls` awareness, help, CLI reference

**Files:**
- Modify: `crates/bin/pgx/src/cmd/doctor.rs` (one check `agents`: count of agent branches and hint `pgx agent doctor`), `crates/bin/pgx/src/cmd/db.rs` (`ls` shows `agent` host in the TTL column as `2h idle (claude)` and `--json` includes the `agent` object), `docs/pgx/local-dev/cli.md` (Agents section), `docs/pgx/local-dev/ttl.md` (idle TTL paragraph)
- Test: extend `tests/doctor.rs` and `tests/agent.rs` (`ls_json_includes_agent_object`)

- [ ] Steps: failing test → implement → pass → commit `feat(pgx): surface agent branches in ls and doctor; document the agent CLI`

---

### Task 10: The `pgx` Agent Skill

**Files:**
- Create: `skills/pgx/SKILL.md`, `skills/pgx/references/{getting-started,installation,database-discovery,postgres-import,database-branching,agent-lifecycle,claude-code,cursor,codex,troubleshooting,security}.md`, `skills/pgx/scripts/detect-project.sh`, `skills/pgx/scripts/README.md`
- Test: `skills/pgx/tests/run.sh` (validator + script fixtures), fixtures under `skills/pgx/tests/fixtures/{rails,django,prisma,node,go,empty}/`

**Interfaces:**
- `detect-project.sh [dir]` prints one JSON object: `{"framework":"rails|django|prisma|node-pg|sqlalchemy|go|unknown","signals":[…files…],"database_config":{"files":[…],"url_sources":["env:DATABASE_URL","file:.env"],"classification":"local|pgx|none|remote|sensitive|unknown","host":"<host only, no credentials>","db":"<name>"},"migrations":"rails|django|prisma|alembic|golang-migrate|unknown","pgx":{"installed":true,"version":"0.1.0","toml":true,"hooks":{"claude":true,"cursor":false,"codex":false}}}`; never prints a password or a full URL; classification `sensitive` when host matches `prod|production|rds.amazonaws|neon.tech|supabase|render|railway|heroku|fly.io|cloud` or `sslmode=require` with a non-local host; `remote` for any non-local host; `local` for localhost/127.0.0.1/::1/socket; `pgx` when the port equals the PGX runtime port and host is loopback.
- SKILL.md frontmatter: `name: pgx`; `description` (≤1024 chars) built from the brief's trigger phrases; `license: AGPL-3.0`; `metadata: {author: pgrundev, version: "0.1.0", homepage: "https://pgx.dev"}`.
- SKILL.md body (<300 lines): the decision flow from the brief (detect → install → runtime → template → `pgx agent setup` → work → cleanup), the three hard rules (never import production data without explicit approval; schema-only by default; fail closed), the per-host activation table (Claude Code: hooks automatic after setup; Cursor: hooks, verified status from Task 14; Codex: `pgx agent exec -- codex` or `pgx agent exec -- <command>`), and "when to read which reference".

- [ ] **Step 1: Write `tests/run.sh` first:** runs `pipx run skills-ref validate skills/pgx` (or `uvx`), asserts `detect-project.sh` output for each fixture with `python3 -I -c` JSON assertions (rails fixture with `config/database.yml` pointing at `localhost` → `local`; `.env` with `DATABASE_URL=postgres://u:secret@db.prod.example.com/x` → `sensitive` and the word `secret` absent from the output), and greps SKILL.md for the six-field frontmatter and `< 500` lines.
- [ ] **Step 2: Run it — expect failures (no files).**
- [ ] **Step 3: Write the Skill files.** Reference files each under 200 lines; `installation.md` includes the exact decision tree (binary on PATH → version check `pgx version` ≥ 0.1.1; else `npm i -g pgxdb` when `node ≥ 18`; else `curl -fsSL https://pgx.dev/install | sh` with `PGX_BIN_DIR`; supported platforms list; port behaviour: PGX picks a free port, never 5432 when busy; never `pgx init --force`). `postgres-import.md`: the `pgx copy --schema-only` default, data-copy approval wording, extension gate behaviour. `security.md`: the 14 rules from the brief in the Skill's voice.
- [ ] **Step 4: Run `skills/pgx/tests/run.sh` — pass. Step 5: Local install test: `npx -y skills add ./skills/pgx -a claude-code -y` in a scratch project; assert `.claude/skills/pgx/SKILL.md` exists; `npx -y skills add ./skills/pgx --list` shows `pgx`.**
- [ ] **Step 6: Commit** `feat(skill): the pgx Agent Skill`

---

### Task 11: Skill evaluation suite (Tests A–G with real PGX, no paid calls)

**Files:**
- Create: `skills/pgx/tests/e2e.sh` (bash, uses `PGX_DIST`, a scratch `PGX_HOME`, a scratch PG18 via `pg_ctl` for scenario B), `skills/pgx/tests/README.md`
- Scenarios: A `fresh` (PATH without pgx: `detect-project.sh` reports `installed:false`; then with the dist on PATH `pgx agent setup` from an empty project → `app` created, hooks written, check passed); B `existing-postgres` (scratch PG with a `users` table; `pgx copy --schema-only` → `pgx agent setup --source myapp_template` → `pgx agent up` twice with different ids → migrate-like DDL in A invisible in B → PG source fingerprint unchanged); C `pgx-only` (no PG: setup, `psql "$DATABASE_URL"` inside `pgx agent exec`); D `concurrent` (6 parallel `pgx agent exec` children each inserting into its own branch, count rows per branch == 1, template unchanged); E `lifecycle` (up/touch/down/revive/expire with `--ttl 3s`); F `security` (sensitive URL never echoed by `detect-project.sh`; `.pgx.toml` with a URL rejected; `pgx agent down` with a foreign branch name refuses; hook payload with `session_id` `../../x` rejected); G `installation-ux` (`npx skills add ./skills/pgx -a claude-code cursor codex -y` twice; `npx skills remove pgx -y`; existing `.claude/settings.json` foreign hooks intact after `pgx agent install` + `uninstall`).

- [ ] Steps: write the script with `set -euo pipefail` and one `pass/fail` line per scenario → run against the built binary → fix → commit `test(skill): end-to-end evaluation suite for the pgx skill`

---

### Task 12: CI

**Files:**
- Modify: `.github/workflows/ci.yml` (add `--test agent --test agent_hook --test agent_exec --test agent_install`; new job `skill` on ubuntu-24.04: `pipx run skills-ref validate skills/pgx`, `skills/pgx/tests/run.sh`, `npx -y skills@latest add ./skills/pgx --list`, `skills/pgx/tests/e2e.sh` with the built dist)

- [ ] Steps: edit → `act`-free syntax check (`python3 -c 'import yaml'` is not available; use `ruby -ryaml -e 'YAML.load_file(".github/workflows/ci.yml")'`) → commit `ci: run the agent suites and validate the pgx skill`

---

### Task 13: Live Claude Code validation (Test H, Test 21, Test 23)

**Files:**
- Create: `tests/apps/rails-agent/` (a minimal real Rails 8 app: `Gemfile` with `rails`, `pg`; `config/database.yml` reading `DATABASE_URL`; one `users` migration; one Minitest model test), `docs/pgx/local-dev/validation/2026-10-25-agent-live-claude.md` (transcript summary, commands, costs)
- Procedure (scratch copies, scratch `PGX_HOME`, scratch PG18 as "the developer's PostgreSQL" with the app's schema loaded):
  1. `npx -y skills add ./skills/pgx -a claude-code -y` in the app copy.
  2. `claude -p "Use PGX to give this project isolated databases for coding agents. Set everything up." --allowedTools "Bash,Read,Edit,Write,Glob,Grep" --output-format json` with `CLAUDECODE` unset. Expected: `.pgx.toml`, hooks in `.claude/settings.json`, `myapp_template` created schema-only, no data copied, the PG source fingerprint unchanged.
  3. `claude -p "Add a last_login_at timestamp to users with a migration and run the tests."` Expected: hook created `agent-claude-<id>`, `bin/rails db:migrate` and `bin/rails test` ran against it (check `pg_stat_statements`-free evidence: the branch has the column, `myapp_template` and the PG source do not).
  4. A second `claude -p` session concurrently (`--session-id` fixed UUIDs) inserting a row; the first does not see it.
  5. `claude --resume <id> -p "run the tests again"` reuses the branch within grace.
  Budget: at most 5 headless runs (~$0.50–2 each). Record each run's cost from the JSON result.

- [ ] Steps: build the app fixture and run it under a plain `DATABASE_URL` first (no agent) → run the live sequence → write the validation note with exact outcomes → commit `test(apps): rails agent fixture and live Claude Code validation notes`

---

### Task 14: Cursor validation and adapter decision

- Procedure: `agent status`; if not logged in, record the blocker (`agent login` is interactive). If logged in: scratch project with `pgx agent setup --hosts cursor`, `agent -p -f --output-format json "Run: echo $DATABASE_URL $PGX_BRANCH and psql \"$DATABASE_URL\" -c 'select current_database()'"`; verify the branch name; a second concurrent `agent -p`; record in `docs/pgx/local-dev/validation/2026-10-25-agent-live-cursor.md`. The compatibility matrix marks Cursor "verified" only on success; otherwise "hooks installed, env propagation unverified: use `pgx agent exec -- agent …`".
- [ ] Steps: run → document → update SKILL.md `cursor.md` and the matrix → commit `docs(agents): Cursor validation status`

---

### Task 15: Codex validation

- Procedure: `command -v codex`; if absent, do not install globally; the wrapper is validated by `tests/agent_exec.rs` and scenario D. Record native hook validation as pending with the exact commands a logged-in user runs (`npm i -g @openai/codex && codex login`, then the same sequence as Task 14 with `pgx agent exec -- codex exec "…"`). Document the sandbox requirement.
- [ ] Steps: document → commit `docs(agents): Codex validation status and sandbox note`

---

### Task 16: Documentation

**Files:**
- Create: `docs/pgx/local-dev/agents.md` (skill-first guide: install the skill, what the agent does, manual CLI, hosts matrix with verified/partial/wrapper, lifecycle semantics, fail-closed, limitations)
- Modify: `README.md` (install block: `npx skills add pgrundev/pgx --skill pgx` first, `npx pgxdb@latest` second; new "Every AI agent gets its own Postgres" section replacing the manual branch story's lead; badge placeholder line commented until verified), `docs/pgx/local-dev/cli.md`, `docs/pgx/local-dev/integrations.md` (agent workflow → the skill), `docs/pgx/local-dev/troubleshooting.md` (hook not firing, PATH, busy source, sandbox, Cursor reopen), `docs/pgx/local-dev/copy.md` (template naming `myapp_template`), `LLM.md` if it indexes docs
- [ ] Steps: write → `grep -n 'pgx agent' README.md docs -r | wc -l` sanity → commit `docs: skill-first agent isolation`

---

### Task 17: Website content (prepared, not deployed)

**Files:**
- Create: `dist/pgx.dev/site/index.html` (rebuilt from the live page's structure and fonts, headline "Every AI agent gets its own Postgres.", subtitle "Install one Skill. Your coding agent handles the database.", the two install commands, six bullets, links), `dist/pgx.dev/site/skill.html` (install and use with Claude Code, Cursor, Codex; links to GitHub, agentskills.io, cli.md; skills.sh link commented until verified), `dist/pgx.dev/site/README.md` (how to deploy: the live site is a static page on Cloudflare whose source is not in this repository; upload these two files).
- [ ] Steps: write → open in a headless check (`python3 -I -c` parse for the headline text) → commit `site: skill-first homepage and skill page (ready to deploy)`

---

### Task 18: Public distribution check and PR

- [ ] Push `feat/pgx-agent`; run `npx -y skills add pgrundev/pgx#feat/pgx-agent --list` (expect `pgx`), and `npx -y skills add https://github.com/pgrundev/pgx/tree/feat/pgx-agent/skills/pgx -a claude-code -y` in a scratch dir.
- [ ] Open the PR against `pgx/main` titled `feat: skill-first agent database isolation (pgx agent + the pgx Skill)` with: the brief's deliverable list mapped to commits, the compatibility matrix, test commands and counts from this branch, the live validation notes, blockers (Cursor login, Codex install/login, website deploy, skills.sh indexing after merge), and `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- [ ] Watch CI; fix; report. After the user merges, run the public install and check `https://skills.sh/pgrundev/pgx`; add the badge only when the page exists.
