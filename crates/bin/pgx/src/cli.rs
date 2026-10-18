//! Argument parsing, help text and the stable exit codes.
use crate::errors::{PgxError, Result};

/// Exit codes are part of the agent-facing interface (docs/pgx/local-dev/cli.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Ok = 0,
    /// engine or runtime error
    Failure = 1,
    Usage = 2,
    /// not initialised, or not running
    NotReady = 3,
    /// already exists, already running, port in use
    Conflict = 4,
    NotFound = 5,
    /// sessions hold the database
    InUse = 6,
    /// corrupted or incomplete installation
    Corrupt = 7,
}

impl ExitCode {
    pub fn name(self) -> &'static str {
        match self {
            ExitCode::Ok => "ok",
            ExitCode::Failure => "failure",
            ExitCode::Usage => "usage",
            ExitCode::NotReady => "not_ready",
            ExitCode::Conflict => "conflict",
            ExitCode::NotFound => "not_found",
            ExitCode::InUse => "in_use",
            ExitCode::Corrupt => "corrupt",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Global {
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Init {
        port: Option<u16>,
        durability: Option<String>,
        force: bool,
    },
    Start,
    /// Run the engine in the foreground (init if needed); the Docker/systemd entrypoint.
    Serve,
    Stop {
        force: bool,
    },
    Restart,
    Status,
    Create {
        name: String,
    },
    Drop {
        name: String,
        force: bool,
    },
    Clone {
        source: String,
        target: String,
    },
    Ls,
    Url {
        name: String,
        socket: bool,
    },
    Doctor,
    Version,
    Help,
}

pub fn parse(args: &[String]) -> Result<(Global, Command)> {
    let mut global = Global::default();
    let mut rest: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => global.json = true,
            "--help" | "-h" => return Ok((global, Command::Help)),
            "--version" | "-V" => return Ok((global, Command::Version)),
            other => rest.push(other),
        }
    }
    let Some((&name, tail)) = rest.split_first() else {
        return Ok((global, Command::Help));
    };
    let mut positional: Vec<&str> = Vec::new();
    let mut flags: Vec<(&str, Option<&str>)> = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        let t = tail[i];
        if let Some(flag) = t.strip_prefix("--") {
            if let Some((k, v)) = flag.split_once('=') {
                flags.push((k, Some(v)));
            } else if matches!(flag, "port" | "durability") {
                let v = tail
                    .get(i + 1)
                    .copied()
                    .ok_or_else(|| PgxError::usage(format!("--{flag} needs a value")))?;
                flags.push((flag, Some(v)));
                i += 1;
            } else {
                flags.push((flag, None));
            }
        } else {
            positional.push(t);
        }
        i += 1;
    }
    let only_flags = |allowed: &[&str]| -> Result<()> {
        for (k, _) in &flags {
            if !allowed.contains(k) {
                return Err(PgxError::usage(format!("pgx {name}: unknown option --{k}")));
            }
        }
        Ok(())
    };
    let has = |k: &str| flags.iter().any(|(f, _)| *f == k);
    let value = |k: &str| flags.iter().find(|(f, _)| *f == k).and_then(|(_, v)| *v);
    let arity = |n: usize, usage: &str| -> Result<()> {
        if positional.len() != n {
            return Err(PgxError::usage(format!("usage: pgx {usage}")));
        }
        Ok(())
    };
    let cmd = match name {
        "init" => {
            only_flags(&["port", "durability", "force"])?;
            arity(0, "init [--port N] [--durability ephemeral|full] [--force]")?;
            let port =
                match value("port") {
                    None => None,
                    Some(p) => Some(p.parse::<u16>().ok().filter(|p| *p >= 1024).ok_or_else(
                        || {
                            PgxError::usage(format!(
                                "--port must be a number between 1024 and 65535, got {p:?}"
                            ))
                        },
                    )?),
                };
            Command::Init {
                port,
                durability: value("durability").map(String::from),
                force: has("force"),
            }
        }
        "start" => {
            only_flags(&[])?;
            arity(0, "start")?;
            Command::Start
        }
        "serve" => {
            only_flags(&[])?;
            arity(0, "serve")?;
            Command::Serve
        }
        "stop" => {
            only_flags(&["force"])?;
            arity(0, "stop [--force]")?;
            Command::Stop {
                force: has("force"),
            }
        }
        "restart" => {
            only_flags(&[])?;
            arity(0, "restart")?;
            Command::Restart
        }
        "status" => {
            only_flags(&[])?;
            arity(0, "status")?;
            Command::Status
        }
        "create" => {
            only_flags(&[])?;
            arity(1, "create <database>")?;
            Command::Create {
                name: positional[0].into(),
            }
        }
        "drop" => {
            only_flags(&["force"])?;
            arity(1, "drop <database> [--force]")?;
            Command::Drop {
                name: positional[0].into(),
                force: has("force"),
            }
        }
        "clone" => {
            only_flags(&[])?;
            arity(2, "clone <source> <target>")?;
            Command::Clone {
                source: positional[0].into(),
                target: positional[1].into(),
            }
        }
        "ls" | "list" => {
            only_flags(&[])?;
            arity(0, "ls")?;
            Command::Ls
        }
        "url" => {
            only_flags(&["socket"])?;
            arity(1, "url <database> [--socket]")?;
            Command::Url {
                name: positional[0].into(),
                socket: has("socket"),
            }
        }
        "doctor" => {
            only_flags(&[])?;
            arity(0, "doctor")?;
            Command::Doctor
        }
        "version" => {
            only_flags(&[])?;
            arity(0, "version")?;
            Command::Version
        }
        "help" => Command::Help,
        other => return Err(PgxError::usage(format!("unknown command {other:?}"))),
    };
    Ok((global, cmd))
}

