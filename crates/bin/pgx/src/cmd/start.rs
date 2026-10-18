//! `pgx start` / `pgx restart`.
use super::Ctx;
use crate::cli::ExitCode;
use crate::cmd::init::port_check;
use crate::errors::{PgxError, Result};
use crate::out::Out;
use crate::{conf, fsdetect, process};
use std::os::fd::AsRawFd;
use std::time::Duration;

/// Serialises concurrent `pgx start`s on one home: the second waits for the
/// first and then finds the engine running.
pub struct StartLock(#[allow(dead_code)] std::fs::File);

impl StartLock {
    pub fn acquire(ctx: &Ctx) -> Result<StartLock> {
        let path = ctx.home.root.join("start.lock");
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
        Ok(StartLock(f))
    }
}

pub fn run(out: &Out) -> Result<ExitCode> {
    let ctx = Ctx::load()?;
    let _lock = StartLock::acquire(&ctx)?;
    start_locked(out, &ctx, false)
}

pub fn restart(out: &Out) -> Result<ExitCode> {
    let ctx = Ctx::load()?;
    let _lock = StartLock::acquire(&ctx)?;
    if let Some(pid) = ctx.running_pid() {
        out.note(&format!("stopping pid {pid}"));
        process::stop(pid, process::StopMode::Fast, Duration::from_secs(60), None)?;
    }
    start_locked(out, &ctx, true)
}

fn start_locked(out: &Out, ctx: &Ctx, restarted: bool) -> Result<ExitCode> {
    let sock_dir = ctx.sock_dir();
    if let Some(pid) = ctx.running_pid() {
        out.emit(
            &format!("PGX is already running (pid {pid}) on port {}", ctx.config.port),
            serde_json::json!({"started": false, "already_running": true, "pid": pid, "port": ctx.config.port, "socket_dir": sock_dir}),
        );
        return Ok(ExitCode::Ok);
    }
    let pid_file = ctx.home.data.join("postmaster.pid");
    if pid_file.exists() {
        out.note("removing a stale postmaster.pid");
        std::fs::remove_file(&pid_file)
            .map_err(|e| PgxError::io("cannot remove", &pid_file, &e))?;
    }
    // Retry the probe briefly: a genuine conflict persists, but a socket lingering
    // from an engine we just stopped or killed clears within the kernel's cleanup
    // window (the engine itself binds with SO_REUSEADDR and would not hit it).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match port_check(ctx.config.port) {
            Ok(()) => break,
            Err(why) => {
                if std::time::Instant::now() >= deadline {
                    return Err(PgxError::new(ExitCode::Conflict, format!("port {} is in use ({why})", ctx.config.port))
                        .hint("stop the other process, or re-initialise on another port with `pgx init --force --port N`"));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    // pgx.conf is regenerated from config.json on every start.
    let block_clone = fsdetect::supports_block_clone(&ctx.home.data);
    let pgx_conf = ctx.home.data.join("pgx.conf");
    std::fs::write(
        &pgx_conf,
        conf::render_pgx_conf(&ctx.config, &sock_dir, block_clone),
    )
    .map_err(|e| PgxError::io("cannot write", &pgx_conf, &e))?;

    let mut child = process::spawn(&ctx.install, &ctx.home, &ctx.config, &sock_dir, &[])?;
    let pid = child.id();
    match process::wait_ready(
        &ctx.home,
        &sock_dir,
        ctx.config.port,
        Some(&mut child),
        Duration::from_secs(60),
    ) {
        Ok(client) => client.close(),
        Err(e) => {
            let _ = process::stop(
                pid,
                process::StopMode::Force,
                Duration::from_secs(10),
                Some(&mut child),
            );
            let _ = std::fs::remove_file(&pid_file);
            return Err(e);
        }
    }
    // The child lives on in its own session; the CLI exits without reaping it.
    let url = format!(
        "postgres://postgres:***@127.0.0.1:{}/postgres",
        ctx.config.port
    );
    out.emit(
        &format!(
            "{} PGX (pid {pid}) on port {}\n  socket  {}\n  url     {url}   (`pgx url <database>` shows the password)\n  log     {}",
            if restarted { "restarted" } else { "started" },
            ctx.config.port,
            sock_dir.display(),
            ctx.home.log_file.display()
        ),
        serde_json::json!({"started": true, "already_running": false, "pid": pid, "port": ctx.config.port, "socket_dir": sock_dir, "log": ctx.home.log_file}),
    );
    Ok(ExitCode::Ok)
}

/// `pgx serve`: run the engine in the FOREGROUND, replacing this process, so it
/// can be PID 1 in a container or a systemd simple service. Initialises first
/// if $PGX_HOME is empty, reading PGX_PORT / PGX_DURABILITY / PGX_LISTEN like
/// the Docker entrypoint expects.
pub fn serve(out: &Out) -> Result<ExitCode> {
    use crate::layout::Home;
    let home = Home::resolve();
    if !home.is_initialized() {
        let port = std::env::var("PGX_PORT").ok().and_then(|p| p.parse().ok());
        let durability = std::env::var("PGX_DURABILITY").ok();
        out.note("pgx serve: initialising a new cluster");
        crate::cmd::init::run(out, port, durability, false)?;
    }
    let ctx = Ctx::load()?;
    if let Some(pid) = ctx.running_pid() {
        return Err(PgxError::new(
            ExitCode::Conflict,
            format!("PGX is already running (pid {pid})"),
        )
        .hint("`pgx serve` runs the engine in the foreground; stop the background runtime first"));
    }
    let sock_dir = ctx.sock_dir();
    let pid_file = ctx.home.data.join("postmaster.pid");
    if pid_file.exists() {
        let _ = std::fs::remove_file(&pid_file);
    }
    let block_clone = fsdetect::supports_block_clone(&ctx.home.data);
    let pgx_conf = ctx.home.data.join("pgx.conf");
    std::fs::write(
        &pgx_conf,
        conf::render_pgx_conf(&ctx.config, &sock_dir, block_clone),
    )
    .map_err(|e| PgxError::io("cannot write", &pgx_conf, &e))?;
    std::fs::create_dir_all(&sock_dir).map_err(|e| PgxError::io("cannot create", &sock_dir, &e))?;
    out.note(&format!(
        "pgx serve: exec engine on port {} (socket {})",
        ctx.config.port,
        sock_dir.display()
    ));
    exec_engine_foreground(&ctx.install, &ctx.home, &ctx.config, &sock_dir)
}

/// Replaces this process with the engine (execvp). Returns only on failure.
fn exec_engine_foreground(
    install: &crate::layout::Install,
    home: &crate::layout::Home,
    cfg: &crate::config::Config,
    sock_dir: &std::path::Path,
) -> Result<ExitCode> {
    use std::os::unix::process::CommandExt;
    // Raise the stack soft limit for the engine, like process::spawn does.
    let mut rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut rl) } == 0 {
        let want = crate::process::STACK_BYTES as libc::rlim_t;
        if rl.rlim_cur < want && (rl.rlim_max == libc::RLIM_INFINITY || rl.rlim_max >= want) {
            rl.rlim_cur = want;
            unsafe { libc::setrlimit(libc::RLIMIT_STACK, &rl) };
        }
    }
    let err = std::process::Command::new(&install.engine)
        .args(crate::process::engine_args(home, cfg, sock_dir))
        .envs(crate::process::engine_env(install))
        .exec();
    Err(PgxError::failure(format!(
        "cannot exec the engine {}: {err}",
        install.engine.display()
    )))
}
