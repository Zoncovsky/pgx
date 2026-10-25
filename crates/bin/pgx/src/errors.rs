//! One error type for the whole CLI: an exit code, a message, an optional hint.
use crate::cli::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgxError {
    pub code: ExitCode,
    pub message: String,
    pub hint: Option<String>,
    /// Structured cause for `--json` consumers (for example the unsupported
    /// extensions that stopped a copy). Absent from human output.
    pub details: Option<serde_json::Value>,
}

impl PgxError {
    pub fn new(code: ExitCode, message: impl Into<String>) -> Self {
        PgxError {
            code,
            message: message.into(),
            hint: None,
            details: None,
        }
    }
    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn usage(message: impl Into<String>) -> Self {
        PgxError::new(ExitCode::Usage, message).hint("run `pgx help` for usage")
    }
    pub fn failure(message: impl Into<String>) -> Self {
        PgxError::new(ExitCode::Failure, message)
    }
    pub fn io(what: &str, path: &std::path::Path, e: &std::io::Error) -> Self {
        PgxError::failure(format!("{what} {}: {e}", path.display()))
    }
    pub fn to_json(&self) -> String {
        let mut err = serde_json::json!({
            "code": self.code.name(),
            "exit_code": self.code as i32,
            "message": self.message,
            "hint": self.hint,
        });
        if let Some(d) = &self.details {
            err["details"] = d.clone();
        }
        serde_json::json!({ "error": err }).to_string()
    }
}

impl std::fmt::Display for PgxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(h) = &self.hint {
            write!(f, "\n  hint: {h}")?;
        }
        Ok(())
    }
}

pub type Result<T> = std::result::Result<T, PgxError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ExitCode;

    #[test]
    fn error_carries_exit_code_and_hint() {
        let e = PgxError::new(ExitCode::NotFound, "database \"x\" does not exist").hint("pgx ls");
        assert_eq!(e.code, ExitCode::NotFound);
        assert_eq!(e.message, "database \"x\" does not exist");
        assert_eq!(e.hint.as_deref(), Some("pgx ls"));
        assert_eq!(e.code as i32, 5);
    }

    #[test]
    fn error_json_shape() {
        let e = PgxError::new(ExitCode::Conflict, "port 5432 is in use");
        let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["code"], "conflict");
        assert_eq!(v["error"]["exit_code"], 4);
        assert_eq!(v["error"]["message"], "port 5432 is in use");
        assert!(v["error"].get("details").is_none());
        let e = e.details(serde_json::json!({"unsupported_extensions": ["postgis"]}));
        let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(
            v["error"]["details"]["unsupported_extensions"][0],
            "postgis"
        );
    }
}
