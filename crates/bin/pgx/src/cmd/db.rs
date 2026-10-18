//! `pgx create | drop | clone | ls | url`.
use super::{db_error, Ctx};
use crate::cli::ExitCode;
use crate::errors::{PgxError, Result};
use crate::names::{self, DbName};
use crate::out::Out;
use crate::pgwire::quote_literal;
use serde_json::json;
use std::time::Instant;

fn exists(
    ctx: &Ctx,
    client: &mut crate::pgwire::Client<std::os::unix::net::UnixStream>,
    name: &DbName,
) -> Result<bool> {
    let _ = ctx;
    let r = client
        .simple_query(&format!(
            "SELECT 1 FROM pg_database WHERE datname = {}",
            quote_literal(name.as_str())
        ))
        .map_err(|e| db_error(e, "cannot query pg_database"))?;
    Ok(!r.rows.is_empty())
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-encode a URL userinfo component (the password): the unreserved set
/// only, so `/ ? # @ : [ ]` and the rest are escaped and cannot alter the
/// authority of a `postgres://` URL.
fn percent_encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn tcp_url(ctx: &Ctx, name: &str) -> String {
    format!(
        "postgres://postgres:{}@127.0.0.1:{}/{}",
        percent_encode_userinfo(&ctx.config.password),
        ctx.config.port,
        percent_encode(name)
    )
}

pub fn socket_url(ctx: &Ctx, name: &str) -> String {
    format!(
        "postgres://postgres@/{}?host={}&port={}",
        percent_encode(name),
        percent_encode(&ctx.sock_dir().to_string_lossy()),
        ctx.config.port
    )
}

pub fn create(out: &Out, name: &str) -> Result<ExitCode> {
    let name = names::validate_user_target(name)?;
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    client
        .simple_query(&format!("CREATE DATABASE {}", name.quoted()))
        .map_err(|e| db_error(e, &format!("cannot create database {name}")))?;
    client.close();
    let url = tcp_url(&ctx, name.as_str());
    out.emit(&format!("created database {name}\n  url  {url}"), json!({"created": name.as_str(), "url": url, "socket_url": socket_url(&ctx, name.as_str())}));
    Ok(ExitCode::Ok)
}

pub fn drop(out: &Out, name: &str, force: bool) -> Result<ExitCode> {
    let name = names::validate_user_target(name)?;
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    if !exists(&ctx, &mut client, &name)? {
        return Err(PgxError::new(
            ExitCode::NotFound,
            format!("database {name} does not exist"),
        )
        .hint("`pgx ls` lists databases"));
    }
    let sql = if force {
        format!("DROP DATABASE {} WITH (FORCE)", name.quoted())
    } else {
        format!("DROP DATABASE {}", name.quoted())
    };
    client.simple_query(&sql).map_err(|e| {
        let err = db_error(e, &format!("cannot drop database {name}"));
        if err.code == ExitCode::InUse {
            err.hint("disconnect the clients, or `pgx drop --force` to terminate their sessions")
        } else {
            err
        }
    })?;
    client.close();
    out.emit(
        &format!("dropped database {name}"),
        json!({"dropped": name.as_str(), "forced": force}),
    );
    Ok(ExitCode::Ok)
}

pub fn clone(out: &Out, source: &str, target: &str) -> Result<ExitCode> {
    let target = names::validate_user_target(target)?;
    let source = names::validate(source)?;
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    if !exists(&ctx, &mut client, &source)? {
        return Err(PgxError::new(
            ExitCode::NotFound,
            format!("source database {source} does not exist"),
        )
        .hint("`pgx ls` lists databases"));
    }
    // STRATEGY = FILE_COPY makes CREATE DATABASE copy the files, so the engine's
    // file_copy_method applies: clonefile on APFS, copy_file_range/FICLONE on
    // Linux reflink filesystems and ZFS, i.e. copy-on-write where the filesystem
    // supports it. The default WAL_LOG strategy copies every block through the
    // buffer cache and never shares blocks, which would defeat disposable clones.
    let file_copy_method = client
        .simple_query("SHOW file_copy_method")
        .ok()
        .and_then(|r| r.scalar().map(String::from))
        .unwrap_or_else(|| "unknown".into());
    let method = if file_copy_method == "clone" {
        "clone"
    } else {
        "copy"
    };
    let t0 = Instant::now();
    client
        .simple_query(&format!(
            "CREATE DATABASE {} TEMPLATE {} STRATEGY = FILE_COPY",
            target.quoted(),
            source.quoted()
        ))
        .map_err(|e| {
            let err = db_error(e, &format!("cannot clone {source} to {target}"));
            if err.code == ExitCode::InUse {
                err.hint(format!(
                    "clients are connected to {source}; disconnect them and retry"
                ))
            } else {
                err
            }
        })?;
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    client.close();
    let url = tcp_url(&ctx, target.as_str());
    out.emit(
        &format!("cloned {source} to {target} in {elapsed_ms:.0} ms ({method})\n  url  {url}"),
        json!({"source": source.as_str(), "target": target.as_str(), "elapsed_ms": (elapsed_ms * 10.0).round() / 10.0, "method": method, "url": url}),
    );
    Ok(ExitCode::Ok)
}

pub fn ls(out: &Out) -> Result<ExitCode> {
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    let r = client
        .simple_query(
            "SELECT d.datname, pg_database_size(d.oid), d.datistemplate, \
             (SELECT count(*) FROM pg_stat_activity a WHERE a.datname = d.datname AND a.backend_type = 'client backend'), \
             d.datconnlimit = -2 \
             FROM pg_database d WHERE NOT d.datistemplate ORDER BY d.datname",
        )
        .map_err(|e| db_error(e, "cannot list databases"))?;
    client.close();
    let mut items = Vec::new();
    let mut text = format!(
        "{:<32} {:>12} {:>11}  {}\n",
        "DATABASE", "SIZE", "CONNECTIONS", "STATE"
    );
    for row in &r.rows {
        let name = row[0].clone().unwrap_or_default();
        let size: u64 = row[1].as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        let conns: u64 = row[3].as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        let invalid = row[4].as_deref() == Some("t");
        text.push_str(&format!(
            "{name:<32} {:>12} {conns:>11}  {}\n",
            human_size(size),
            if invalid {
                "invalid (interrupted drop: run `pgx drop` again)"
            } else {
                "ok"
            }
        ));
        items.push(json!({"name": name, "size_bytes": size, "is_template": row[2].as_deref() == Some("t"), "connections": conns, "invalid": invalid}));
    }
    out.emit(text.trim_end(), json!(items));
    Ok(ExitCode::Ok)
}

pub fn url(out: &Out, name: &str, socket: bool) -> Result<ExitCode> {
    let name = names::validate(name)?;
    let ctx = Ctx::load()?;
    let mut client = ctx.connect("postgres")?;
    if !exists(&ctx, &mut client, &name)? {
        return Err(PgxError::new(
            ExitCode::NotFound,
            format!("database {name} does not exist"),
        )
        .hint(format!("`pgx create {name}`")));
    }
    client.close();
    let url = if socket {
        socket_url(&ctx, name.as_str())
    } else {
        tcp_url(&ctx, name.as_str())
    };
    out.emit(
        &url,
        json!({"database": name.as_str(), "url": url, "port": ctx.config.port, "host": "127.0.0.1", "user": "postgres", "socket_dir": ctx.sock_dir(), "socket_url": socket_url(&ctx, name.as_str())}),
    );
    Ok(ExitCode::Ok)
}

fn human_size(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_keeps_paths_and_escapes_specials() {
        assert_eq!(percent_encode("/tmp/pgx run"), "/tmp/pgx%20run");
        assert_eq!(percent_encode("p@ss:w/rd"), "p%40ss%3Aw/rd");
        assert_eq!(percent_encode("Abc-_.~09"), "Abc-_.~09");
    }

    #[test]
    fn userinfo_encoding_escapes_slash_and_url_specials() {
        // A password with / ? # @ : must not break the authority of a postgres:// URL.
        assert_eq!(percent_encode_userinfo("pa/ss"), "pa%2Fss");
        assert_eq!(percent_encode_userinfo("a?b#c@d:e"), "a%3Fb%23c%40d%3Ae");
        assert_eq!(percent_encode_userinfo("Abc-_.~09"), "Abc-_.~09");
        assert!(!percent_encode_userinfo("x/y?z").contains('/'));
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(7_500_000), "7.2 MB");
    }
}
