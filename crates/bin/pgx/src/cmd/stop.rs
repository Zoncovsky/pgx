//! `pgx stop [--force]`.
use super::Ctx;
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::out::Out;
use crate::process;
use std::time::Duration;

pub fn run(out: &Out, force: bool) -> Result<ExitCode> {
    let ctx = Ctx::load()?;
    let pid_file = ctx.home.data.join("postmaster.pid");
    let Some(pid) = ctx.running_pid() else {
        if pid_file.exists() {
            out.note("removing a stale postmaster.pid");
            let _ = std::fs::remove_file(&pid_file);
        }
        out.emit("PGX is not running", serde_json::json!({"stopped": false, "was_running": false}));
        return Ok(ExitCode::Ok);
    };
    let mode = if force { process::StopMode::Force } else { process::StopMode::Fast };
    process::stop(pid, mode, Duration::from_secs(60), None)?;
    if pid_file.exists() {
        // A killed engine cannot remove its own pid file.
        std::fs::remove_file(&pid_file).map_err(|e| PgxError::io("cannot remove", &pid_file, &e))?;
    }
    out.emit(&format!("stopped PGX (pid {pid})"), serde_json::json!({"stopped": true, "was_running": true, "pid": pid, "forced": force}));
    Ok(ExitCode::Ok)
}
