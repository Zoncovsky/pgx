//! `pgx agent …`: one isolated database per coding-agent session.
//!
//! The deterministic half of the agent integration. The Skill in
//! `skills/pgx/` decides *when* a task needs a database and how to prepare a
//! template; these commands do the work that must be exact: session identity,
//! idempotent branch creation, idle TTL and grace, hook plumbing for Claude
//! Code, Cursor and Codex, a process wrapper, installation and diagnosis.
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::out::Out;
use std::path::PathBuf;

pub mod identity;
pub mod project;

/// Where hook configuration is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The project's shared file (`.claude/settings.json`, `.cursor/hooks.json`, `.codex/hooks.json`).
    Project,
    /// The per-developer project file (`.claude/settings.local.json`; the same as Project for hosts without one).
    Local,
    /// The user's home (`~/.claude/settings.json`, `~/.cursor/hooks.json`, `~/.codex/hooks.json`).
    User,
}

impl Scope {
    fn parse(s: &str) -> Result<Scope> {
        match s {
            "project" => Ok(Scope::Project),
            "local" => Ok(Scope::Local),
            "user" => Ok(Scope::User),
            other => Err(PgxError::usage(format!(
                "--scope must be project, local or user, got {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdOpts {
    pub id: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpOpts {
    pub id: Option<String>,
    pub host: Option<String>,
    pub source: Option<String>,
    pub ttl: Option<String>,
    /// Print `export` lines instead of the human block.
    pub env: bool,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownOpts {
    pub id: Option<String>,
    pub host: Option<String>,
    /// Drop immediately instead of retiring with a grace period.
    pub now: bool,
    pub grace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOpts {
    pub id: Option<String>,
    pub host: Option<String>,
    pub source: Option<String>,
    pub ttl: Option<String>,
    /// Retire with grace instead of dropping when the command exits.
    pub keep: bool,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOpts {
    pub host: Option<String>,
    pub source: Option<String>,
    pub ttl: Option<String>,
    pub grace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOpts {
    /// Empty means "the hosts detected on this machine".
    pub hosts: Vec<String>,
    pub scope: Scope,
    pub source: Option<String>,
    pub ttl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupOpts {
    pub source: Option<String>,
    pub hosts: Vec<String>,
    pub scope: Scope,
    pub ttl: Option<String>,
    pub no_hooks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCmd {
    Up(UpOpts),
    Touch(IdOpts),
    Down(DownOpts),
    Exec(ExecOpts),
    Env(IdOpts),
    Ls,
    Hook(HookOpts),
    Install(InstallOpts),
    Uninstall(InstallOpts),
    Setup(SetupOpts),
    Doctor,
}

const USAGE: &str =
    "usage: pgx agent <setup|doctor|exec|ls|up|touch|down|env|hook|install|uninstall> [options]";

const VALUE_FLAGS: [&str; 7] = ["id", "host", "source", "ttl", "grace", "hosts", "scope"];
const BOOL_FLAGS: [&str; 4] = ["env", "now", "keep", "no-hooks"];

/// Parse everything after `pgx agent`. `--` ends option parsing (for `exec`).
pub fn parse(tail: &[&str]) -> Result<AgentCmd> {
    let Some((&sub, rest)) = tail.split_first() else {
        return Err(PgxError::usage(USAGE));
    };
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let t = rest[i];
        if t == "--" {
            command = rest[i + 1..].iter().map(|s| s.to_string()).collect();
            break;
        }
        if let Some(flag) = t.strip_prefix("--") {
            if let Some((k, v)) = flag.split_once('=') {
                flags.push((k.to_string(), Some(v.to_string())));
            } else if VALUE_FLAGS.contains(&flag) {
                let v = rest
                    .get(i + 1)
                    .copied()
                    .ok_or_else(|| PgxError::usage(format!("--{flag} needs a value")))?;
                flags.push((flag.to_string(), Some(v.to_string())));
                i += 1;
            } else if BOOL_FLAGS.contains(&flag) {
                flags.push((flag.to_string(), None));
            } else {
                return Err(PgxError::usage(format!(
                    "pgx agent {sub}: unknown option --{flag}"
                )));
            }
        } else {
            positional.push(t.to_string());
        }
        i += 1;
    }
    let only = |allowed: &[&str]| -> Result<()> {
        for (k, _) in &flags {
            if !allowed.contains(&k.as_str()) {
                return Err(PgxError::usage(format!(
                    "pgx agent {sub}: unknown option --{k}"
                )));
            }
        }
        Ok(())
    };
    let value = |k: &str| -> Option<String> {
        flags
            .iter()
            .find(|(f, _)| f == k)
            .and_then(|(_, v)| v.clone())
    };
    let has = |k: &str| flags.iter().any(|(f, _)| f == k);
    let no_positional = |usage: &str| -> Result<()> {
        if positional.is_empty() {
            Ok(())
        } else {
            Err(PgxError::usage(format!("usage: pgx agent {usage}")))
        }
    };
    let hosts_list = |v: Option<String>| -> Vec<String> {
        v.map(|s| {
            s.split(',')
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
                .collect()
        })
        .unwrap_or_default()
    };
    let scope = match value("scope") {
        Some(s) => Scope::parse(&s)?,
        None => Scope::Project,
    };
    let cmd = match sub {
        "up" => {
            only(&["id", "host", "source", "ttl", "env"])?;
            no_positional(
                "up [--id <session>] [--host <h>] [--source <db>] [--ttl <dur>] [--env]",
            )?;
            AgentCmd::Up(UpOpts {
                id: value("id"),
                host: value("host"),
                source: value("source"),
                ttl: value("ttl"),
                env: has("env"),
                cwd: None,
            })
        }
        "touch" => {
            only(&["id", "host"])?;
            no_positional("touch [--id <session>] [--host <h>]")?;
            AgentCmd::Touch(IdOpts {
                id: value("id"),
                host: value("host"),
            })
        }
        "env" => {
            only(&["id", "host"])?;
            no_positional("env [--id <session>] [--host <h>]")?;
            AgentCmd::Env(IdOpts {
                id: value("id"),
                host: value("host"),
            })
        }
        "down" => {
            only(&["id", "host", "now", "grace"])?;
            no_positional("down [--id <session>] [--host <h>] [--now | --grace <dur>]")?;
            if has("now") && has("grace") {
                return Err(PgxError::usage(
                    "pgx agent down: --now and --grace exclude each other",
                ));
            }
            AgentCmd::Down(DownOpts {
                id: value("id"),
                host: value("host"),
                now: has("now"),
                grace: value("grace"),
            })
        }
        "exec" => {
            only(&["id", "host", "source", "ttl", "keep"])?;
            if !positional.is_empty() || command.is_empty() {
                return Err(PgxError::usage(
                    "usage: pgx agent exec [--source <db>] [--ttl <dur>] [--keep] -- <command> [args…]",
                ));
            }
            AgentCmd::Exec(ExecOpts {
                id: value("id"),
                host: value("host"),
                source: value("source"),
                ttl: value("ttl"),
                keep: has("keep"),
                command,
            })
        }
        "ls" | "list" => {
            only(&[])?;
            no_positional("ls")?;
            AgentCmd::Ls
        }
        "doctor" => {
            only(&[])?;
            no_positional("doctor")?;
            AgentCmd::Doctor
        }
        "hook" => {
            only(&["host", "source", "ttl", "grace"])?;
            no_positional(
                "hook [--host claude|cursor|codex] [--source <db>] [--ttl <dur>] [--grace <dur>]",
            )?;
            AgentCmd::Hook(HookOpts {
                host: value("host"),
                source: value("source"),
                ttl: value("ttl"),
                grace: value("grace"),
            })
        }
        "install" | "uninstall" => {
            only(&["scope", "source", "ttl", "hosts"])?;
            let mut hosts = positional.clone();
            hosts.extend(hosts_list(value("hosts")));
            let opts = InstallOpts {
                hosts,
                scope,
                source: value("source"),
                ttl: value("ttl"),
            };
            if sub == "install" {
                AgentCmd::Install(opts)
            } else {
                AgentCmd::Uninstall(opts)
            }
        }
        "setup" => {
            only(&["source", "hosts", "scope", "ttl", "no-hooks"])?;
            no_positional("setup [--source <db>] [--hosts claude,cursor,codex] [--scope project|local|user] [--ttl <dur>] [--no-hooks]")?;
            AgentCmd::Setup(SetupOpts {
                source: value("source"),
                hosts: hosts_list(value("hosts")),
                scope,
                ttl: value("ttl"),
                no_hooks: has("no-hooks"),
            })
        }
        other => {
            return Err(PgxError::usage(format!(
                "unknown agent subcommand {other:?}\n{USAGE}"
            )))
        }
    };
    Ok(cmd)
}

pub fn run(out: &Out, cmd: AgentCmd) -> Result<ExitCode> {
    let _ = out;
    let name = match cmd {
        AgentCmd::Up(_) => "up",
        AgentCmd::Touch(_) => "touch",
        AgentCmd::Down(_) => "down",
        AgentCmd::Exec(_) => "exec",
        AgentCmd::Env(_) => "env",
        AgentCmd::Ls => "ls",
        AgentCmd::Hook(_) => "hook",
        AgentCmd::Install(_) => "install",
        AgentCmd::Uninstall(_) => "uninstall",
        AgentCmd::Setup(_) => "setup",
        AgentCmd::Doctor => "doctor",
    };
    Err(PgxError::failure(format!(
        "pgx agent {name}: not implemented"
    )))
}