pub fn help_text() -> String {
    format!(
        "pgx {} — a local, disposable, PostgreSQL-compatible runtime

Usage: pgx [--json] <command> [arguments]

Runtime
  pgx init [--port N] [--durability ephemeral|full] [--force]
                          create the cluster under $PGX_HOME (default ~/.pgx)
  pgx start               start the runtime (idempotent)
  pgx serve               run the engine in the foreground (init if needed; for Docker/systemd)
  pgx stop [--force]      stop it (fast shutdown; --force kills after the timeout)
  pgx restart
  pgx status              running?, pid, port, databases, memory

Databases
  pgx create <database>
  pgx drop <database> [--force]     --force terminates connected sessions first
  pgx clone <source> <target>       copy-on-write where the filesystem supports it
  pgx ls
  pgx url <database> [--socket]     connection URL (TCP with password, or Unix socket)

Tools
  pgx doctor              check the installation, data directory, port, storage
  pgx version
  pgx help

Options
  --json                  machine-readable output on stdout (also for errors)

Environment
  PGX_HOME                data home (default ~/.pgx)

Exit codes
  0 ok · 1 engine or runtime error · 2 usage · 3 not initialised / not running
  4 conflict (exists, running, port in use) · 5 not found · 6 database in use
  7 corrupted or incomplete installation
",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(ExitCode::Ok as i32, 0);
        assert_eq!(ExitCode::Failure as i32, 1);
        assert_eq!(ExitCode::Usage as i32, 2);
        assert_eq!(ExitCode::NotReady as i32, 3);
        assert_eq!(ExitCode::Conflict as i32, 4);
        assert_eq!(ExitCode::NotFound as i32, 5);
        assert_eq!(ExitCode::InUse as i32, 6);
        assert_eq!(ExitCode::Corrupt as i32, 7);
    }

    #[test]
    fn parse_global_json_flag() {
        let (g, c) = parse(&args("--json status")).unwrap();
        assert!(g.json);
        assert_eq!(c, Command::Status);
        let (g, c) = parse(&args("status --json")).unwrap();
        assert!(g.json);
        assert_eq!(c, Command::Status);
    }

    #[test]
    fn parse_subcommands_with_arguments() {
        assert_eq!(
            parse(&args("create myapp")).unwrap().1,
            Command::Create {
                name: "myapp".into()
            }
        );
        assert_eq!(
            parse(&args("drop myapp --force")).unwrap().1,
            Command::Drop {
                name: "myapp".into(),
                force: true
            }
        );
        assert_eq!(
            parse(&args("drop myapp")).unwrap().1,
            Command::Drop {
                name: "myapp".into(),
                force: false
            }
        );
        assert_eq!(
            parse(&args("clone tpl t1")).unwrap().1,
            Command::Clone {
                source: "tpl".into(),
                target: "t1".into()
            }
        );
        assert_eq!(
            parse(&args("url myapp")).unwrap().1,
            Command::Url {
                name: "myapp".into(),
                socket: false
            }
        );
        assert_eq!(
            parse(&args("url myapp --socket")).unwrap().1,
            Command::Url {
                name: "myapp".into(),
                socket: true
            }
        );
        assert_eq!(
            parse(&args("init --port 5555")).unwrap().1,
            Command::Init {
                port: Some(5555),
                durability: None,
                force: false
            }
        );
        assert_eq!(
            parse(&args("init --durability full --force")).unwrap().1,
            Command::Init {
                port: None,
                durability: Some("full".into()),
                force: true
            }
        );
        assert_eq!(
            parse(&args("stop --force")).unwrap().1,
            Command::Stop { force: true }
        );
        assert_eq!(parse(&args("serve")).unwrap().1, Command::Serve);
        assert_eq!(parse(&args("ls")).unwrap().1, Command::Ls);
        assert_eq!(parse(&args("doctor")).unwrap().1, Command::Doctor);
        assert_eq!(parse(&args("version")).unwrap().1, Command::Version);
        assert_eq!(parse(&args("--version")).unwrap().1, Command::Version);
        assert_eq!(parse(&args("help")).unwrap().1, Command::Help);
        assert_eq!(parse(&args("--help")).unwrap().1, Command::Help);
        assert_eq!(parse(&args("")).unwrap().1, Command::Help);
    }

    #[test]
    fn unknown_subcommand_is_usage_error() {
        let e = parse(&args("frobnicate")).unwrap_err();
        assert_eq!(e.code, ExitCode::Usage);
        assert!(e.message.contains("frobnicate"), "{}", e.message);
    }

    #[test]
    fn missing_and_extra_arguments_are_usage_errors() {
        assert_eq!(parse(&args("create")).unwrap_err().code, ExitCode::Usage);
        assert_eq!(
            parse(&args("create a b")).unwrap_err().code,
            ExitCode::Usage
        );
        assert_eq!(parse(&args("clone a")).unwrap_err().code, ExitCode::Usage);
        assert_eq!(
            parse(&args("init --port notanumber")).unwrap_err().code,
            ExitCode::Usage
        );
        assert_eq!(
            parse(&args("init --port 70000")).unwrap_err().code,
            ExitCode::Usage
        );
        assert_eq!(
            parse(&args("status --bogus")).unwrap_err().code,
            ExitCode::Usage
        );
    }

    #[test]
    fn help_mentions_every_command() {
        let h = help_text();
        for c in [
            "init", "start", "stop", "restart", "status", "create", "drop", "clone", "ls", "url",
            "doctor", "version", "help",
        ] {
            assert!(
                h.contains(&format!("pgx {c}")) || h.contains(&format!("  {c}")),
                "help lacks {c}"
            );
        }
        assert!(h.contains("--json"));
        assert!(h.contains("Exit codes"));
    }
}
