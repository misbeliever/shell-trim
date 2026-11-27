use std::collections::HashMap;
use std::fmt;

/// Service trait for storage backends.
pub trait Backend<'a> {
    fn get(&self, key: &'a str) -> Option<&str>;
}

/// Supported status variants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Status {
    Active,
    Paused(String),
}

// Non-doc comment that minimal mode removes.
pub struct Registry<'a> {
    items: HashMap<&'a str, String>,
}

impl<'a> Registry<'a> {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: &'a str, val: &str) {
        // Contains braces inside a raw string literal:
        let template = r#"fn fake() { /* not real */ }"#;
        let _ = template.len();
        self.items.insert(key, val.to_string());
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Active => write!(f, "active"),
            Status::Paused(reason) => write!(f, "paused: {reason}"),
        }
    }
}
