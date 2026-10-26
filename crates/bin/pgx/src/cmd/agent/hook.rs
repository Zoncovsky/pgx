//! `pgx agent hook`: the one command every host's hook configuration runs.
//! Reads the host's JSON payload on stdin, works out who is calling and what
//! happened, and answers in that host's dialect. It never exits 2 (a blocking
//! error in every host) and never prints a password to stdout.
use super::hosts::{self, Event, HookInput, Host};
use super::session::{self, ResolveInput, Resolved, UpResult};
use super::HookOpts;
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::out::Out;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};

fn env_map() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn notice(msg: &str) {
    eprintln!("pgx agent hook: {msg}");
}

/// The sentence the agent sees when its branch is ready.
fn ready_context(res: &UpResult, host: Host, env: &HashMap<String, String>) -> String {
    let mut origin = format!("a copy of {}", res.source);
    if res.source_created {
        origin.push_str(", created empty because no .pgx.toml names a source");
    } else if res.fresh {
        origin.push_str(", fresh");
    }
    let mut s = format!(
        "PGX: this session's database is branch {} ({origin}). DATABASE_URL and PG* are already set in your shell; do not point them elsewhere. The branch is disposable and expires {} after your last activity.",
        res.branch,
        crate::ttl::humanize_long(res.idle_ttl.as_seconds().unwrap_or(0)),
    );
    if host == Host::Codex {
        s.push_str(
            " Shell commands do not inherit it from this hook: run database commands as `pgx agent exec -- <command>`, or start Codex as `pgx agent exec -- codex`.",
        );
        if env
            .get("CODEX_SANDBOX_NETWORK_DISABLED")
            .is_some_and(|v| v == "1")
        {
            s.push_str(
                " Codex's sandbox blocks loopback TCP; set sandbox_workspace_write.network_access = true for this project.",
            );
        }
    }
    s
}

fn failed_context(reason: &str) -> String {
    format!(
        "PGX isolation FAILED for this session: {reason}. DATABASE_URL now points at an unreachable sentinel so nothing runs against another database. Fix it (pgx agent doctor) and start a new session."
    )
}

/// Append to `$CLAUDE_ENV_FILE`; without it, the lines go to stderr as a notice.
fn write_claude_env(env: &HashMap<String, String>, lines: &str) {
    match env.get("CLAUDE_ENV_FILE").filter(|p| !p.is_empty()) {
        Some(path) => {
            let r = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(lines.as_bytes()));
            if let Err(e) = r {
                notice(&format!("cannot write {path}: {e}"));
            }
        }
        None => {
            // Never echo values (the URL carries the password): names only.
            let names: Vec<&str> = lines
                .lines()
                .filter_map(|l| l.strip_prefix("export "))
                .filter_map(|l| l.split('=').next())
                .collect();
            notice(&format!(
                "CLAUDE_ENV_FILE is not set (not a SessionStart hook?); could not export {}",
                names.join(", ")
            ))
        }
    }
}

fn pairs_to_json(pairs: &[(&'static str, String)]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), json!(v));
    }
    serde_json::Value::Object(m)
}

fn emit_start(host: Host, env: &HashMap<String, String>, res: &UpResult) {
    let context = ready_context(res, host, env);
    match host {
        Host::Claude => {
            write_claude_env(env, &session::env_lines(res));
            println!("{context}");
        }
        Host::Cursor => println!(
            "{}",
            json!({"env": pairs_to_json(&session::env_pairs(res)), "additional_context": context})
        ),
        Host::Codex => println!(
            "{}",
            json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}})
        ),
    }
}

/// `up` failed. With isolation enabled the session fails closed; otherwise it
/// just learns why it has no database.
fn emit_failure(
    host: Host,
    env: &HashMap<String, String>,
    isolation: bool,
    reason: &str,
) -> ExitCode {
    notice(&format!("no database for this session: {reason}"));
    if isolation {
        let context = failed_context(reason);
        match host {
            Host::Claude => {
                write_claude_env(env, &session::sentinel_env_lines(reason));
                println!("{context}");
            }
            Host::Cursor => println!(
                "{}",
                json!({"env": pairs_to_json(&session::sentinel_pairs(reason)), "additional_context": context})
            ),
            Host::Codex => println!(
                "{}",
                json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}})
            ),
        }
        // Hosts apply a hook's output only on exit 0 (Cursor drops the JSON,
        // Claude drops the context): the sentinel travels on success, the
        // stderr notice explains. Exit 1 is for the case nothing was emitted.
        return ExitCode::Ok;
    }
    ExitCode::Failure
}

fn cursor_ack(host: Host) {
    if host == Host::Cursor {
        println!("{{}}");
    }
}

pub fn run(out: &Out, opts: HookOpts) -> Result<ExitCode> {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() || payload.trim().is_empty() {
        return Ok(ExitCode::Ok);
    }
    let env = env_map();
    let value: serde_json::Value = match serde_json::from_str(&payload) {
        Ok(v) => v,
        Err(e) => {
            notice(&format!("payload is not JSON: {e}"));
            return Ok(ExitCode::Failure);
        }
    };
    let input: HookInput = match hosts::detect(&value, &env, opts.host.as_deref()) {
        Ok(i) => i,
        Err(e) => {
            notice(&e.message);
            return Ok(ExitCode::Failure);
        }
    };
    if input.background || input.event == Event::Ignore {
        cursor_ack(input.host);
        return Ok(ExitCode::Ok);
    }
    let resolved: std::result::Result<Resolved, PgxError> = session::resolve(&ResolveInput {
        id: Some(input.session_id.clone()),
        host: Some(input.host.as_str().to_string()),
        source: opts.source.clone(),
        ttl: opts.ttl.clone(),
        grace: opts.grace.clone(),
        cwd: input.cwd.clone(),
        fallbacks: input.project_dir.iter().cloned().collect(),
    });
    let r = match resolved {
        Ok(r) => r,
        Err(e) => {
            // A broken .pgx.toml is an opted-in project whose setup is wrong:
            // fail closed on start, stay quiet otherwise.
            return Ok(match input.event {
                Event::Start => emit_failure(input.host, &env, true, &e.message),
                _ => {
                    cursor_ack(input.host);
                    ExitCode::Ok
                }
            });
        }
    };
    match input.event {
        Event::Start => match session::up(&out.quiet(), &r) {
            Ok(res) => {
                emit_start(input.host, &env, &res);
                Ok(ExitCode::Ok)
            }
            Err(e) => {
                let mut reason = e.message.clone();
                if let Some(h) = &e.hint {
                    reason.push_str(&format!(" ({h})"));
                }
                Ok(emit_failure(
                    input.host,
                    &env,
                    r.isolation_enabled(),
                    &reason,
                ))
            }
        },
        Event::Activity => {
            if let Err(e) = session::touch(&r) {
                notice(&e.message);
            }
            cursor_ack(input.host);
            Ok(ExitCode::Ok)
        }
        Event::End => {
            if let Err(e) = session::down(&r, false) {
                notice(&e.message);
            }
            Ok(ExitCode::Ok)
        }
        Event::Ignore => {
            cursor_ack(input.host);
            Ok(ExitCode::Ok)
        }
    }
}
