use std::io::{self, Write};
use std::process::{Command, ExitCode};

use crate::cli::ExecTrimOptions;
use crate::telemetry;

pub fn run(bin: &str, opts: &ExecTrimOptions) -> Result<ExitCode, String> {
    let output = Command::new(bin).args(&opts.args).output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            format!("'{bin}' not found in PATH")
        } else {
            format!("failed to run {bin}: {e}")
        }
    })?;

    let code = output.status.code().unwrap_or(1);
    let combined = crate::cargo_cmd::combine_streams(&output.stdout, &output.stderr);
    if combined.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = combined.len();
    let subcmd = if bin == "swiftc" {
        "swiftc"
    } else {
        opts.args.first().map(|s| s.as_str()).unwrap_or("")
    };

    let (rendered, eff_mode) = if opts.raw {
        (combined.clone(), "raw")
    } else {
        let trimmed = match subcmd {
            "test" => trim_swift_test(&combined, code == 0),
            "build" | "swiftc" => trim_swift_build(&combined, code == 0),
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
        let key = format!("{bin}:{subcmd}");
        let req_mode = if opts.raw { "raw" } else { "auto" };
        let total_lines = combined.lines().count().max(1);
        telemetry::log_cmd_event(
            "swift",
            &key,
            "swift",
            req_mode,
            eff_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

fn is_build_noise(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    // SwiftPM progress lines: `[12/54] Compiling ...`, `[4/10] Linking ...`
    if trimmed.starts_with('[') && trimmed.contains("] ") {
        let rest = &trimmed[1..];
        if let Some((ratio, _)) = rest.split_once(']')
            && ratio.contains('/')
            && ratio.bytes().all(|b| b.is_ascii_digit() || b == b'/')
        {
            return true;
        }
    }
    trimmed.starts_with("Building for ")
        || trimmed.starts_with("Planning build")
        || trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Linking ")
        || trimmed.starts_with("Emitting module ")
        || trimmed.starts_with("Write swift-version-")
        || trimmed.starts_with("Write auxiliary file ")
        || trimmed.starts_with("Copy ")
        || trimmed.starts_with("Merging module ")
        || trimmed.starts_with("Wrapping AST ")
        || trimmed.starts_with("Fetching ")
        || trimmed.starts_with("Computed ")
        || trimmed.starts_with("Creating working copy ")
        || trimmed.starts_with("Working copy of ")
}

fn is_diagnostic_header(line: &str) -> bool {
    line.contains(": error:")
        || line.contains(": warning:")
        || line.contains(": note:")
        || line.starts_with("error:")
        || line.starts_with("warning:")
        || line.starts_with("fatal error:")
}

pub fn trim_swift_build(raw: &str, success: bool) -> String {
    let mut out = Vec::new();
    let mut in_diag_block = false;
    let mut diag_context_left = 0usize;
    let mut summary_line: Option<&str> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Build complete!") {
            summary_line = Some(trimmed);
            in_diag_block = false;
            continue;
        }
        if is_diagnostic_header(trimmed) {
            out.push(line);
            in_diag_block = true;
            diag_context_left = 4; // keep source snippet + caret lines
            continue;
        }
        if in_diag_block && diag_context_left > 0 {
            if is_build_noise(trimmed) {
                in_diag_block = false;
                continue;
            }
            out.push(line);
            diag_context_left -= 1;
            continue;
        }
        if !success && !is_build_noise(trimmed) {
            out.push(line);
        }
    }

    if let Some(sum) = summary_line {
        out.push(sum);
    } else if out.is_empty() && success {
        out.push("Build succeeded.");
    }

    if out.is_empty() {
        return raw.to_string();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn trim_swift_test(raw: &str, success: bool) -> String {
    let mut out = Vec::new();
    let mut in_diag_block = false;
    let mut diag_context_left = 0usize;
    let mut in_failing_test = false;
    let mut xctest_summary: Option<&str> = None;
    let mut swift_testing_summary: Option<&str> = None;
    let mut passed_count = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();

        // Swift Testing pass/start lines (`◇ Test ...`, `✔ Test ...`, `◇ Suite ...`, `✔ Suite ...`)
        if trimmed.starts_with("✔ Test run with ") || trimmed.starts_with("✘ Test run with ") {
            swift_testing_summary = Some(trimmed);
            continue;
        }
        if trimmed.starts_with("◇ ") || trimmed.starts_with("✔ ") {
            if trimmed.starts_with("✔ Test ") {
                passed_count += 1;
            }
            continue;
        }
        if trimmed.starts_with("✘ ") {
            out.push(line);
            in_failing_test = true;
            continue;
        }

        // XCTest lines
        if trimmed.starts_with("Test Suite 'All tests' ")
            || (trimmed.starts_with("Executed ") && trimmed.contains("test"))
        {
            if trimmed.starts_with("Executed ") {
                xctest_summary = Some(trimmed);
            }
            in_failing_test = false;
            continue;
        }
        if trimmed.starts_with("Test Suite '") {
            in_failing_test = false;
            continue;
        }
        if trimmed.starts_with("Test Case '") {
            if trimmed.contains(" passed ") {
                passed_count += 1;
                in_failing_test = false;
                continue;
            }
            if trimmed.contains(" started.") {
                in_failing_test = false;
                continue;
            }
            if trimmed.contains(" failed ") {
                out.push(line);
                in_failing_test = false;
                continue;
            }
        }

        // Compiler / assertion errors
        if is_diagnostic_header(trimmed) {
            out.push(line);
            in_diag_block = true;
            diag_context_left = 4;
            continue;
        }
        if in_diag_block && diag_context_left > 0 {
            if is_build_noise(trimmed) {
                in_diag_block = false;
                continue;
            }
            out.push(line);
            diag_context_left -= 1;
            continue;
        }

        if is_build_noise(trimmed) || trimmed.starts_with("Build complete!") {
            continue;
        }

        if in_failing_test
            || (!success
                && (trimmed.contains("XCTAssert")
                    || trimmed.contains("Expectation failed")
                    || trimmed.contains("failed -")))
        {
            out.push(line);
        }
    }

    if let Some(s) = xctest_summary {
        out.push(s);
    }
    if let Some(s) = swift_testing_summary {
        out.push(s);
    }
    if out.is_empty() && success {
        if passed_count > 0 {
            return format!("All {passed_count} tests passed.\n");
        }
        return "Tests passed.\n".to_string();
    }
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
    fn trims_swift_build_noise_and_keeps_errors() {
        let raw = "\
Building for debugging...
[1/12] Compiling DictationCore Backend.swift
[2/12] Compiling MurMew App.swift
Sources/App.swift:12:9: error: cannot find 'foo' in scope
        foo()
        ^~~
[3/12] Linking MurMew
";
        let trimmed = trim_swift_build(raw, false);
        assert!(trimmed.contains("error: cannot find 'foo' in scope"));
        assert!(trimmed.contains("foo()"));
        assert!(!trimmed.contains("[1/12] Compiling"));
        assert!(!trimmed.contains("Building for debugging"));
    }

    #[test]
    fn trims_xctest_and_swift_testing_passes_but_keeps_failures() {
        let raw = "\
Building for debugging...
[1/4] Compiling Tests
Build complete! (0.42s)
Test Suite 'All tests' started at 2026-11-26
Test Case '-[FooTests testOne]' started.
Test Case '-[FooTests testOne]' passed (0.001 seconds).
Test Case '-[FooTests testTwo]' started.
Sources/Tests.swift:42: error: -[FooTests testTwo] : XCTAssertEqual failed: (\"1\") is not equal to (\"2\")
Test Case '-[FooTests testTwo]' failed (0.003 seconds).
Executed 2 tests, with 1 failure (0 unexpected) in 0.004 (0.005) seconds
";
        let trimmed = trim_swift_test(raw, false);
        assert!(!trimmed.contains("testOne"));
        assert!(!trimmed.contains("[1/4] Compiling"));
        assert!(trimmed.contains("XCTAssertEqual failed"));
        assert!(trimmed.contains("testTwo]' failed"));
        assert!(trimmed.contains("Executed 2 tests, with 1 failure"));
    }
}
