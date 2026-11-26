use std::io::{self, Write};
use std::process::{Command, ExitCode, Stdio};

use crate::cli::ExecTrimOptions;
use crate::telemetry;

const MAX_DIFF_LINES_PER_FILE: usize = 120;

const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Podfile.lock",
    "Package.resolved",
    "Gemfile.lock",
    "poetry.lock",
    "uv.lock",
    "composer.lock",
];

pub fn run(opts: &ExecTrimOptions) -> Result<ExitCode, String> {
    let subcmd = opts.args.first().map(|s| s.as_str()).unwrap_or("");

    if !opts.raw && subcmd == "status" && !has_short_status_flag(&opts.args[1..]) {
        return run_compact_status(&opts.args[1..], opts.log_stats);
    }

    if !opts.raw && subcmd == "log" && !has_custom_log_format(&opts.args[1..]) {
        return run_compact_log(&opts.args[1..], opts.log_stats);
    }

    let output = Command::new("git")
        .stdin(Stdio::inherit())
        .args(&opts.args)
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "'git' not found in PATH".to_string()
            } else {
                format!("failed to run git: {e}")
            }
        })?;

    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(&output.stderr);
    }

    let code = output.status.code().unwrap_or(1);
    if output.stdout.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = output.stdout.len();
    let raw_str = String::from_utf8_lossy(&output.stdout);

    let (rendered, eff_mode) = if opts.raw || subcmd != "diff" || is_already_compact_diff(&opts.args[1..]) {
        (raw_str.into_owned(), "raw")
    } else {
        let trimmed = trim_git_diff(&raw_str, opts.all);
        if trimmed.len() >= raw_bytes {
            (raw_str.into_owned(), "raw")
        } else {
            (trimmed, if opts.all { "all" } else { "trimmed" })
        }
    };

    let out_bytes = rendered.len();
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if opts.log_stats && matches!(subcmd, "diff" | "status" | "log") {
        let key = format!("git:{subcmd}");
        let req_mode = if opts.raw {
            "raw"
        } else if opts.all {
            "all"
        } else {
            "auto"
        };
        let total_lines = raw_bytes_lines(&output.stdout);
        telemetry::log_cmd_event(
            "git",
            &key,
            "git",
            req_mode,
            eff_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

fn raw_bytes_lines(b: &[u8]) -> usize {
    b.iter().filter(|&&x| x == b'\n').count().max(1)
}

fn has_short_status_flag(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "-s" || a == "--short" || a.starts_with("--porcelain"))
}

fn has_custom_log_format(args: &[String]) -> bool {
    args.iter().any(|a| {
        a == "--oneline"
            || a == "-p"
            || a == "--patch"
            || a == "--stat"
            || a.starts_with("--format")
            || a.starts_with("--pretty")
    })
}

fn has_log_limit(args: &[String]) -> bool {
    args.iter().any(|a| {
        a == "-n"
            || a.starts_with("--max-count")
            || (a.starts_with('-') && a.len() > 1 && a[1..].bytes().all(|b| b.is_ascii_digit()))
    })
}

fn is_already_compact_diff(args: &[String]) -> bool {
    args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--stat" | "--shortstat" | "--numstat" | "--name-only" | "--name-status"
        )
    })
}

