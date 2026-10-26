//! The project a session runs in, and its committed `.pgx.toml`.
//!
//! PGX is one runtime per user, so database names are global across projects.
//! The branch source therefore has to be a per-project fact; `.pgx.toml` in
//! the project root carries it, and the project root's canonical path gives
//! every project a stable id that keeps identical session ids apart.
use super::identity::{fnv1a64, hex16};
use crate::errors::{PgxError, Result};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = ".pgx.toml";
pub const DEFAULT_SOURCE: &str = "app";
pub const DEFAULT_IDLE_TTL: &str = "2h";
pub const DEFAULT_GRACE: &str = "10m";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Canonical project root.
    pub root: PathBuf,
    /// 16 hex digits of FNV-1a 64 over the canonical root path.
    pub project_id: String,
    pub source: Option<String>,
    pub idle_ttl: Option<String>,
    pub grace_period: Option<String>,
    /// The `.pgx.toml` that was read, if any.
    pub config_file: Option<PathBuf>,
}

fn is_repo_root(dir: &Path) -> bool {
    // A `.git` directory, or the `.git` file a worktree carries.
    dir.join(".git").exists()
}

/// Walk up from `dir` looking first for `.pgx.toml`, then for a repository
/// root. Returns `(root, config_file)`.
fn find_root(dir: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let f = d.join(CONFIG_FILE);
        if f.is_file() {
            return Some((d.to_path_buf(), Some(f)));
        }
        cur = d.parent();
    }
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if is_repo_root(d) {
            return Some((d.to_path_buf(), None));
        }
        cur = d.parent();
    }
    None
}

/// Resolve the project for a session. `start` is where the session runs (the
/// hook's `cwd`); `fallbacks` are the host's idea of the project root
/// (`CLAUDE_PROJECT_DIR`, `CURSOR_PROJECT_DIR`, `workspace_roots[0]`), tried
/// when `start` yields neither a `.pgx.toml` nor a repository. With nothing
/// found anywhere, the project is `start` itself.
pub fn locate(start: &Path, fallbacks: &[PathBuf]) -> Result<ProjectConfig> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let start = canon(start);
    let found = find_root(&start).or_else(|| {
        fallbacks
            .iter()
            .filter(|f| f.is_dir())
            .find_map(|f| find_root(&canon(f)))
    });
    let (root, config_file) = found.unwrap_or((start.clone(), None));
    let root = canon(&root);
    let (source, idle_ttl, grace_period) = match &config_file {
        Some(f) => {
            let text =
                std::fs::read_to_string(f).map_err(|e| PgxError::io("cannot read", f, &e))?;
            parse_pgx_toml(&text)
                .map_err(|e| PgxError::usage(format!("{}: {}", f.display(), e.message)))?
        }
        None => (None, None, None),
    };
    Ok(ProjectConfig {
        project_id: hex16(fnv1a64(root.to_string_lossy().as_bytes())),
        root,
        source,
        idle_ttl,
        grace_period,
        config_file: config_file.map(|f| canon(&f)),
    })
}

/// The `[agent]` table of `.pgx.toml`: `source`, `idle_ttl`, `grace_period`.
/// A deliberately tiny TOML subset: `[section]` headers, `key = "value"` or
/// `key = 'value'` lines, `#` comments. Unknown keys and other sections are
/// ignored. Values are validated here so a hostile or mistaken file cannot
/// smuggle a connection URL or an invalid name into a command.
pub fn parse_pgx_toml(text: &str) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let mut in_agent = false;
    let mut source = None;
    let mut idle_ttl = None;
    let mut grace = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_agent = section.trim() == "agent";
            continue;
        }
        if !in_agent {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let Some(v) = unquote(v.trim()) else {
            continue;
        };
        if v.contains("://") {
            return Err(PgxError::usage(format!(
                "{CONFIG_FILE}: {k} must be a PGX database name, not a connection URL (connection URLs do not belong in {CONFIG_FILE})"
            )));
        }
        match k {
            "source" => {
                crate::names::validate(&v).map_err(|e| {
                    PgxError::usage(format!("{CONFIG_FILE}: source: {}", e.message))
                })?;
                source = Some(v);
            }
            "idle_ttl" | "grace_period" => {
                match crate::ttl::parse_ttl(&v) {
                    Ok(crate::ttl::Ttl::Seconds(_)) => {}
                    Ok(crate::ttl::Ttl::Unlimited) => {
                        return Err(PgxError::usage(format!(
                            "{CONFIG_FILE}: {k} must be a finite duration such as 2h; agent branches always expire"
                        )))
                    }
                    Err(e) => {
                        return Err(PgxError::usage(format!("{CONFIG_FILE}: {k}: {}", e.message)))
                    }
                }
                if k == "idle_ttl" {
                    idle_ttl = Some(v);
                } else {
                    grace = Some(v);
                }
            }
            _ => {}
        }
    }
    Ok((source, idle_ttl, grace))
}

fn strip_comment(line: &str) -> &str {
    // A `#` outside quotes starts a comment.
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '#') => return &line[..i],
            (None, '"') | (None, '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> Option<String> {
    let inner = v
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
    Some(inner.to_string())
}

