//! `pgx agent exec -- <command>`: a branch for the lifetime of one process.
//! The universal adapter: Codex, any CLI agent, a test suite, a script. The
//! wrapper is the parent of the command, so it can set the command's
//! environment without touching anyone else's.
use super::session::{self, ResolveInput};
use super::ExecOpts;
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::out::Out;
use serde_json::json;
use std::io::Write;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

static GOT_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    GOT_SIGNAL.store(sig, Ordering::SeqCst);
}

fn install_handlers() {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: a plain handler that stores an integer.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// A host label from the command's basename: `codex`, `agent`, `sh`, …
fn host_from_command(cmd: &str) -> String {
    let base = std::path::Path::new(cmd)
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(16)
        .collect();
    match super::identity::validate_host(&cleaned) {
        Ok(h) => h,
        Err(_) => "exec".to_string(),
    }
}

fn random_id() -> String {
    let mut buf = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut buf);
    } else {
        let t = crate::fsutil::now_epoch() ^ (std::process::id() as u64);
        buf = t.to_le_bytes();
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run(out: &Out, opts: ExecOpts) -> Result<ExitCode> {
    let program = opts.command[0].clone();
    // Attach to the surrounding agent session when one is visible, otherwise
    // this run is its own session.
    let attached = opts.id.is_none()
        && ["CLAUDE_CODE_SESSION_ID", "CODEX_SESSION_ID"]
            .iter()
            .any(|v| std::env::var(v).is_ok_and(|s| !s.is_empty()));
    let (id, host) = if attached {
        (None, opts.host.clone())
    } else {
        (
            Some(opts.id.clone().unwrap_or_else(random_id)),
            Some(
                opts.host
                    .clone()
                    .unwrap_or_else(|| host_from_command(&program)),
            ),
        )
    };
    let r = session::resolve(&ResolveInput {
        id,
        host,
        source: opts.source.clone(),
        ttl: opts.ttl.clone(),
        ..Default::default()
    })?;
    let res = session::up(&out.quiet(), &r)?;

    let mut cmd = std::process::Command::new(&program);
    cmd.args(&opts.command[1..]);
    for (k, v) in session::env_pairs(&res) {
        cmd.env(k, v);
    }
    cmd.env("PGX_AGENT_SESSION", &r.key.session_id);
    install_handlers();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            cleanup(&r, attached, opts.keep);
            return Err(PgxError::failure(format!("cannot run {program}: {e}")));
        }
    };

    // Wait, forwarding signals and keeping the idle TTL alive.
    // Beat well inside the TTL: a quarter of it (at least 30 s) for normal
    // TTLs, half of it for very short ones, so the supervisor never removes
    // the branch under a running command.
    let idle = r.idle_secs();
    let heartbeat = Duration::from_secs(if idle >= 120 {
        (idle / 4).max(30)
    } else {
        (idle / 2).max(1)
    });
    let mut last_beat = Instant::now();
    let mut forwarded = 0;
    let status = loop {
        if let Some(st) = child
            .try_wait()
            .map_err(|e| PgxError::failure(format!("wait: {e}")))?
        {
            break st;
        }
        let sig = GOT_SIGNAL.load(Ordering::SeqCst);
        if sig != 0 && sig != forwarded {
            // SAFETY: kill(2) on our own child.
            unsafe {
                libc::kill(child.id() as libc::pid_t, sig);
            }
            forwarded = sig;
        }
        if last_beat.elapsed() >= heartbeat {
            let _ = session::touch(&r);
            last_beat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    use std::os::unix::process::ExitStatusExt;
    let code = match (status.code(), status.signal()) {
        (Some(c), _) => c,
        (None, Some(s)) => 128 + s,
        _ => 1,
    };
    let outcome = cleanup(&r, attached, opts.keep);
    if out.json {
        println!(
            "{}",
            json!({
                "branch": res.branch.as_str(),
                "session_id": r.key.session_id,
                "host": r.key.host,
                "exit_code": code,
                "attached": attached,
                "dropped": outcome == Outcome::Dropped,
                "retired": outcome == Outcome::Retired,
            })
        );
    }
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Kept,
    Retired,
    Dropped,
}

/// After the command: leave an attached session alone, retire with `--keep`,
/// drop otherwise. Failures are reported, never fatal (the command's exit
/// status is what the caller is waiting for; the TTL is the backstop).
fn cleanup(r: &session::Resolved, attached: bool, keep: bool) -> Outcome {
    if attached {
        return Outcome::Kept;
    }
    match session::down(r, !keep) {
        Ok(d) if d.dropped => Outcome::Dropped,
        Ok(_) => Outcome::Retired,
        Err(e) => {
            eprintln!("pgx agent exec: could not retire the branch: {e}");
            Outcome::Kept
        }
    }
}
