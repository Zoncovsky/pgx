//! The lifecycle primitives: `up` (ensure this session's branch), `touch`
//! (activity: extend the idle TTL), `down` (session end: retire with a grace
//! period, or drop), `env` (the exports for an existing session), `ls`.
use super::identity::{self, SessionKey};
use super::project::{self, ProjectConfig};
use crate::cli::ExitCode;
use crate::cmd::db::{self, database_oid, socket_url, tcp_url};
use crate::cmd::Ctx;
use crate::errors::{PgxError, Result};
use crate::layout::Home;
use crate::names::{self, DbName};
use crate::out::Out;
use crate::ttl::Ttl;
use crate::ttlstore::{AgentMeta, MetaStore, TtlRecord};
use serde_json::json;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

type Client = crate::pgwire::Client<std::os::unix::net::UnixStream>;

/// `touch` rewrites the record at most this often.
pub const TOUCH_INTERVAL_SECS: u64 = 30;

/// What a command needs to know about the session before touching PGX.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub key: SessionKey,
    pub project: ProjectConfig,
    pub source: DbName,
    /// Whether the source came from `--source`/`PGX_SOURCE`/`.pgx.toml`
    /// (as opposed to the default `app`).
    pub explicit_source: bool,
    pub idle_ttl: Ttl,
    /// The idle TTL as the user wrote it (`2h`, `90s`): `Ttl::label()` can
    /// produce composite forms the parser rejects, so the original is kept.
    pub idle_label: String,
    pub grace: Ttl,
}

impl Resolved {
    /// A project that opted into isolation: a `.pgx.toml` exists or a source was named.
    pub fn isolation_enabled(&self) -> bool {
        self.project.config_file.is_some() || self.explicit_source
    }
    pub fn idle_secs(&self) -> u64 {
        self.idle_ttl.as_seconds().unwrap_or(7200)
    }
    pub fn grace_secs(&self) -> u64 {
        self.grace.as_seconds().unwrap_or(600)
    }
}