/// The file `pgx agent setup` writes. No secrets, no URLs: names and durations only.
pub fn render_pgx_toml(source: &str, idle_ttl: &str, grace: &str) -> String {
    format!(
        "# .pgx.toml — PGX agent configuration for this project (safe to commit).\n\
# Written by `pgx agent setup`; see docs/pgx/local-dev/agents.md.\n\
[agent]\n\
source = \"{source}\"        # the PGX database every agent session branches from\n\
idle_ttl = \"{idle_ttl}\"           # a branch expires this long after the agent's last activity\n\
grace_period = \"{grace}\"      # how long a branch survives after its session ends\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "pgx-agent-project-{tag}-{}-{}",
            std::process::id(),
            crate::fsutil::now_epoch()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn toml_subset_reads_the_agent_table() {
        let (s, t, g) = parse_pgx_toml(
            "# PGX agent config\n[agent]\nsource = \"myapp_template\"\nidle_ttl = \"3h\"\n",
        )
        .unwrap();
        assert_eq!(s.as_deref(), Some("myapp_template"));
        assert_eq!(t.as_deref(), Some("3h"));
        assert!(g.is_none());
        // Keys outside [agent] and unknown keys are ignored.
        let (s, _, g) = parse_pgx_toml(
            "[other]\nsource = \"nope\"\n[agent]\nextra = 1\ngrace_period = \"5m\"\n",
        )
        .unwrap();
        assert!(s.is_none());
        assert_eq!(g.as_deref(), Some("5m"));
        // Single quotes and inline comments.
        let (s, _, _) = parse_pgx_toml("[agent]\nsource = 'tpl' # the template\n").unwrap();
        assert_eq!(s.as_deref(), Some("tpl"));
    }

    #[test]
    fn toml_rejects_connection_urls_and_bad_names() {
        assert!(parse_pgx_toml("[agent]\nsource = \"postgres://u:p@h/db\"\n").is_err());
        assert!(parse_pgx_toml("[agent]\nsource = \"bad name\"\n").is_err());
        assert!(parse_pgx_toml("[agent]\nidle_ttl = \"soon\"\n").is_err());
        assert!(parse_pgx_toml("[agent]\nidle_ttl = \"unlimited\"\n").is_err());
    }

    #[test]
    fn render_roundtrips() {
        let text = render_pgx_toml("myapp_template", "2h", "10m");
        assert!(text.starts_with("# .pgx.toml"));
        let (s, t, g) = parse_pgx_toml(&text).unwrap();
        assert_eq!(s.as_deref(), Some("myapp_template"));
        assert_eq!(t.as_deref(), Some("2h"));
        assert_eq!(g.as_deref(), Some("10m"));
        assert!(!text.contains("://"));
    }

    #[test]
    fn locate_prefers_pgx_toml_then_git_then_start() {
        let root = tmp("locate");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        // No .pgx.toml anywhere: the git top level wins.
        let p = locate(&root.join("sub/deep"), &[]).unwrap();
        assert_eq!(p.root, std::fs::canonicalize(&root).unwrap());
        assert!(p.config_file.is_none());
        assert!(p.source.is_none());
        assert_eq!(p.project_id.len(), 16);
        // A .pgx.toml below the git root wins over it.
        std::fs::write(root.join("sub/.pgx.toml"), "[agent]\nsource = \"tpl\"\n").unwrap();
        let p = locate(&root.join("sub/deep"), &[]).unwrap();
        assert_eq!(p.root, std::fs::canonicalize(root.join("sub")).unwrap());
        assert_eq!(p.source.as_deref(), Some("tpl"));
        assert_eq!(
            p.config_file,
            Some(
                std::fs::canonicalize(root.join("sub"))
                    .unwrap()
                    .join(".pgx.toml")
            )
        );
        // A worktree `.git` file counts as a repository root too.
        let wt = tmp("wt");
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere\n").unwrap();
        std::fs::create_dir_all(wt.join("a/b")).unwrap();
        assert_eq!(
            locate(&wt.join("a/b"), &[]).unwrap().root,
            std::fs::canonicalize(&wt).unwrap()
        );
        // Fallback directories are searched when the start has nothing.
        let bare = tmp("bare");
        let p = locate(&bare, &[root.join("sub")]).unwrap();
        assert_eq!(p.source.as_deref(), Some("tpl"));
        // Nothing at all: the start directory itself.
        let p = locate(&bare, &[]).unwrap();
        assert_eq!(p.root, std::fs::canonicalize(&bare).unwrap());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(wt);
        let _ = std::fs::remove_dir_all(bare);
    }

    #[test]
    fn project_ids_are_stable_and_distinct() {
        let a = tmp("ida");
        let b = tmp("idb");
        let ia = locate(&a, &[]).unwrap().project_id;
        assert_eq!(ia, locate(&a, &[]).unwrap().project_id);
        assert_ne!(ia, locate(&b, &[]).unwrap().project_id);
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }
}
