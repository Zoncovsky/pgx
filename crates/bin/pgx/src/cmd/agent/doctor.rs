//! `pgx agent doctor`: is this project ready for automatic isolation, and is
//! the current session actually using its branch?
use super::hosts::Host;
use super::install;
use super::project;
use super::session;
use super::Scope;
use crate::cli::ExitCode;
use crate::errors::Result;
use crate::layout::Home;
use crate::out::Out;
use crate::process;
use serde_json::{json, Value};
use std::path::Path;

struct Check {
    name: String,
    status: &'static str,
    detail: String,
    hint: Option<String>,
}

fn ok(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: "ok",
        detail: detail.into(),
        hint: None,
    }
}
fn warn(name: impl Into<String>, detail: impl Into<String>, hint: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: "warn",
        detail: detail.into(),
        hint: Some(hint.into()),
    }
}
fn fail(name: impl Into<String>, detail: impl Into<String>, hint: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: "fail",
        detail: detail.into(),
        hint: Some(hint.into()),
    }
}

/// Our entries per event in one hook file: (events covered, duplicate events).
fn count_ours(path: &Path) -> Option<(usize, usize, bool)> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc: Value = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(_) => return Some((0, 0, true)),
    };
    let mut covered = 0;
    let mut dup = 0;
    if let Some(hooks) = doc.get("hooks").and_then(|h| h.as_object()) {
        for (_, list) in hooks {
            let n = list
                .as_array()
                .map(|a| a.iter().filter(|e| install::is_ours(e)).count())
                .unwrap_or(0);
            if n > 0 {
                covered += 1;
            }
            if n > 1 {
                dup += 1;
            }
        }
    }
    Some((covered, dup, false))
}

fn expected_events(host: Host) -> usize {
    match host {
        Host::Claude | Host::Cursor => 4,
        Host::Codex => 3,
    }
}

fn hooks_check(host: Host, root: &Path) -> Check {
    let name = format!("hooks:{}", host.as_str());
    let expected = expected_events(host);
    let mut found = Vec::new();
    let mut partial = Vec::new();
    let mut dups = Vec::new();
    let mut broken = Vec::new();
    for (scope, path) in install::host_files(host, root) {
        let Some((covered, dup, invalid)) = count_ours(&path) else {
            continue;
        };
        let label = format!(
            "{} ({})",
            path.display(),
            match scope {
                Scope::Project => "project",
                Scope::Local => "local",
                Scope::User => "user",
            }
        );
        if invalid {
            broken.push(label);
        } else if dup > 0 {
            dups.push(label);
        } else if covered == expected {
            found.push(label);
        } else if covered > 0 {
            partial.push(label);
        }
    }
    if !broken.is_empty() {
        return fail(
            name,
            format!("not valid JSON: {}", broken.join(", ")),
            "fix the file by hand",
        );
    }
    if !dups.is_empty() {
        return warn(
            name,
            format!("duplicate pgx entries in {}", dups.join(", ")),
            "run `pgx agent uninstall` then `pgx agent install` for that scope",
        );
    }
    if found.len() > 1 {
        return warn(
            name,
            format!("installed at more than one scope: {}", found.join(", ")),
            "keep one; the host would run the hook twice (harmless, but noisy)",
        );
    }
    if let Some(f) = found.first() {
        return ok(name, format!("{f}: {expected} hooks"));
    }
    if let Some(p) = partial.first() {
        return warn(
            name,
            format!("partial installation in {p}"),
            "run `pgx agent install` to complete it",
        );
    }
    warn(
        name,
        "not installed",
        format!(
            "run `pgx agent setup` (or `pgx agent install {}`)",
            host.as_str()
        ),
    )
}