/// Inputs to `resolve`, from flags or from a host payload.
#[derive(Debug, Clone, Default)]
pub struct ResolveInput {
    pub id: Option<String>,
    pub host: Option<String>,
    pub source: Option<String>,
    pub ttl: Option<String>,
    pub grace: Option<String>,
    /// Where the session runs; the current directory when None.
    pub cwd: Option<PathBuf>,
    /// The host's project root(s), tried after `cwd`.
    pub fallbacks: Vec<PathBuf>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn finite_ttl(label: &str, what: &str) -> Result<Ttl> {
    match crate::ttl::parse_ttl(label)? {
        Ttl::Unlimited => Err(PgxError::usage(format!(
            "{what} must be a finite duration such as 2h: agent branches always expire (use `pgx branch … --ttl unlimited` for a permanent copy)"
        ))),
        t => Ok(t),
    }
}

pub fn resolve(input: &ResolveInput) -> Result<Resolved> {
    let host = match input.host.clone().or_else(|| env("PGX_AGENT_HOST")) {
        Some(h) => identity::validate_host(&h)?,
        None if env("CLAUDE_CODE_SESSION_ID").is_some() => "claude".to_string(),
        None if env("CODEX_SESSION_ID").is_some() => "codex".to_string(),
        None => "exec".to_string(),
    };
    let session_id = match input
        .id
        .clone()
        .or_else(|| identity::session_env_var(&host).and_then(env))
    {
        Some(id) => identity::validate_session_id(&id)?,
        None => {
            return Err(PgxError::usage(
                "no session id: pass --id <session>, or run inside a supported agent (Claude Code sets CLAUDE_CODE_SESSION_ID, Codex sets CODEX_SESSION_ID)",
            ))
        }
    };
    let cwd = match &input.cwd {
        Some(c) => c.clone(),
        None => std::env::current_dir()
            .map_err(|e| PgxError::failure(format!("cannot read the current directory: {e}")))?,
    };
    let mut fallbacks = input.fallbacks.clone();
    for var in ["CLAUDE_PROJECT_DIR", "CURSOR_PROJECT_DIR"] {
        if let Some(d) = env(var) {
            fallbacks.push(PathBuf::from(d));
        }
    }
    let project = project::locate(&cwd, &fallbacks)?;
    let (source_name, explicit_source) = match input
        .source
        .clone()
        .or_else(|| env("PGX_SOURCE"))
        .or_else(|| project.source.clone())
    {
        Some(s) => (s, true),
        None => (project::DEFAULT_SOURCE.to_string(), false),
    };
    let source = names::validate(&source_name)?;
    let idle_label = input
        .ttl
        .clone()
        .or_else(|| project.idle_ttl.clone())
        .unwrap_or_else(|| project::DEFAULT_IDLE_TTL.to_string());
    let grace_label = input
        .grace
        .clone()
        .or_else(|| project.grace_period.clone())
        .unwrap_or_else(|| project::DEFAULT_GRACE.to_string());
    let idle_ttl = finite_ttl(&idle_label, "--ttl")?;
    let grace = finite_ttl(&grace_label, "--grace")?;
    Ok(Resolved {
        key: SessionKey {
            host,
            session_id,
            project_id: project.project_id.clone(),
        },
        project,
        source,
        explicit_source,
        idle_ttl,
        idle_label,
        grace,
    })
}

/// Serialise every lifecycle operation on one session. Two hook deliveries for
/// the same session (Cursor's native file plus its Claude-compat loader, or a
/// racing resume) take turns instead of cloning twice.
pub fn lock_session(home: &Home, key: &SessionKey) -> Result<std::fs::File> {
    let dir = home.root.join("agent").join("locks");
    std::fs::create_dir_all(&dir).map_err(|e| PgxError::io("cannot create", &dir, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let material = format!("{}|{}|{}", key.project_id, key.host, key.session_id);
    let path = dir.join(format!(
        "{}.lock",
        identity::hex16(identity::fnv1a64(material.as_bytes()))
    ));
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| PgxError::io("cannot open", &path, &e))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(PgxError::failure(format!(
            "cannot lock {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(f)
}

fn meta_matches(m: &AgentMeta, key: &SessionKey) -> bool {
    m.host == key.host && m.session_id == key.session_id && m.project_id == key.project_id
}

/// The record owned by this session, if any.
pub fn find_record(store: &MetaStore, key: &SessionKey) -> Result<Option<TtlRecord>> {
    let (records, _bad) = store.list();
    Ok(records
        .into_iter()
        .find(|r| r.agent.as_ref().is_some_and(|m| meta_matches(m, key))))
}

/// `Ctx::load` with the agent-friendly message when PGX was never initialised.
pub fn load_ctx() -> Result<Ctx> {
    let home = Home::resolve();
    if !home.is_initialized() {
        return Err(PgxError::new(
            ExitCode::NotReady,
            format!("PGX is not initialised at {}", home.root.display()),
        )
        .hint("run `pgx run` once (or `pgx agent setup`), then start the agent session again"));
    }
    Ctx::load()
}

/// What `up` produced.
#[derive(Debug, Clone)]
pub struct UpResult {
    pub branch: DbName,
    pub url: String,
    pub socket_url: String,
    pub created: bool,
    /// The previous branch for this session had vanished; this is a new one.
    pub fresh: bool,
    pub source: String,
    pub source_created: bool,
    pub expires_at: u64,
    pub now: u64,
    pub idle_ttl: Ttl,
    pub key: SessionKey,
    pub project_root: PathBuf,
    pub explicit_source: bool,
    pub method: &'static str,
    pub elapsed_ms: f64,
    pub port: u16,
    pub password: String,
}

fn new_meta(r: &Resolved, now: u64) -> AgentMeta {
    AgentMeta {
        host: r.key.host.clone(),
        session_id: r.key.session_id.clone(),
        project_id: r.key.project_id.clone(),
        project_root: r.project.root.to_string_lossy().into_owned(),
        source: r.source.as_str().to_string(),
        last_touch: now,
        retired: false,
    }
}

/// Ensure this session's branch exists and is current. Idempotent.
pub fn up(out: &Out, r: &Resolved) -> Result<UpResult> {
    let ctx = load_ctx()?;
    let _lock = lock_session(&ctx.home, &r.key)?;
    let quiet = out.quiet();
    crate::cmd::start::ensure_running(&quiet, &ctx)?;
    crate::cmd::start::spawn_supervisor(&ctx.home);
    let store = MetaStore::new(&ctx.home.root);
    let mut client = ctx.connect("postgres")?;
    let now = crate::fsutil::now_epoch();
    let idle = r.idle_secs();

    // An existing branch for this session: extend it and hand it back.
    let mut fresh = false;
    if let Some(mut rec) = find_record(&store, &r.key)? {
        match database_oid(&mut client, &rec.name)? {
            Some(oid) if oid == rec.oid => {
                rec.expires_at = Some(now + idle);
                rec.ttl_label = r.idle_label.clone();
                rec.ttl_seconds = Some(idle);
                rec.state = crate::ttlstore::State::Active;
                if let Some(m) = rec.agent.as_mut() {
                    m.last_touch = now;
                    m.retired = false;
                }
                store.save(&rec)?;
                let branch = names::validate(&rec.name)?;
                client.close();
                return Ok(UpResult {
                    url: tcp_url(&ctx, branch.as_str()),
                    socket_url: socket_url(&ctx, branch.as_str()),
                    branch,
                    created: false,
                    fresh: false,
                    source: rec
                        .agent
                        .as_ref()
                        .map(|m| m.source.clone())
                        .unwrap_or_default(),
                    source_created: false,
                    expires_at: now + idle,
                    now,
                    idle_ttl: r.idle_ttl,
                    key: r.key.clone(),
                    project_root: r.project.root.clone(),
                    explicit_source: r.explicit_source,
                    method: "existing",
                    elapsed_ms: 0.0,
                    port: ctx.config.port,
                    password: ctx.config.password.clone(),
                });
            }
            _ => {
                // Dropped by hand, expired, or a different incarnation: forget it.
                store.remove(&rec.name)?;
                fresh = true;
            }
        }
    }

    // The source must exist and be idle.
    let mut source_created = false;
    if !db::exists(&ctx, &mut client, &r.source)? {
        if r.explicit_source {
            return Err(PgxError::new(
                ExitCode::NotFound,
                format!("source database {} does not exist", r.source),
            )
            .hint(format!(
                "import it with `pgx copy --from \"$DATABASE_URL\" --to {} --schema-only`, or create it with `pgx create {}`",
                r.source, r.source
            )));
        }
        // The default `app`, exactly as `pgx run` would create it.
        if let Err(e) = client.simple_query(&format!("CREATE DATABASE {}", r.source.quoted())) {
            let err = crate::cmd::db_error(e, &format!("cannot create database {}", r.source));
            if err.code != ExitCode::Conflict {
                return Err(err);
            }
        }
        source_created = true;
    }
    let busy = db::active_connections(&mut client, r.source.as_str())?;
    if busy > 0 {
        return Err(PgxError::new(
            ExitCode::InUse,
            format!(
                "source {} has {busy} active connection{}; a branch source must be idle",
                r.source,
                if busy == 1 { "" } else { "s" }
            ),
        )
        .hint(format!(
            "keep it as a template nobody connects to (pgx copy --to {}_template) or disconnect first",
            r.source
        )));
    }

    // Pick the name: readable unless a foreign database already has it.
    let short = names::validate(&identity::branch_name(&r.key, false))?;
    let branch = if db::exists(&ctx, &mut client, &short)? {
        let long = names::validate(&identity::branch_name(&r.key, true))?;
        if db::exists(&ctx, &mut client, &long)? {
            return Err(PgxError::new(
                ExitCode::Conflict,
                format!("databases {short} and {long} both exist and belong to no agent session"),
            )
            .hint("drop or rename them, or pick another --id"));
        }
        long
    } else {
        short
    };

    let cloned = db::clone_database(&ctx, &mut client, &r.source, &branch, Some(&r.idle_label))?;
    // Attach the session to the record `clone_database` wrote.
    let mut rec = store
        .load(branch.as_str())?
        .ok_or_else(|| PgxError::failure(format!("no TTL record for {branch} after cloning")))?;
    rec.agent = Some(new_meta(r, now));
    rec.expires_at = Some(now + idle);
    store.save(&rec)?;
    client.close();
    Ok(UpResult {
        url: tcp_url(&ctx, branch.as_str()),
        socket_url: socket_url(&ctx, branch.as_str()),
        branch,
        created: true,
        fresh,
        source: r.source.as_str().to_string(),
        source_created,
        expires_at: now + idle,
        now,
        idle_ttl: r.idle_ttl,
        key: r.key.clone(),
        project_root: r.project.root.clone(),
        explicit_source: r.explicit_source,
        method: cloned.method,
        elapsed_ms: cloned.elapsed_ms,
        port: ctx.config.port,
        password: ctx.config.password.clone(),
    })
}

/// Outcome of `touch`.
#[derive(Debug, Clone)]
pub struct Touched {
    pub branch: Option<String>,
    pub skipped: bool,
    pub expires_at: Option<u64>,
}

/// Activity: move the deadline to now + idle TTL, at most once per
/// `TOUCH_INTERVAL_SECS`. No database connection; the supervisor reads the file.
pub fn touch(r: &Resolved) -> Result<Touched> {
    let home = Home::resolve();
    if !home.is_initialized() {
        return Ok(Touched {
            branch: None,
            skipped: true,
            expires_at: None,
        });
    }
    let _lock = lock_session(&home, &r.key)?;
    let store = MetaStore::new(&home.root);
    let Some(mut rec) = find_record(&store, &r.key)? else {
        return Ok(Touched {
            branch: None,
            skipped: true,
            expires_at: None,
        });
    };
    let now = crate::fsutil::now_epoch();
    let last = rec.agent.as_ref().map(|m| m.last_touch).unwrap_or(0);
    if now.saturating_sub(last) < TOUCH_INTERVAL_SECS {
        return Ok(Touched {
            branch: Some(rec.name),
            skipped: true,
            expires_at: rec.expires_at,
        });
    }
    let idle = rec.ttl_seconds.unwrap_or(r.idle_secs());
    rec.expires_at = Some(now + idle);
    rec.state = crate::ttlstore::State::Active;
    if let Some(m) = rec.agent.as_mut() {
        m.last_touch = now;
        m.retired = false;
    }
    store.save(&rec)?;
    Ok(Touched {
        branch: Some(rec.name),
        skipped: false,
        expires_at: rec.expires_at,
    })
}

/// Outcome of `down`.
#[derive(Debug, Clone)]
pub struct DownResult {
    pub branch: Option<String>,
    pub dropped: bool,
    pub retired_until: Option<u64>,
}

/// Session end: retire the branch (grace period) or drop it now. Only a branch
/// whose record names this exact session is ever touched.
pub fn down(r: &Resolved, now_flag: bool) -> Result<DownResult> {
    let home = Home::resolve();
    if !home.is_initialized() {
        return Ok(DownResult {
            branch: None,
            dropped: false,
            retired_until: None,
        });
    }
    let _lock = lock_session(&home, &r.key)?;
    let store = MetaStore::new(&home.root);
    let Some(mut rec) = find_record(&store, &r.key)? else {
        return Ok(DownResult {
            branch: None,
            dropped: false,
            retired_until: None,
        });
    };
    let now = crate::fsutil::now_epoch();
    if !now_flag {
        rec.expires_at = Some(now + r.grace_secs());
        if let Some(m) = rec.agent.as_mut() {
            m.retired = true;
            m.last_touch = now;
        }
        store.save(&rec)?;
        return Ok(DownResult {
            branch: Some(rec.name),
            dropped: false,
            retired_until: Some(now + r.grace_secs()),
        });
    }
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    drop_branch(&mut client, &rec)?;
    client.close();
    store.remove(&rec.name)?;
    Ok(DownResult {
        branch: Some(rec.name),
        dropped: true,
        retired_until: None,
    })
}

/// Terminate sessions on this branch only, then drop it. The OID gate keeps a
/// reused name (a different incarnation) safe.
fn drop_branch(client: &mut Client, rec: &TtlRecord) -> Result<()> {
    match database_oid(client, &rec.name)? {
        None => return Ok(()),
        Some(oid) if oid != rec.oid => return Ok(()),
        Some(_) => {}
    }
    let name = names::validate(&rec.name)?;
    let _ = client.simple_query(&format!(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = {} AND pid <> pg_backend_pid()",
        crate::pgwire::quote_literal(&rec.name)
    ));
    client
        .simple_query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            name.quoted()
        ))
        .map_err(|e| crate::cmd::db_error(e, &format!("cannot drop {}", rec.name)))?;
    Ok(())
}

fn sq(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// The nine exports a session receives.
pub fn env_lines(res: &UpResult) -> String {
    let mut s = String::new();
    for (k, v) in [
        ("DATABASE_URL", res.url.clone()),
        ("PGX_BRANCH", res.branch.as_str().to_string()),
        ("PGX_SOURCE", res.source.clone()),
        ("PGHOST", "127.0.0.1".to_string()),
        ("PGPORT", res.port.to_string()),
        ("PGUSER", "postgres".to_string()),
        ("PGPASSWORD", res.password.clone()),
        ("PGDATABASE", res.branch.as_str().to_string()),
        ("PGX_ISOLATION", "ok".to_string()),
    ] {
        s.push_str(&format!("export {k}={}\n", sq(&v)));
    }
    s
}

/// Fail closed: a URL that cannot connect, so a session whose isolation failed
/// never falls back to an inherited database.
pub fn sentinel_env_lines(reason: &str) -> String {
    let mut s = String::new();
    for (k, v) in [
        (
            "DATABASE_URL",
            "postgres://pgx-isolation-failed@127.0.0.1:1/pgx_isolation_failed",
        ),
        ("PGX_ISOLATION", "failed"),
        ("PGX_ISOLATION_REASON", reason),
        ("PGHOST", "127.0.0.1"),
        ("PGPORT", "1"),
        ("PGUSER", "pgx-isolation-failed"),
        ("PGDATABASE", "pgx_isolation_failed"),
    ] {
        s.push_str(&format!("export {k}={}\n", sq(v)));
    }
    s.push_str("unset PGPASSWORD PGX_BRANCH PGX_SOURCE\n");
    s
}

pub fn up_json(res: &UpResult) -> serde_json::Value {
    let mut v = json!({
        "branch": res.branch.as_str(),
        "source": res.source,
        "source_created": res.source_created,
        "created": res.created,
        "fresh": res.fresh,
        "url": res.url,
        "socket_url": res.socket_url,
        "host": res.key.host,
        "session_id": res.key.session_id,
        "project_id": res.key.project_id,
        "project_root": res.project_root.to_string_lossy(),
        "idle_ttl": res.idle_ttl.label(),
        "idle_ttl_seconds": res.idle_ttl.as_seconds(),
        "expires_at": crate::fsutil::rfc3339(res.expires_at),
        "expires_in_seconds": res.expires_at.saturating_sub(res.now),
        "method": res.method,
        "elapsed_ms": (res.elapsed_ms * 10.0).round() / 10.0,
    });
    v["port"] = json!(res.port);
    v
}

pub fn up_text(res: &UpResult) -> String {
    let verb = if res.created { "created" } else { "ready" };
    let mut src = res.source.clone();
    if res.source_created {
        src.push_str(" (created empty: no .pgx.toml names a source)");
    } else if res.fresh {
        src.push_str(" (fresh: the previous branch was gone)");
    }
    format!(
        "✓ Branch {verb}: {}\n✓ Source: {src}\n✓ Expires in: {} after the last activity\n\nDATABASE_URL:\n{}",
        res.branch,
        crate::ttl::humanize_long(res.idle_ttl.as_seconds().unwrap_or(0)),
        res.url
    )
}

/// `pgx agent ls`.
pub fn ls(out: &Out) -> Result<ExitCode> {
    let ctx = load_ctx()?;
    let store = MetaStore::new(&ctx.home.root);
    let (records, _bad) = store.list();
    let now = crate::fsutil::now_epoch();
    let mut sizes = std::collections::HashMap::new();
    if ctx.running_pid().is_some() {
        if let Ok(mut client) = ctx.connect("postgres") {
            if let Ok(r) =
                client.simple_query("SELECT datname, pg_database_size(oid) FROM pg_database")
            {
                for row in r.rows {
                    if let (Some(n), Some(s)) = (
                        row[0].clone(),
                        row[1].as_deref().and_then(|s| s.parse::<u64>().ok()),
                    ) {
                        sizes.insert(n, s);
                    }
                }
            }
            client.close();
        }
    }
    let mut text = format!(
        "{:<34} {:<8} {:<14} {:<16} {:<8} {:<12} {:>9}\n",
        "NAME", "HOST", "SESSION", "SOURCE", "STATE", "EXPIRES IN", "SIZE"
    );
    let mut items = Vec::new();
    for rec in records.iter().filter(|r| r.agent.is_some()) {
        let m = rec.agent.as_ref().unwrap();
        let state = if rec.is_expired(now) {
            "expired"
        } else if m.retired {
            "retired"
        } else {
            "active"
        };
        let sid: String = m.session_id.chars().take(12).collect();
        let size = sizes.get(&rec.name).copied();
        text.push_str(&format!(
            "{:<34} {:<8} {:<14} {:<16} {:<8} {:<12} {:>9}\n",
            rec.name,
            m.host,
            sid,
            m.source,
            state,
            crate::ttl::humanize_remaining(rec.expires_at, now),
            size.map(human_size).unwrap_or_else(|| "-".into()),
        ));
        items.push(json!({
            "name": rec.name,
            "state": state,
            "ttl": rec.ttl_label,
            "ttl_seconds": rec.ttl_seconds,
            "expires_at": rec.expires_at.map(crate::fsutil::rfc3339),
            "expires_in_seconds": rec.expires_at.map(|e| e.saturating_sub(now)),
            "created_at": crate::fsutil::rfc3339(rec.created_at),
            "age_seconds": now.saturating_sub(rec.created_at),
            "size_bytes": size,
            "agent": m.to_json(),
        }));
    }
    if items.is_empty() {
        out.emit("no agent branches", json!(items));
    } else {
        out.emit(text.trim_end(), json!(items));
    }
    Ok(ExitCode::Ok)
}

fn human_size(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b < K {
        format!("{b:.0} B")
    } else if b < K * K {
        format!("{:.0} KB", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB", b / K / K)
    } else {
        format!("{:.2} GB", b / K / K / K)
    }
}

/// `pgx agent env`: the exports for an existing session, creating nothing.
pub fn env_for_existing(r: &Resolved) -> Result<String> {
    let ctx = load_ctx()?;
    let store = MetaStore::new(&ctx.home.root);
    let Some(rec) = find_record(&store, &r.key)? else {
        return Err(PgxError::new(
            ExitCode::NotFound,
            format!("no branch for {} session {}", r.key.host, r.key.session_id),
        )
        .hint("`pgx agent up` creates one"));
    };
    let branch = names::validate(&rec.name)?;
    let res = UpResult {
        url: tcp_url(&ctx, branch.as_str()),
        socket_url: socket_url(&ctx, branch.as_str()),
        branch,
        created: false,
        fresh: false,
        source: rec
            .agent
            .as_ref()
            .map(|m| m.source.clone())
            .unwrap_or_default(),
        source_created: false,
        expires_at: rec.expires_at.unwrap_or(0),
        now: crate::fsutil::now_epoch(),
        idle_ttl: Ttl::Seconds(rec.ttl_seconds.unwrap_or(0)),
        key: r.key.clone(),
        project_root: r.project.root.clone(),
        explicit_source: r.explicit_source,
        method: "existing",
        elapsed_ms: 0.0,
        port: ctx.config.port,
        password: ctx.config.password.clone(),
    };
    Ok(env_lines(&res))
}

#[allow(dead_code)]
fn _assert_path(_: &Path) {}
