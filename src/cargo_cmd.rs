use std::io::{self, Write};
use std::process::{Command, ExitCode};

use crate::cli::ExecTrimOptions;
use crate::telemetry;

pub fn run(opts: &ExecTrimOptions) -> Result<ExitCode, String> {
    let output = Command::new("cargo")
        .args(&opts.args)
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "'cargo' not found in PATH".to_string()
            } else {
                format!("failed to run cargo: {e}")
            }
        })?;

    let code = output.status.code().unwrap_or(1);
    let combined = combine_streams(&output.stdout, &output.stderr);
    if combined.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = combined.len();
    let subcmd = opts.args.first().map(|s| s.as_str()).unwrap_or("");

    let (rendered, eff_mode) = if opts.raw {
        (combined.clone(), "raw")
    } else {
        let trimmed = match subcmd {
            "test" => trim_cargo_test(&combined, code == 0),
            "build" | "check" | "clippy" | "b" | "c" => trim_cargo_build(&combined, code == 0),
            _ => combined.clone(),
        };
        if trimmed.len() >= raw_bytes {
            (combined.clone(), "raw")
        } else {
            (trimmed, "trimmed")
        }
    };

    let out_bytes = rendered.len();
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if opts.log_stats {
        let key = format!("cargo:{subcmd}");
        let req_mode = if opts.raw { "raw" } else { "auto" };
        let total_lines = combined.lines().count().max(1);
        telemetry::log_cmd_event(
            "cargo",
            &key,
            "rust",
            req_mode,
            eff_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

pub(crate) fn combine_streams(stdout: &[u8], stderr: &[u8]) -> String {
    let mut s = String::with_capacity(stdout.len() + stderr.len() + 1);
    if !stdout.is_empty() {
        s.push_str(&String::from_utf8_lossy(stdout));
    }
    if !stderr.is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&String::from_utf8_lossy(stderr));
    }
    s
}

fn is_cargo_progress(trimmed: &str) -> bool {
    trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Checking ")
        || trimmed.starts_with("Downloading ")
        || trimmed.starts_with("Downloaded ")
        || trimmed.starts_with("Updating ")
        || trimmed.starts_with("Locking ")
        || trimmed.starts_with("Adding ")
        || trimmed.starts_with("Fresh ")
        || trimmed.starts_with("Blocking waiting for file lock")
        || trimmed.starts_with("Running unittests ")
        || trimmed.starts_with("Running tests/")
        || trimmed.starts_with("Running `")
        || trimmed.starts_with("Doc-tests ")
}

pub fn trim_cargo_build(raw: &str, success: bool) -> String {
    let mut out = Vec::new();
    let mut finished_line: Option<&str> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() && out.is_empty() {
            continue;
        }
        if trimmed.starts_with("Finished ") {
            finished_line = Some(trimmed);
            continue;
        }
        if is_cargo_progress(trimmed) {
            continue;
        }
        out.push(line);
    }

    // Trim trailing blanks
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }

    if let Some(fin) = finished_line {
        out.push(fin);
    } else if out.is_empty() && success {
        out.push("Finished.");
    }

    if out.is_empty() {
        return raw.to_string();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn trim_cargo_test(raw: &str, success: bool) -> String {
    let mut out = Vec::new();
    let mut in_failures_section = false;
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut total_ignored = 0usize;
    let mut saw_result = false;

    for line in raw.lines() {
        let trimmed = line.trim();

        if is_cargo_progress(trimmed) || trimmed.starts_with("Finished ") {
            continue;
        }
        if trimmed.starts_with("running ") && trimmed.ends_with(" tests")
            || trimmed == "running 0 tests"
            || trimmed == "running 1 test"
        {
            continue;
        }
        if trimmed.starts_with("test ") && trimmed.ends_with("... ok") {
            continue;
        }
        if trimmed.starts_with("test ") && trimmed.ends_with("... ignored") {
            continue;
        }

        // Aggregate `test result: ok. 15 passed; 0 failed; 0 ignored; ...`
        if trimmed.starts_with("test result:") {
            saw_result = true;
            in_failures_section = false;
            if let Some((p, f, ig)) = parse_test_result_counts(trimmed) {
                total_passed += p;
                total_failed += f;
                total_ignored += ig;
            }
            continue;
        }

        if trimmed == "failures:" {
            in_failures_section = true;
            out.push(line);
            continue;
        }

        if in_failures_section {
            out.push(line);
            continue;
        }

        // Keep compiler warnings/errors or failing test lines
        if !trimmed.is_empty()
            && (trimmed.starts_with("error")
                || trimmed.starts_with("warning:")
                || trimmed.starts_with("-->")
                || trimmed.contains("FAILED")
                || !success)
        {
            out.push(line);
        }
    }

    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }

    if saw_result {
        let status = if total_failed == 0 && success {
            "ok"
        } else {
            "FAILED"
        };
        let summary = if total_ignored > 0 {
            format!(
                "test result: {status}. {total_passed} passed; {total_failed} failed; {total_ignored} ignored"
            )
        } else {
            format!("test result: {status}. {total_passed} passed; {total_failed} failed")
        };
        let mut s = if out.is_empty() {
            summary
        } else {
            format!("{}\n{summary}", out.join("\n"))
        };
        s.push('\n');
        return s;
    }

    if out.is_empty() {
        return raw.to_string();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn parse_test_result_counts(line: &str) -> Option<(usize, usize, usize)> {
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut ignored = 0usize;
    for part in line.split(';') {
        let tokens: Vec<&str> = part.split_whitespace().collect();
        for w in tokens.windows(2) {
            if let Ok(n) = w[0].parse::<usize>() {
                match w[1].trim_end_matches('.') {
                    "passed" => passed = n,
                    "failed" => failed = n,
                    "ignored" => ignored = n,
                    _ => {}
                }
            }
        }
    }
    Some((passed, failed, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_cargo_build_strips_progress_and_keeps_warnings() {
        let raw = "\
   Compiling serde v1.0.0
   Checking trm v0.1.0 (/tmp/trm)
warning: unused variable: `x`
 --> src/main.rs:10:9
  |
10|     let x = 1;
  |         ^
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.42s
";
        let out = trim_cargo_build(raw, true);
        assert!(!out.contains("Compiling serde"));
        assert!(!out.contains("Checking trm"));
        assert!(out.contains("warning: unused variable"));
        assert!(out.contains("Finished `dev` profile"));
    }

    #[test]
    fn trim_cargo_test_aggregates_passing_and_keeps_failures() {
        let passing = "\
   Compiling trm v0.1.0
    Finished `test` profile
     Running unittests src/main.rs
running 2 tests
test a ... ok
test b ... ignored
test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
     Running tests/integration.rs
running 2 tests
test c ... ok
test d ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        let out = trim_cargo_test(passing, true);
        assert_eq!(out, "test result: ok. 3 passed; 0 failed; 1 ignored\n");

        let failing = "\
running 2 tests
test a ... ok
test b ... FAILED

failures:

---- b stdout ----
thread 'b' panicked at 'boom'

failures:
    b

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
";
        let fail_out = trim_cargo_test(failing, false);
        assert!(!fail_out.contains("test a ... ok"));
        assert!(fail_out.contains("test b ... FAILED"));
        assert!(fail_out.contains("panicked at 'boom'"));
        assert!(fail_out.contains("test result: FAILED. 1 passed; 1 failed"));
    }
}