pub fn run(out: &Out, cwd: &Path) -> Result<ExitCode> {
    let mut checks: Vec<Check> = Vec::new();
    let home = Home::resolve();
    let project = project::locate(cwd, &[]);
    let (root, config) = match &project {
        Ok(p) => (p.root.clone(), p.config_file.clone()),
        Err(e) => {
            checks.push(fail("config", e.message.clone(), "fix .pgx.toml"));
            (cwd.to_path_buf(), None)
        }
    };
    if let Ok(p) = &project {
        match &config {
            Some(f) => checks.push(ok(
                "config",
                format!(
                    "{} (source {}, idle TTL {}, grace {})",
                    f.display(),
                    p.source
                        .clone()
                        .unwrap_or_else(|| project::DEFAULT_SOURCE.into()),
                    p.idle_ttl
                        .clone()
                        .unwrap_or_else(|| project::DEFAULT_IDLE_TTL.into()),
                    p.grace_period
                        .clone()
                        .unwrap_or_else(|| project::DEFAULT_GRACE.into()),
                ),
            )),
            None => checks.push(warn(
                "config",
                format!(
                    "no .pgx.toml found from {} (sessions would branch from `app`)",
                    cwd.display()
                ),
                "run `pgx agent setup --source <db>`",
            )),
        }
    }

    // Runtime.
    let ctx = if home.is_initialized() {
        crate::cmd::Ctx::load().ok()
    } else {
        None
    };
    let running = ctx.as_ref().and_then(|c| c.running_pid());
    match (&ctx, running) {
        (None, _) => checks.push(fail(
            "runtime",
            format!("PGX is not initialised at {}", home.root.display()),
            "run `pgx run` or `pgx agent setup`",
        )),
        (Some(c), Some(pid)) => checks.push(ok(
            "runtime",
            format!("running, pid {pid}, port {}", c.config.port),
        )),
        (Some(_), None) => checks.push(warn(
            "runtime",
            "initialised but stopped",
            "run `pgx start` (hooks start it on demand too)",
        )),
    }
    let sup = std::fs::read_to_string(home.root.join("supervisor.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    match (running, sup) {
        (Some(_), Some(pid)) if process::pid_alive(pid) => {
            checks.push(ok("supervisor", format!("running, pid {pid}")))
        }
        (Some(_), _) => checks.push(warn(
            "supervisor",
            "not running; expired branches are not being removed",
            "run `pgx restart`",
        )),
        _ => {}
    }

    // Source.
    // The same precedence the hook uses: PGX_SOURCE, then .pgx.toml, then app.
    let source_name = std::env::var("PGX_SOURCE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| project.as_ref().ok().and_then(|p| p.source.clone()))
        .unwrap_or_else(|| project::DEFAULT_SOURCE.to_string());
    if let (Some(c), Some(_)) = (&ctx, running) {
        match c.connect("postgres") {
            Ok(mut client) => {
                let exists = crate::names::validate(&source_name)
                    .ok()
                    .and_then(|n| crate::cmd::db::exists(c, &mut client, &n).ok())
                    .unwrap_or(false);
                if !exists {
                    checks.push(fail(
                        "source",
                        format!("{source_name} does not exist"),
                        format!("pgx copy --from \"$DATABASE_URL\" --to {source_name} --schema-only, or pgx create {source_name}"),
                    ));
                } else {
                    let conns =
                        crate::cmd::db::active_connections(&mut client, &source_name).unwrap_or(0);
                    let size: u64 = client
                        .simple_query(&format!(
                            "SELECT pg_database_size({})",
                            crate::pgwire::quote_literal(&source_name)
                        ))
                        .ok()
                        .and_then(|r| r.scalar().and_then(|s| s.parse().ok()))
                        .unwrap_or(0);
                    if conns > 0 {
                        checks.push(fail(
                            "source",
                            format!("{source_name} has {conns} active connection(s)"),
                            "a branch source must be idle: disconnect, or keep a dedicated template nobody connects to",
                        ));
                    } else {
                        checks.push(ok(
                            "source",
                            format!("{source_name} ({:.1} MB, idle)", size as f64 / 1048576.0),
                        ));
                    }
                }
                client.close();
            }
            Err(e) => checks.push(fail(
                "source",
                e.message,
                "is the runtime healthy? `pgx doctor`",
            )),
        }
    } else {
        checks.push(warn(
            "source",
            format!("{source_name}: not checked (runtime down)"),
            "start PGX",
        ));
    }

    // Hooks per host.
    let mut hosts = install::detect_hosts(&root);
    for h in [Host::Claude, Host::Cursor, Host::Codex] {
        if !hosts.contains(&h)
            && install::host_files(h, &root)
                .iter()
                .any(|(_, p)| p.exists())
        {
            hosts.push(h);
        }
    }
    if hosts.is_empty() {
        hosts = vec![Host::Claude, Host::Cursor, Host::Codex];
    }
    for h in hosts {
        checks.push(hooks_check(h, &root));
    }

    // The current session, when we are inside one.
    let host_var = [
        ("claude", "CLAUDE_CODE_SESSION_ID"),
        ("codex", "CODEX_SESSION_ID"),
    ]
    .into_iter()
    .find_map(|(h, v)| {
        std::env::var(v)
            .ok()
            .filter(|s| !s.is_empty())
            .map(|id| (h, id))
    });
    if let Some((host, id)) = host_var {
        let resolved = session::resolve(&session::ResolveInput {
            id: Some(id.clone()),
            host: Some(host.to_string()),
            cwd: Some(cwd.to_path_buf()),
            ..Default::default()
        });
        match resolved {
            Err(e) => checks.push(fail(
                "session",
                e.message,
                "fix the session id or .pgx.toml",
            )),
            Ok(r) => {
                let store = crate::ttlstore::MetaStore::new(&home.root);
                match session::find_record(&store, &r.key) {
                    Ok(Some(rec)) => {
                        let branch = rec.name.clone();
                        match std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
                            Some(url) => {
                                let path = url.split('?').next().unwrap_or("");
                                if path.ends_with(&format!("/{branch}")) {
                                    checks.push(ok("session", format!("{host} session {id}: DATABASE_URL is branch {branch}")));
                                } else {
                                    checks.push(fail(
                                        "session",
                                        format!("{host} session {id}: branch {branch} exists but DATABASE_URL names another database"),
                                        "do not run database commands in this session; `eval \"$(pgx agent env)\"` or start a new session so the hook sets it",
                                    ));
                                }
                            }
                            None => checks.push(warn(
                                "session",
                                format!("{host} session {id}: branch {branch} exists but DATABASE_URL is not set here"),
                                "eval \"$(pgx agent env)\"",
                            )),
                        }
                    }
                    Ok(None) => checks.push(warn(
                        "session",
                        format!("{host} session {id}: no branch yet"),
                        "the SessionStart hook creates one; `pgx agent up` does it by hand",
                    )),
                    Err(e) => checks.push(fail("session", e.message, "see `pgx doctor`")),
                }
            }
        }
    }

    // Agent branches overall.
    let store = crate::ttlstore::MetaStore::new(&home.root);
    let (records, _) = store.list();
    let now = crate::fsutil::now_epoch();
    let agents: Vec<_> = records.iter().filter(|r| r.agent.is_some()).collect();
    let retired = agents
        .iter()
        .filter(|r| r.agent.as_ref().unwrap().retired)
        .count();
    let expired = agents.iter().filter(|r| r.is_expired(now)).count();
    checks.push(ok(
        "branches",
        format!(
            "{} agent branch(es): {} retired, {} expired awaiting cleanup",
            agents.len(),
            retired,
            expired
        ),
    ));

    let any_fail = checks.iter().any(|c| c.status == "fail");
    let mut text = String::new();
    for c in &checks {
        text.push_str(&format!("{:<5} {:<14} {}\n", c.status, c.name, c.detail));
        if c.status != "ok" {
            if let Some(h) = &c.hint {
                text.push_str(&format!("      {:<14} hint: {h}\n", ""));
            }
        }
    }
    text.push_str(if any_fail {
        "\nproblems found"
    } else {
        "\nno problems found"
    });
    out.emit(
        &text,
        json!({
            "ok": !any_fail,
            "checks": checks.iter().map(|c| json!({"name": c.name, "status": c.status, "detail": c.detail, "hint": c.hint})).collect::<Vec<_>>(),
        }),
    );
    Ok(if any_fail {
        ExitCode::Failure
    } else {
        ExitCode::Ok
    })
}
