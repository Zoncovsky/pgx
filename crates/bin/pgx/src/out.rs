//! Human or JSON output. Every command produces both forms; `--json` selects.
pub struct Out {
    pub json: bool,
}

impl Out {
    pub fn new(json: bool) -> Out {
        Out { json }
    }
    /// `text` goes to stdout in human mode; `value` is printed as one JSON line in --json mode.
    pub fn emit(&self, text: &str, value: serde_json::Value) {
        if self.json {
            println!("{value}");
        } else if !text.is_empty() {
            println!("{text}");
        }
    }
    /// Progress lines for humans only (stderr, so stdout stays parseable).
    pub fn note(&self, text: &str) {
        if !self.json {
            eprintln!("{text}");
        }
    }
}
