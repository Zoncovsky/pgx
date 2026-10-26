//! Who a branch belongs to: `(host, session_id, project_id)`, and the database
//! name derived from it.
use crate::errors::{PgxError, Result};

/// FNV-1a 64-bit: a stable, dependency-free hash. (`std`'s `DefaultHasher` is
/// not stable across Rust versions, and a renamed branch would break resume.)
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hex16(v: u64) -> String {
    format!("{v:016x}")
}

/// The identity of one coding-agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey {
    pub host: String,
    pub session_id: String,
    pub project_id: String,
}

impl SessionKey {
    /// The bytes every derived hash is computed over.
    fn material(&self) -> String {
        format!("{}|{}|{}", self.project_id, self.host, self.session_id)
    }
}

/// A host label: `^[a-z][a-z0-9]{0,15}$` after lowercasing.
pub fn validate_host(s: &str) -> Result<String> {
    let h = s.to_ascii_lowercase();
    let ok = !h.is_empty()
        && h.len() <= 16
        && h.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && h.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(h)
    } else {
        Err(PgxError::usage(format!(
            "invalid host {s:?}: use 1-16 lowercase letters or digits, starting with a letter (claude, cursor, codex, …)"
        )))
    }
}

/// A session id as hosts hand it over (UUIDs, chat ids) or as a user picks it
/// for `pgx agent exec --id`: 1..=128 bytes of printable ASCII, no whitespace,
/// no `/`, no shell metacharacters. It is used as a lookup key and never reaches
/// SQL unquoted, but a tight alphabet keeps hostile payloads boring.
pub fn validate_session_id(s: &str) -> Result<String> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@' | b'+' | b'=')
        })
        && !s.contains("..");
    if ok {
        Ok(s.to_string())
    } else {
        Err(PgxError::usage(format!(
            "invalid session id {s:?}: use up to 128 characters from A-Z a-z 0-9 - _ . : @ + ="
        )))
    }
}

/// `agent-<host>-<sid8>`; with `long`, `agent-<host>-<sid8>-<16 hex of the full key>`.
/// `sid8` is the first 8 characters of the session id, lowercased, when all of
/// them are `[a-z0-9]` (UUIDs qualify); otherwise the first 8 hex digits of
/// the key's FNV hash.
pub fn branch_name(key: &SessionKey, long: bool) -> String {
    let lowered = key.session_id.to_ascii_lowercase();
    let first8: String = lowered.chars().take(8).collect();
    let sid8 = if first8.len() == 8
        && first8
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        first8
    } else {
        // Hash the id alone: the short name stays project-independent and the
        // long name carries the project, exactly like the readable case.
        hex16(fnv1a64(key.session_id.as_bytes()))[..8].to_string()
    };
    if long {
        format!(
            "agent-{}-{}-{}",
            key.host,
            sid8,
            hex16(fnv1a64(key.material().as_bytes()))
        )
    } else {
        format!("agent-{}-{}", key.host, sid8)
    }
}

/// The environment variable a host sets in the commands it runs, carrying its
/// own session id. Cursor sets none.
pub fn session_env_var(host: &str) -> Option<&'static str> {
    match host {
        "claude" => Some("CLAUDE_CODE_SESSION_ID"),
        "codex" => Some("CODEX_SESSION_ID"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(host: &str, sid: &str) -> SessionKey {
        SessionKey {
            host: host.to_string(),
            session_id: sid.to_string(),
            project_id: "a1b2c3d4e5f60718".to_string(),
        }
    }

    #[test]
    fn fnv_matches_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(hex16(0xaf63dc4c8601ec8c), "af63dc4c8601ec8c");
        assert_eq!(hex16(1), "0000000000000001");
    }

    #[test]
    fn uuid_session_gives_readable_name() {
        let k = key("claude", "9B1F3C2D-aaaa-bbbb-cccc-dddddddddddd");
        assert_eq!(branch_name(&k, false), "agent-claude-9b1f3c2d");
        let long = branch_name(&k, true);
        assert!(long.starts_with("agent-claude-9b1f3c2d-"), "{long}");
        assert_eq!(long.len(), "agent-claude-9b1f3c2d-".len() + 16);
    }

    #[test]
    fn odd_session_ids_hash_and_stay_valid_names() {
        let k = key("exec", "my task!");
        let short = branch_name(&k, false);
        assert!(short.starts_with("agent-exec-"), "{short}");
        assert_eq!(short.len(), "agent-exec-".len() + 8);
        assert!(crate::names::validate(&short).is_ok());
        assert!(crate::names::validate(&branch_name(&k, true)).is_ok());
        // Different projects, same host and id: same short name, different long name.
        let mut other = key("exec", "my task!");
        other.project_id = "ffffffffffffffff".into();
        assert_eq!(branch_name(&other, false), short);
        assert_ne!(branch_name(&other, true), branch_name(&k, true));
    }

    #[test]
    fn short_ids_are_not_padded_with_garbage() {
        let k = key("codex", "abc");
        let n = branch_name(&k, false);
        assert!(crate::names::validate(&n).is_ok());
        assert!(n.starts_with("agent-codex-"), "{n}");
    }

    #[test]
    fn hosts_and_ids_are_validated() {
        assert_eq!(validate_host("claude").unwrap(), "claude");
        assert_eq!(validate_host("Claude").unwrap(), "claude");
        assert!(validate_host("").is_err());
        assert!(validate_host("my host").is_err());
        assert!(validate_host("a-b").is_err());
        assert!(validate_host(&"x".repeat(17)).is_err());
        assert_eq!(validate_session_id("9b1f-3c2d").unwrap(), "9b1f-3c2d");
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("a b").is_err());
        assert!(validate_session_id("../x").is_err());
        assert!(validate_session_id("x; rm -rf /").is_err());
        assert!(validate_session_id(&"x".repeat(129)).is_err());
        assert!(validate_session_id("é").is_err());
    }

    #[test]
    fn session_env_vars_per_host() {
        assert_eq!(session_env_var("claude"), Some("CLAUDE_CODE_SESSION_ID"));
        assert_eq!(session_env_var("codex"), Some("CODEX_SESSION_ID"));
        assert_eq!(session_env_var("cursor"), None);
        assert_eq!(session_env_var("exec"), None);
    }
}