fn run_compact_status(extra_args: &[String], log_stats: bool) -> Result<ExitCode, String> {
    let porcelain = Command::new("git")
        .arg("status")
        .args(["--porcelain=v1", "-b"])
        .args(extra_args)
        .output()
        .map_err(|e| format!("failed to run git status: {e}"))?;

    if !porcelain.stderr.is_empty() {
        let _ = io::stderr().write_all(&porcelain.stderr);
    }
    let code = porcelain.status.code().unwrap_or(1);
    if code != 0 {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_porcelain = String::from_utf8_lossy(&porcelain.stdout);
    let compact = format_porcelain_status(&raw_porcelain);

    // Measure baseline against standard `git status` for honest telemetry
    let baseline_bytes = if log_stats {
        Command::new("git")
            .arg("status")
            .args(extra_args)
            .output()
            .map(|o| o.stdout.len().max(compact.len()))
            .unwrap_or(compact.len())
    } else {
        compact.len()
    };

    io::stdout()
        .write_all(compact.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if log_stats {
        let eff = if compact.len() < baseline_bytes {
            "trimmed"
        } else {
            "raw"
        };
        telemetry::log_cmd_event(
            "git",
            "git:status",
            "git",
            "auto",
            eff,
            compact.lines().count().max(1),
            baseline_bytes,
            compact.len(),
        );
    }

    Ok(ExitCode::SUCCESS)
}

pub fn format_porcelain_status(porcelain: &str) -> String {
    let mut branch = String::new();
    let mut staged: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut untracked: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    for line in porcelain.lines() {
        if let Some(b) = line.strip_prefix("## ") {
            branch = b.to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        let path = line[3..].trim();

        if x == '?' && y == '?' {
            untracked.push(path.to_string());
        } else if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
            conflicts.push(format!("{x}{y} {path}"));
        } else {
            if x != ' ' {
                staged.push(format!("{x} {path}"));
            }
            if y != ' ' {
                modified.push(format!("{y} {path}"));
            }
        }
    }

    let mut out = Vec::new();
    let is_clean =
        staged.is_empty() && modified.is_empty() && untracked.is_empty() && conflicts.is_empty();

    if !branch.is_empty() {
        if is_clean {
            out.push(format!("branch: {branch} (clean)"));
        } else {
            out.push(format!("branch: {branch}"));
        }
    } else if is_clean {
        out.push("clean".to_string());
    }

    if !conflicts.is_empty() {
        out.push(format_group("conflicts", &conflicts, 20));
    }
    if !staged.is_empty() {
        out.push(format_group("staged", &staged, 25));
    }
    if !modified.is_empty() {
        out.push(format_group("modified", &modified, 25));
    }
    if !untracked.is_empty() {
        out.push(format_group("untracked", &untracked, 20));
    }

    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn format_group(label: &str, items: &[String], cap: usize) -> String {
    let shown: Vec<&str> = items.iter().take(cap).map(|s| s.as_str()).collect();
    if items.len() > cap {
        format!(
            "{label} ({}): {}, … (+{} more)",
            items.len(),
            shown.join(", "),
            items.len() - cap
        )
    } else {
        format!("{label} ({}): {}", items.len(), shown.join(", "))
    }
}

fn run_compact_log(extra_args: &[String], log_stats: bool) -> Result<ExitCode, String> {
    let mut cmd = Command::new("git");
    cmd.arg("log").arg("--oneline");
    if !has_log_limit(extra_args) {
        cmd.args(["-n", "20"]);
    }
    cmd.args(extra_args);

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run git log: {e}"))?;
    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(&output.stderr);
    }
    let code = output.status.code().unwrap_or(1);
    if code != 0 || output.stdout.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let out_bytes = output.stdout.len();
    io::stdout()
        .write_all(&output.stdout)
        .map_err(|e| format!("write failed: {e}"))?;

    if log_stats {
        // Estimate verbose git log baseline (~3.5x oneline: commit hash + Author + Date + blank lines)
        let lines = raw_bytes_lines(&output.stdout);
        let est_raw = out_bytes + lines * 110;
        telemetry::log_cmd_event(
            "git",
            "git:log",
            "git",
            "auto",
            "trimmed",
            lines,
            est_raw,
            out_bytes,
        );
    }

    Ok(ExitCode::SUCCESS)
}

fn is_lockfile(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    LOCKFILES.contains(&name) || name.ends_with(".min.js") || name.ends_with(".min.css")
}

pub fn trim_git_diff(raw: &str, show_all: bool) -> String {
    let mut out = Vec::new();
    let mut cur_file = String::new();
    let mut skipping_file = false;
    let mut file_lines_shown = 0usize;
    let mut file_lines_omitted = 0usize;

    let flush_omitted = |out: &mut Vec<String>, file: &str, omitted: &mut usize| {
        if *omitted > 0 {
            out.push(format!(
                "… (+{} more diff lines in {file} — pass --all or --raw)",
                *omitted
            ));
            *omitted = 0;
        }
    };

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_omitted(&mut out, &cur_file, &mut file_lines_omitted);
            // Extract `b/path` from `a/path b/path`
            cur_file = rest
                .rsplit_once(" b/")
                .map(|(_, b)| b.to_string())
                .unwrap_or_else(|| rest.to_string());
            file_lines_shown = 0;
            if !show_all && is_lockfile(&cur_file) {
                skipping_file = true;
                out.push(format!(
                    "=== {cur_file} (skipped — lockfile/minified, pass --raw to view) ==="
                ));
            } else {
                skipping_file = false;
                out.push(format!("=== {cur_file} ==="));
            }
            continue;
        }

        if skipping_file {
            continue;
        }

        // Drop noisy diff metadata headers
        if line.starts_with("index ")
            || line.starts_with("--- a/")
            || line.starts_with("+++ b/")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
        {
            continue;
        }

        if !show_all && file_lines_shown >= MAX_DIFF_LINES_PER_FILE {
            file_lines_omitted += 1;
            continue;
        }

        out.push(line.to_string());
        file_lines_shown += 1;
    }

    flush_omitted(&mut out, &cur_file, &mut file_lines_omitted);

    if out.is_empty() {
        return raw.to_string();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_porcelain_status() {
        let p = "## main...origin/main [ahead 1]\nM  src/main.rs\n M src/cli.rs\n?? notes.txt\n";
        let s = format_porcelain_status(p);
        assert!(s.contains("branch: main...origin/main [ahead 1]"));
        assert!(s.contains("staged (1): M src/main.rs"));
        assert!(s.contains("modified (1): M src/cli.rs"));
        assert!(s.contains("untracked (1): notes.txt"));
    }

    #[test]
    fn trims_diff_headers_and_skips_lockfiles() {
        let diff = "\
diff --git a/Cargo.lock b/Cargo.lock
index 111..222 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,3 +1,3 @@
-old
+new
diff --git a/src/lib.rs b/src/lib.rs
index 333..444 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-let a = 1;
+let a = 2;
";
        let trimmed = trim_git_diff(diff, false);
        assert!(trimmed.contains("Cargo.lock (skipped"));
        assert!(!trimmed.contains("-old"));
        assert!(trimmed.contains("=== src/lib.rs ==="));
        assert!(trimmed.contains("+let a = 2;"));
        assert!(!trimmed.contains("index 333..444"));
    }
}
