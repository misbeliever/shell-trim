use std::env;
use std::fs;
use std::path::PathBuf;

/// User configuration loaded lazily from `~/.config/trm/config.toml` (or `$TRM_CONFIG`).
/// Zero external crates: parses flat `key = value` pairs and optional `[read]` / `[rg]` / `[stats]` headers.
#[derive(Debug, Clone)]
pub struct Config {
    pub small_lines: usize,
    pub outline_lines: usize,
    pub minified_bytes: usize,
    pub rg_max_total: usize,
    pub rg_per_file: usize,
    pub rg_max_chars: usize,
    pub reread_window_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            small_lines: 80,
            outline_lines: 260,
            minified_bytes: 12_288,
            rg_max_total: 50,
            rg_per_file: 6,
            rg_max_chars: 180,
            reread_window_secs: 180,
        }
    }
}

impl Config {
    pub fn home_dir() -> PathBuf {
        PathBuf::from(
            env::var("TRM_HOME")
                .or_else(|_| env::var("HOME"))
                .unwrap_or_else(|_| ".".into()),
        )
    }

    pub fn path() -> PathBuf {
        if let Ok(p) = env::var("TRM_CONFIG") {
            return PathBuf::from(p);
        }
        Self::home_dir()
            .join(".config")
            .join("trm")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let mut cfg = Self::default();
        let path = Self::path();
        let Ok(content) = fs::read_to_string(&path) else {
            return cfg;
        };

        let mut section = String::new();
        for raw_line in content.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_lowercase();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_lowercase();
            let val = v.trim().trim_matches('"').trim_matches('\'');
            let full_key = if section.is_empty() {
                key.clone()
            } else {
                format!("{section}.{key}")
            };

            match full_key.as_str() {
                "small_lines" | "read.small_lines" => {
                    if let Ok(n) = val.parse() {
                        cfg.small_lines = n;
                    }
                }
                "outline_lines" | "read.outline_lines" => {
                    if let Ok(n) = val.parse() {
                        cfg.outline_lines = n;
                    }
                }
                "minified_bytes" | "read.minified_bytes" => {
                    if let Ok(n) = val.parse() {
                        cfg.minified_bytes = n;
                    }
                }
                "rg_max_total" | "rg.max_total" => {
                    if let Ok(n) = val.parse() {
                        cfg.rg_max_total = n;
                    }
                }
                "rg_per_file" | "rg.per_file" => {
                    if let Ok(n) = val.parse() {
                        cfg.rg_per_file = n;
                    }
                }
                "rg_max_chars" | "rg.max_chars" => {
                    if let Ok(n) = val.parse() {
                        cfg.rg_max_chars = n;
                    }
                }
                "reread_window_secs" | "stats.reread_window_secs" => {
                    if let Ok(n) = val.parse() {
                        cfg.reread_window_secs = n;
                    }
                }
                _ => {}
            }
        }
        cfg
    }
}
