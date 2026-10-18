//! Black-box harness: a temporary installation prefix (the engine and share
//! trees of an assembled distribution, plus the `pgx` binary under test) and a
//! temporary `PGX_HOME`. Every test gets its own; the engine is stopped and
//! the directories removed on drop, even when the test panics.
//!
//! Needs `PGX_DIST=<assembled prefix>` (see dist/assemble.sh). Without it the
//! harness fails loudly: these suites are release gates, never silent skips.
#![allow(dead_code)]
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Pgx {
    pub prefix: PathBuf,
    pub home: PathBuf,
    pub port: u16,
}

pub fn dist_prefix() -> PathBuf {
    let p = std::env::var("PGX_DIST").unwrap_or_else(|_| {
        panic!("PGX_DIST is not set: point it at an assembled distribution prefix (dist/assemble.sh --out ...)")
    });
    let p = PathBuf::from(p);
    for rel in [
        "bin/postgres",
        "share/timezone",
        "share/pgx/base-cluster/PG_VERSION",
        "share/pgx/dist.json",
    ] {
        assert!(p.join(rel).exists(), "PGX_DIST={} lacks {rel}", p.display());
    }
    p
}

pub fn free_port() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    l.local_addr().unwrap().port()
}

pub fn scratch_root() -> PathBuf {
    // Short: the Unix socket path must stay under 103 bytes.
    let root = std::env::var("PGX_TEST_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/pgxt"));
    std::fs::create_dir_all(&root).unwrap();
    root
}

impl Pgx {
    /// A fresh harness: prefix and home created, nothing initialised yet.
    pub fn new() -> Pgx {
        let dist = dist_prefix();
        let id = format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let base = scratch_root().join(id);
        let prefix = base.join("p");
        let home = base.join("h");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_pgx"), prefix.join("bin/pgx")).unwrap();
        std::os::unix::fs::symlink(dist.join("bin/postgres"), prefix.join("bin/postgres")).unwrap();
        std::os::unix::fs::symlink(dist.join("share"), prefix.join("share")).unwrap();
        Pgx {
            prefix,
            home,
            port: free_port(),
        }
    }

    pub fn with_home(home: PathBuf) -> Pgx {
        let mut p = Pgx::new();
        std::fs::create_dir_all(&home).unwrap();
        p.home = home;
        p
    }

    pub fn bin(&self) -> PathBuf {
        self.prefix.join("bin/pgx")
    }

    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(self.bin());
        c.args(args)
            .env("PGX_HOME", &self.home)
            .env_remove("PGPORT")
            .env_remove("PGHOST");
        c
    }

    pub fn run(&self, args: &[&str]) -> Run {
        let out = self.cmd(args).output().expect("spawn pgx");
        Run::from(out)
    }

    pub fn init(&self) -> Run {
        let port = self.port.to_string();
        self.run(&["init", "--port", &port])
    }

    /// `pgx run [extra...]` with PGX_PORT pinned to this harness's free port, so
    /// a fresh run initialises there instead of scanning 5432-5500 (and colliding
    /// with a real PostgreSQL or a sibling test).
    pub fn run_cmd(&self, extra: &[&str]) -> Run {
        let mut args = vec!["run"];
        args.extend_from_slice(extra);
        let out = self
            .cmd(&args)
            .env("PGX_PORT", self.port.to_string())
            .output()
            .expect("spawn pgx run");
        Run::from(out)
    }

    /// Spawn `pgx run` without waiting — for concurrency tests.
    pub fn spawn_run(&self) -> std::process::Child {
        self.cmd(&["run"])
            .env("PGX_PORT", self.port.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn pgx run")
    }

    pub fn init_ok(&self) {
        let r = self.init();
        assert_eq!(r.code, 0, "pgx init failed: {}", r.all());
    }

    pub fn start_ok(&self) {
        let r = self.run(&["start"]);
        assert_eq!(
            r.code,
            0,
            "pgx start failed: {}\n{}",
            r.all(),
            self.log_tail()
        );
    }

    pub fn stop_ok(&self) {
        let r = self.run(&["stop"]);
        assert_eq!(r.code, 0, "pgx stop failed: {}", r.all());
    }

    pub fn log_tail(&self) -> String {
        std::fs::read_to_string(self.home.join("log/pgx.log"))
            .map(|s| {
                s.lines()
                    .rev()
                    .take(25)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    pub fn config(&self) -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.home.join("config.json")).expect("config.json"),
        )
        .unwrap()
    }

    pub fn password(&self) -> String {
        self.config()["password"].as_str().unwrap().to_string()
    }

    pub fn sock_dir(&self) -> PathBuf {
        self.config()["run_dir"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join("run"))
    }

    /// Any `postgres` process whose command line names this home's data dir.
    pub fn engine_pids(&self) -> Vec<u32> {
        let out = Command::new("pgrep")
            .args([
                "-f",
                "--",
                &format!("postgres .*-D {}( |$)", self.home.join("data").display()),
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    /// Runs SQL through psql if available, else through `pgx`'s own client via a tiny helper.
    pub fn psql(&self, db: &str, sql: &str) -> Run {
        let psql = find_psql();
        let out = Command::new(psql)
            .args([
                "-X",
                "-q",
                "-v",
                "ON_ERROR_STOP=1",
                "-h",
                &self.sock_dir().to_string_lossy(),
                "-p",
                &self.port.to_string(),
                "-U",
                "postgres",
                "-d",
                db,
                "-Atc",
                sql,
            ])
            .output()
            .expect("spawn psql");
        Run::from(out)
    }

    pub fn psql_tcp(&self, db: &str, password: &str, sql: &str) -> Run {
        let psql = find_psql();
        let out = Command::new(psql)
            .args([
                "-X",
                "-q",
                "-v",
                "ON_ERROR_STOP=1",
                "-h",
                "127.0.0.1",
                "-p",
                &self.port.to_string(),
                "-U",
                "postgres",
                "-d",
                db,
                "-Atc",
                sql,
            ])
            .env("PGPASSWORD", password)
            .output()
            .expect("spawn psql");
        Run::from(out)
    }
}

pub fn find_psql() -> PathBuf {
    if let Ok(p) = std::env::var("PGX_TEST_PSQL") {
        return PathBuf::from(p);
    }
    for cand in [
        "/opt/homebrew/opt/postgresql@18/bin/psql",
        "/usr/lib/postgresql/18/bin/psql",
        "/usr/local/pgsql/bin/psql",
    ] {
        if Path::new(cand).exists() {
            return PathBuf::from(cand);
        }
    }
    PathBuf::from("psql")
}

impl Drop for Pgx {
    fn drop(&mut self) {
        let _ = self.cmd(&["stop", "--force"]).output();
        for pid in self.engine_pids() {
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
        if std::env::var("PGX_TEST_KEEP").is_err() {
            if let Some(base) = self.prefix.parent() {
                let _ = std::fs::remove_dir_all(base);
            }
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
}

#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl From<Output> for Run {
    fn from(o: Output) -> Run {
        Run {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Run {
    pub fn all(&self) -> String {
        format!(
            "exit {}\nstdout: {}\nstderr: {}",
            self.code, self.stdout, self.stderr
        )
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.all()))
    }
}
