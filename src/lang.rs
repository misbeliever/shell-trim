use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Swift,
    TypeScript,
    JavaScript,
    Rust,
    Python,
    Go,
    CLike, // C, C++, Obj-C, Java, Kotlin, C#
    Shell,
    Yaml,
    Json,
    Markdown,
    Toml,
    Sql,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `//` and `/* ... */` (nested if `nested_block = true`)
    Slash { nested_block: bool },
    /// `#` line comments (Python, Shell, YAML, TOML)
    Hash { python_docstrings: bool },
    /// `--` and `/* ... */`
    Sql,
    /// No comment stripping (JSON, Markdown, Generic)
    None,
}

impl Language {
    pub fn detect(path: &str, override_ext: Option<&str>) -> Self {
        if let Some(ov) = override_ext {
            return Self::from_ext(&ov.to_lowercase());
        }
        let p = Path::new(path);
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            match lower.as_str() {
                "makefile" | "dockerfile" | "podfile" | "gemfile" | "vagrantfile" => {
                    return Language::Shell;
                }
                "package.swift" => return Language::Swift,
                "cargo.toml" => return Language::Toml,
                _ => {}
            }
        }
        match p.extension().and_then(|e| e.to_str()) {
            Some(ext) => Self::from_ext(&ext.to_lowercase()),
            None => Language::Generic,
        }
    }

    pub fn from_ext(ext: &str) -> Self {
        match ext.trim_start_matches('.') {
            "swift" => Language::Swift,
            "ts" | "tsx" | "mts" | "cts" | "typescript" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" | "javascript" => Language::JavaScript,
            "rs" | "rust" => Language::Rust,
            "py" | "pyi" | "pyw" | "python" => Language::Python,
            "go" | "golang" => Language::Go,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" | "java" | "kt"
            | "kts" | "cs" | "objc" => Language::CLike,
            "sh" | "bash" | "zsh" | "fish" | "shell" | "rb" | "pl" | "conf" | "env" => {
                Language::Shell
            }
            "yml" | "yaml" => Language::Yaml,
            "json" | "jsonc" => Language::Json,
            "md" | "mdx" | "markdown" => Language::Markdown,
            "toml" => Language::Toml,
            "sql" => Language::Sql,
            _ => Language::Generic,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Language::Swift => "swift",
            Language::TypeScript => "ts",
            Language::JavaScript => "js",
            Language::Rust => "rust",
            Language::Python => "python",
            Language::Go => "go",
            Language::CLike => "c-like",
            Language::Shell => "shell",
            Language::Yaml => "yaml",
            Language::Json => "json",
            Language::Markdown => "markdown",
            Language::Toml => "toml",
            Language::Sql => "sql",
            Language::Generic => "text",
        }
    }

    pub fn comment_style(&self) -> CommentStyle {
        match self {
            Language::Swift | Language::Rust => CommentStyle::Slash { nested_block: true },
            Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::CLike => CommentStyle::Slash {
                nested_block: false,
            },
            Language::Python => CommentStyle::Hash {
                python_docstrings: true,
            },
            Language::Shell | Language::Yaml | Language::Toml => CommentStyle::Hash {
                python_docstrings: false,
            },
            Language::Sql => CommentStyle::Sql,
            Language::Json | Language::Markdown | Language::Generic => CommentStyle::None,
        }
    }

    pub fn supports_outline(&self) -> bool {
        matches!(
            self,
            Language::Swift
                | Language::TypeScript
                | Language::JavaScript
                | Language::Rust
                | Language::Python
                | Language::Go
                | Language::CLike
                | Language::Markdown
                | Language::Json
                | Language::Yaml
        )
    }
}
