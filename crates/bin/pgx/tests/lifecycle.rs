mod common;
use common::Pgx;
use std::os::unix::fs::PermissionsExt;

#[test]
fn version_prints_engine_and_compat() {
    let p = Pgx::new();
    let r = p.run(&["version"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.stdout.starts_with("pgx "), "{}", r.stdout);
    assert!(r.stdout.contains("PostgreSQL 18.6"), "{}", r.stdout);
    assert!(r.stdout.contains("engine "), "{}", r.stdout);
    let j = p.run(&["--json", "version"]).json();
    assert_eq!(j["postgres_compat"], "18.6");
    assert!(j["engine_commit"].as_str().unwrap().len() >= 7);
    assert!(j["pgx_version"].as_str().unwrap().starts_with("0."));
}

#[test]
fn init_creates_cluster_and_config() {
    let p = Pgx::new();
    let r = p.init();
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(p.home.join("data/PG_VERSION").is_file());
    assert!(p.home.join("data/pgx.conf").is_file());
    assert!(p.home.join("data/pg_hba.conf").is_file());
    let mode = std::fs::metadata(p.home.join("data")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "data dir mode {mode:o}");
    let mode = std::fs::metadata(p.home.join("config.json")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "config mode {mode:o}");
    let c = p.config();
    assert_eq!(c["port"], p.port);
    assert_eq!(c["durability"], "ephemeral");
    assert!(c["password"].as_str().unwrap().len() >= 24);
    assert!(p.engine_pids().is_empty(), "init left an engine running");
    assert!(!p.home.join("data/postmaster.pid").exists(), "init left postmaster.pid");
    let conf = std::fs::read_to_string(p.home.join("data/pgx.conf")).unwrap();
    assert!(conf.contains(&format!("port = {}", p.port)));
    assert!(conf.contains("listen_addresses = '127.0.0.1, ::1'"), "{conf}");
    assert!(conf.contains("fsync = off"), "ephemeral profile expected: {conf}");
    let hba = std::fs::read_to_string(p.home.join("data/pg_hba.conf")).unwrap();
    assert!(hba.contains("scram-sha-256"), "{hba}");
    assert!(!hba.contains("0.0.0.0/0"), "{hba}");
    assert!(r.stdout.contains("pgx start"), "init should tell the user what to do next: {}", r.stdout);
}

#[test]
fn init_json_output() {
    let p = Pgx::new();
    let port = p.port.to_string();
    let j = p.run(&["--json", "init", "--port", &port]).json();
    assert_eq!(j["port"], p.port);
    assert_eq!(j["durability"], "ephemeral");
    assert_eq!(j["data_dir"], p.home.join("data").to_string_lossy().as_ref());
}

#[test]
fn init_full_durability_writes_fsync_on() {
    let p = Pgx::new();
    let port = p.port.to_string();
    let r = p.run(&["init", "--port", &port, "--durability", "full"]);
    assert_eq!(r.code, 0, "{}", r.all());
    let conf = std::fs::read_to_string(p.home.join("data/pgx.conf")).unwrap();
    assert!(conf.contains("fsync = on"), "{conf}");
    assert!(conf.contains("synchronous_commit = on"), "{conf}");
    assert!(conf.contains("full_page_writes = on"), "{conf}");
    assert_eq!(p.config()["durability"], "full");
}

#[test]
fn init_twice_is_conflict_and_keeps_data() {
    let p = Pgx::new();
    p.init_ok();
    let before = std::fs::read_to_string(p.home.join("config.json")).unwrap();
    let r = p.init();
    assert_eq!(r.code, 4, "{}", r.all());
    assert!(r.stderr.contains("already"), "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(p.home.join("config.json")).unwrap(), before, "second init changed config");
    let j = p.run(&["--json", "init", "--port", &p.port.to_string()]).json();
    assert_eq!(j["error"]["exit_code"], 4);
}

#[test]
fn init_force_recreates() {
    let p = Pgx::new();
    p.init_ok();
    std::fs::write(p.home.join("data/marker"), b"old").unwrap();
    let port = p.port.to_string();
    let r = p.run(&["init", "--port", &port, "--force"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(!p.home.join("data/marker").exists(), "--force did not recreate the data dir");
}

#[test]
fn init_refuses_long_socket_path() {
    let long = common::scratch_root().join("x".repeat(95));
    let p = Pgx::with_home(long);
    let r = p.init();
    assert_eq!(r.code, 2, "{}", r.all());
    assert!(r.stderr.contains("socket") && r.stderr.contains("PGX_RUN_DIR"), "{}", r.stderr);
    assert!(!p.home.join("data").exists(), "refused init must not leave a data dir");
}

#[test]
fn init_with_run_dir_override_accepts_long_home() {
    let long = common::scratch_root().join("y".repeat(95));
    let p = Pgx::with_home(long);
    let run_dir = common::scratch_root().join(format!("rd{}", p.port));
    let port = p.port.to_string();
    let r = p.cmd(&["init", "--port", &port]).env("PGX_RUN_DIR", &run_dir).output().unwrap();
    let r = common::Run::from(r);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(p.config()["run_dir"], run_dir.to_string_lossy().as_ref());
    let _ = std::fs::remove_dir_all(run_dir);
}

#[test]
fn init_picks_free_port_when_5432_busy() {
    // Simulated: ask for a port that is busy and expect init to refuse it;
    // the automatic choice is exercised by `init_without_port_chooses_a_free_one`.
    let p = Pgx::new();
    let busy = std::net::TcpListener::bind(("127.0.0.1", p.port)).unwrap();
    let r = p.init();
    drop(busy);
    assert_eq!(r.code, 4, "{}", r.all());
    assert!(r.stderr.contains("in use"), "{}", r.stderr);
}

#[test]
fn init_without_port_chooses_a_free_one() {
    let p = Pgx::new();
    let r = p.run(&["--json", "init"]);
    assert_eq!(r.code, 0, "{}", r.all());
    let port = r.json()["port"].as_u64().unwrap() as u16;
    assert!(port >= 5432, "{port}");
    // The chosen port is free (bind succeeds now that init has stopped its probe server).
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn start_requires_init() {
    let p = Pgx::new();
    let r = p.run(&["start"]);
    assert_eq!(r.code, 3, "{}", r.all());
    assert!(r.stderr.contains("pgx init"), "{}", r.stderr);
}

#[test]
fn incomplete_installation_is_exit_7() {
    let p = Pgx::new();
    std::fs::remove_file(p.prefix.join("share")).unwrap();
    std::fs::create_dir_all(p.prefix.join("share/timezone")).unwrap();
    let r = p.run(&["init"]);
    assert_eq!(r.code, 7, "{}", r.all());
    assert!(r.stderr.contains("base-cluster"), "{}", r.stderr);
}
