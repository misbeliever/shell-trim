use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn trm_bin() -> &'static str {
    env!("CARGO_BIN_EXE_trm")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn run_trm(args: &[&str]) -> Vec<u8> {
    let out = Command::new(trm_bin())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("failed to execute trm");
    assert!(
        out.status.success(),
        "trm {:?} failed (status {:?}): {}",
        args,
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn assert_golden(name: &str, actual_bytes: &[u8]) {
    let expected_path = fixtures_dir()
        .join("expected")
        .join(format!("{name}.golden"));
    let actual = String::from_utf8(actual_bytes.to_vec())
        .unwrap_or_else(|_| panic!("non-UTF8 output for {name}"));

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        if let Some(parent) = expected_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&expected_path, actual.as_bytes()).unwrap();
        return;
    }

    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|e| {
        panic!(
            "missing golden file {}: {e}\nRun `UPDATE_GOLDEN=1 cargo test --test golden` to generate.",
            expected_path.display()
        )
    });

    if actual != expected {
        let mut first_diff = String::new();
        for (idx, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
            if a != e {
                first_diff = format!(
                    "first diff at line {}:\n  expected: {:?}\n  actual:   {:?}",
                    idx + 1,
                    e,
                    a
                );
                break;
            }
        }
        if first_diff.is_empty() {
            first_diff = format!(
                "line count differs (expected {}, actual {})",
                expected.lines().count(),
                actual.lines().count()
            );
        }
        panic!("golden mismatch for {name}:\n{first_diff}\n\n--- ACTUAL ---\n{actual}");
    }
}

#[test]
fn golden_snapshots_across_languages() {
    let cases: &[(&str, &str)] = &[
        ("swift_sample.swift", "DashboardView"),
        ("rust_sample.rs", "Registry"),
        ("ts_sample.ts", "UserController"),
        ("python_sample.py", "run"),
        ("go_sample.go", "Handle"),
        ("crlf_sample.rs", "compute"),
    ];

    for (file, symbol) in cases {
        let rel = format!("tests/fixtures/{file}");
        let p = rel.as_str();

        let outline = run_trm(&["read", p, "--outline", "--no-stats"]);
        assert_golden(&format!("{file}.outline"), &outline);

        let minimal = run_trm(&["read", p, "--minimal", "--no-stats"]);
        assert_golden(&format!("{file}.minimal"), &minimal);

        let aggressive = run_trm(&["read", p, "--aggressive", "--no-stats"]);
        assert_golden(&format!("{file}.aggressive"), &aggressive);

        let sym_out = run_trm(&["read", p, "-s", symbol, "--no-stats"]);
        assert_golden(&format!("{file}.symbol_{symbol}"), &sym_out);
    }
}

#[test]
fn crlf_slice_and_symbol_preserve_exact_crlf_bytes() {
    let p = "tests/fixtures/crlf_sample.rs";

    let sym_out = run_trm(&["read", p, "-s", "compute", "--no-stats"]);
    let sym_str = String::from_utf8(sym_out).unwrap();
    assert!(
        sym_str.contains("\r\n"),
        "symbol read must preserve CRLF line endings: {sym_str:?}"
    );
    assert!(
        sym_str.contains("a + b\r\n}\r\n"),
        "expected exact CRLF body in symbol output: {sym_str:?}"
    );

    let range_out = run_trm(&["read", p, "-r", "3-6", "--no-stats"]);
    assert_eq!(
        String::from_utf8(range_out).unwrap(),
        "pub fn compute(a: i32, b: i32) -> i32 {\r\n    // inner comment\r\n    a + b\r\n}\r\n"
    );
}
