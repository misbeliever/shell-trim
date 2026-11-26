use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::cli::ExecTrimOptions;
use crate::telemetry;

const MAX_LS_ITEMS: usize = 80;
const MAX_FIND_ITEMS: usize = 80;

const NOISY_DIRS: &[&str] = &[
    ".git",
    ".build",
    "node_modules",
    "target",
    "DerivedData",
    ".next",
    "__pycache__",
    ".venv",
    "dist",
];

pub fn run_ls(opts: &ExecTrimOptions) -> Result<ExitCode, String> {
    let output = Command::new("ls")
        .args(&opts.args)
        .output()
        .map_err(|e| format!("failed to run ls: {e}"))?;

    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(&output.stderr);
    }
    let code = output.status.code().unwrap_or(1);
    if output.stdout.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = output.stdout.len();
    let raw_str = String::from_utf8_lossy(&output.stdout);
    let total_lines = raw_str.lines().count().max(1);

    let (rendered, eff_mode) = if opts.raw {
        (raw_str.into_owned(), "raw")
    } else {
        let is_long = opts
            .args
            .iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('l'));
        let compact = if is_long {
            compact_ls_long(&raw_str, opts.all)
        } else {
            compact_ls_plain(&raw_str, opts.all)
        };
        if compact.len() >= raw_bytes {
            (raw_str.into_owned(), "raw")
        } else {
            (compact, if opts.all { "all" } else { "trimmed" })
        }
    };

    let out_bytes = rendered.len();
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if opts.log_stats {
        let req_mode = if opts.raw {
            "raw"
        } else if opts.all {
            "all"
        } else {
            "auto"
        };
        let key = format_fs_key("ls", &opts.args);
        telemetry::log_cmd_event(
            "fs",
            &key,
            "fs",
            req_mode,
            eff_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

pub fn compact_ls_long(raw: &str, show_all: bool) -> String {
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut total_seen = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("total ") {
            continue;
        }
        // Standard `ls -l` columns: perms links owner group size month day time/year name...
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 9 {
            continue;
        }
        let perms = parts[0];
        let size_bytes: u64 = parts[4].parse().unwrap_or(0);
        let name = parts[8..].join(" ");
        if name == "." || name == ".." {
            continue;
        }
        total_seen += 1;
        if perms.starts_with('d') {
            dirs.push(format!("{name}/"));
        } else if size_bytes >= 1024 {
            files.push(format!("{name} ({})", fmt_size(size_bytes)));
        } else {
            files.push(name);
        }
    }

    if total_seen == 0 {
        return raw.to_string();
    }

    format_ls_groups(&dirs, &files, show_all)
}

pub fn compact_ls_plain(raw: &str, show_all: bool) -> String {
    let entries: Vec<&str> = raw.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if entries.len() <= 12 {
        return raw.to_string();
    }
    let cap = if show_all { usize::MAX } else { MAX_LS_ITEMS };
    let shown: Vec<&str> = entries.iter().copied().take(cap).collect();
    let mut out = shown.join(", ");
    if entries.len() > cap {
        out.push_str(&format!(
            "\n[trm: +{} more entries — pass --all]",
            entries.len() - cap
        ));
    }
    out.push('\n');
    out
}

fn format_ls_groups(dirs: &[String], files: &[String], show_all: bool) -> String {
    let cap = if show_all { usize::MAX } else { MAX_LS_ITEMS };
    let mut out = Vec::new();
    let mut budget = cap;

    if !dirs.is_empty() {
        let take = dirs.len().min(budget);
        budget = budget.saturating_sub(take);
        let mut line = format!("dirs ({}): {}", dirs.len(), dirs[..take].join("  "));
        if dirs.len() > take {
            line.push_str(&format!("  … (+{} more)", dirs.len() - take));
        }
        out.push(line);
    }

    if !files.is_empty() {
        let take = files.len().min(budget);
        let mut line = format!("files ({}): {}", files.len(), files[..take].join(", "));
        if files.len() > take {
            line.push_str(&format!(", … (+{} more)", files.len() - take));
        }
        out.push(line);
    }

    if out.is_empty() {
        return String::new();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn run_find(opts: &ExecTrimOptions) -> Result<ExitCode, String> {
    let output = Command::new("find")
        .args(&opts.args)
        .output()
        .map_err(|e| format!("failed to run find: {e}"))?;

    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(&output.stderr);
    }
    let code = output.status.code().unwrap_or(1);
    if output.stdout.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = output.stdout.len();
    let raw_str = String::from_utf8_lossy(&output.stdout);
    let total_lines = raw_str.lines().count().max(1);

    let (rendered, eff_mode) = if opts.raw {
        (raw_str.into_owned(), "raw")
    } else {
        let compact = compact_find_output(&raw_str, opts.all);
        if compact.len() < raw_bytes {
            (compact, if opts.all { "all" } else { "trimmed" })
        } else if !opts.all {
            let filtered = filter_noisy_find_lines(&raw_str);
            if filtered.len() < raw_bytes {
                (filtered, "trimmed")
            } else {
                (raw_str.into_owned(), "raw")
            }
        } else {
            (raw_str.into_owned(), "raw")
        }
    };

    let out_bytes = rendered.len();
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if opts.log_stats {
        let req_mode = if opts.raw {
            "raw"
        } else if opts.all {
            "all"
        } else {
            "auto"
        };
        let key = format_fs_key("find", &opts.args);
        telemetry::log_cmd_event(
            "fs",
            &key,
            "fs",
            req_mode,
            eff_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

pub fn compact_find_output(raw: &str, show_all: bool) -> String {
    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut skipped_noisy = 0usize;
    let mut total_kept = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "." {
            continue;
        }
        if !show_all && is_noisy_path(trimmed) {
            skipped_noisy += 1;
            continue;
        }
        let p = Path::new(trimmed);
        let dir = p
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| trimmed.to_string());
        by_dir.entry(dir).or_default().push(name);
        total_kept += 1;
    }

    if total_kept == 0 && skipped_noisy == 0 {
        return raw.to_string();
    }

    let cap = if show_all { usize::MAX } else { MAX_FIND_ITEMS };
    let mut out = String::new();
    let mut shown = 0usize;
    let mut hidden_files = 0usize;
    let mut hidden_dirs = 0usize;

    for (dir, names) in by_dir {
        if shown >= cap {
            hidden_files += names.len();
            hidden_dirs += 1;
            continue;
        }
        let budget = cap - shown;
        let take = names.len().min(budget);
        out.push_str(&dir);
        out.push_str("/ ");
        out.push_str(&names[..take].join(", "));
        if names.len() > take {
            let rem = names.len() - take;
            out.push_str(&format!(" (+{rem} more)"));
            hidden_files += rem;
        }
        out.push('\n');
        shown += take;
    }

    if hidden_files > 0 || skipped_noisy > 0 {
        let mut notes = Vec::new();
        if hidden_files > 0 {
            notes.push(format!("+{hidden_files} more in {hidden_dirs} dirs"));
        }
        if skipped_noisy > 0 {
            notes.push(format!("{skipped_noisy} noisy build/git paths hidden"));
        }
        out.push_str(&format!("[trm: {} — pass --all for full]\n", notes.join(", ")));
    }

    out
}

fn format_fs_key(cmd: &str, args: &[String]) -> String {
    if args.is_empty() {
        return cmd.to_string();
    }
    let joined = args.join(" ");
    let mut s = format!("{cmd}:{joined}");
    if s.len() > 80 {
        let mut end = 79;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

fn filter_noisy_find_lines(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for line in raw.lines() {
        if !is_noisy_path(line.trim()) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn is_noisy_path(path: &str) -> bool {
    path.split('/').any(|seg| NOISY_DIRS.contains(&seg))
}

fn fmt_size(b: u64) -> String {
    if b >= 1_048_576 {
        format!("{:.1}M", b as f64 / 1_048_576.0)
    } else {
        format!("{}K", b / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_ls_long_and_strips_metadata_noise() {
        let raw = "\
total 48
drwxr-xr-x   5 alice staff   160 Nov 26 12:00 .
drwxr-xr-x  10 alice staff   320 Nov 26 12:00 ..
drwxr-xr-x  12 alice staff   384 Nov 26 12:00 src
-rw-r--r--   1 alice staff   412 Nov 26 12:00 Cargo.toml
-rw-r--r--   1 alice staff  5120 Nov 26 12:00 README.md
";
        let out = compact_ls_long(raw, false);
        assert!(out.contains("dirs (1): src/"), "got: {out}");
        assert!(out.contains("files (2): Cargo.toml, README.md (5K)"), "got: {out}");
        assert!(!out.contains("alice"), "should strip owner/perms: {out}");
    }

    #[test]
    fn compacts_find_groups_by_dir_and_hides_noisy_paths() {
        let raw = "\
./src/main.rs
./src/cli.rs
./target/debug/trm
./.git/HEAD
./tests/integration.rs
";
        let out = compact_find_output(raw, false);
        assert!(out.contains("./src/ main.rs, cli.rs"), "got: {out}");
        assert!(out.contains("./tests/ integration.rs"), "got: {out}");
        assert!(!out.contains(".git/HEAD"), "should hide .git: {out}");
        assert!(!out.contains("target/debug"), "should hide target: {out}");
        assert!(out.contains("2 noisy build/git paths hidden"), "got: {out}");
    }
}
