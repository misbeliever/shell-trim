use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn trm_bin() -> &'static str {
    env!("CARGO_BIN_EXE_trm")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trm_test_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn swift_nested_comments_and_strings_preserved() {
    let dir = temp_dir("swift_comments");
    let file = dir.join("Sample.swift");
    let src = r#"import Foundation

/// Doc comment that must stay in minimal mode.
// Regular comment to strip
/* Outer comment
   /* Nested comment */
   still inside outer comment
*/
// MARK: - Networking
struct Client {
    let url = "https://example.com/api/*not-a-comment*/"
    // inline comment
    func fetch() -> String {
        return "ok // not a comment"
    }
}
"#;
    fs::write(&file, src).unwrap();

    let out = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--minimal", "--no-stats"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();

    assert!(stdout.contains("/// Doc comment that must stay in minimal mode."));
    assert!(stdout.contains("// MARK: - Networking"));
    assert!(stdout.contains("https://example.com/api/*not-a-comment*/"));
    assert!(stdout.contains("return \"ok // not a comment\""));
    assert!(!stdout.contains("Regular comment to strip"));
    assert!(!stdout.contains("Nested comment"));
    assert!(!stdout.contains("inline comment"));

    // Aggressive strips doc comments and MARK too
    let agg = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--aggressive", "--no-stats"])
        .output()
        .unwrap();
    assert!(agg.status.success());
    let agg_out = String::from_utf8(agg.stdout).unwrap();
    assert!(!agg_out.contains("/// Doc comment"));
    assert!(!agg_out.contains("MARK:"));
    assert!(agg_out.contains("https://example.com/api/*not-a-comment*/"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn swift_outline_extracts_types_marks_and_line_ranges() {
    let dir = temp_dir("swift_outline");
    let file = dir.join("BigView.swift");
    let mut src = String::from(
        "import Foundation\nimport SwiftUI\nimport AppKit\n\n// MARK: - Main\n@MainActor\nfinal class ChatStore: ObservableObject {\n",
    );
    src.push_str("    func loadMessages() async {\n");
    for i in 0..10 {
        src.push_str(&format!("        let x{i} = {i}\n"));
    }
    src.push_str("    }\n");
    src.push_str("    private func reset() {\n        print(\"hi\")\n    }\n}\n");
    fs::write(&file, &src).unwrap();

    let out = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--outline", "--no-stats"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();

    assert!(stdout.contains("// 3 imports (Foundation, SwiftUI, AppKit)"));
    assert!(stdout.contains("// MARK: - Main"));
    assert!(stdout.contains("final class ChatStore: ObservableObject"));
    assert!(stdout.contains("func loadMessages() async"));
    assert!(stdout.contains("private func reset()"));
    assert!(!stdout.contains("let x0 = 0"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn python_comments_docstrings_and_outline() {
    let dir = temp_dir("py_test");
    let file = dir.join("worker.py");
    let src = r#"#!/usr/bin/env python3
import os
from pathlib import Path

# strip me
class Runner:
    """Keeps docstring in minimal, strips in aggressive."""

    def run(self, url: str = "http://x/#frag"):
        # inner comment
        return url
"#;
    fs::write(&file, src).unwrap();

    let min = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--minimal", "--no-stats"])
        .output()
        .unwrap();
    let min_s = String::from_utf8(min.stdout).unwrap();
    assert!(min_s.contains("#!/usr/bin/env python3"));
    assert!(min_s.contains("Keeps docstring in minimal"));
    assert!(min_s.contains("http://x/#frag"));
    assert!(!min_s.contains("# strip me"));
    assert!(!min_s.contains("# inner comment"));

    let agg = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--aggressive", "--no-stats"])
        .output()
        .unwrap();
    let agg_s = String::from_utf8(agg.stdout).unwrap();
    assert!(!agg_s.contains("Keeps docstring"));
    assert!(agg_s.contains("http://x/#frag"));

    let outline = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--outline", "--no-stats"])
        .output()
        .unwrap();
    let out_s = String::from_utf8(outline.stdout).unwrap();
    assert!(out_s.contains("# 2 imports"));
    assert!(out_s.contains("class Runner:"));
    assert!(out_s.contains("def run("));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn range_slicing_and_auto_mode_thresholds() {
    let dir = temp_dir("range_auto");
    let file = dir.join("big.rs");
    let mut src = String::from("use std::io;\nuse std::fs;\n\n");
    for f in 0..40 {
        src.push_str(&format!("// comment for fn_{f}\npub fn fn_{f}() {{\n"));
        for j in 0..8 {
            src.push_str(&format!("    let v{j} = {j};\n"));
        }
        src.push_str("}\n\n");
    }
    fs::write(&file, &src).unwrap();

    // Auto mode on >300 line Rust file -> outline
    let auto_out = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(auto_out.status.success());
    let auto_s = String::from_utf8(auto_out.stdout).unwrap();
    assert!(auto_s.contains("[trm: outline of"));
    assert!(auto_s.contains("pub fn fn_0()"));

    // Explicit range -> raw (keeps comments & exact content; -L adds line numbers)
    let range_out = Command::new(trm_bin())
        .args([
            "read",
            file.to_str().unwrap(),
            "-r",
            "4-10",
            "-L",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(range_out.status.success());
    let range_s = String::from_utf8(range_out.stdout).unwrap();
    assert!(range_s.contains(" 4| // comment for fn_0"));
    assert!(range_s.contains(" 5| pub fn fn_0() {"));
    assert!(!range_s.contains("fn_1"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn binary_file_is_safely_skipped() {
    let dir = temp_dir("binary");
    let file = dir.join("blob.bin");
    fs::write(&file, [0x89, b'P', b'N', b'G', 0x00, 0x01, 0x02]).unwrap();

    let out = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[trm: binary file"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn telemetry_tracks_gross_net_drilldowns_and_misses() {
    let dir = temp_dir("telemetry");
    let log = dir.join("events.jsonl");
    let file = dir.join("large.swift");
    let mut src = String::from("import Foundation\n\n");
    for i in 0..40 {
        src.push_str(&format!("// helper {i}\nfunc f{i}() {{\n"));
        for j in 0..8 {
            src.push_str(&format!("    let x{j} = {j}\n"));
        }
        src.push_str("}\n");
    }
    fs::write(&file, &src).unwrap();

    // 1. Outline read (auto on >300 lines)
    let r1 = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(r1.status.success());

    // 2. Targeted raw slice (<50% of file) -> outline drill-down, NOT a miss!
    let r2 = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", file.to_str().unwrap(), "-r", "3-15"])
        .output()
        .unwrap();
    assert!(r2.status.success());

    // 3. Minimal read followed immediately by full raw read -> trim miss!
    let r3 = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", file.to_str().unwrap(), "--minimal"])
        .output()
        .unwrap();
    assert!(r3.status.success());

    let r4 = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();
    assert!(r4.status.success());

    let stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--json"])
        .output()
        .unwrap();
    assert!(stats.status.success());
    let json = String::from_utf8(stats.stdout).unwrap();
    assert!(json.contains("\"events\":4"), "got: {json}");
    assert!(json.contains("\"outline_drilldowns\":1"), "got: {json}");
    assert!(json.contains("\"trim_misses\":1"), "got: {json}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn telemetry_scopes_misses_per_agent_and_model() {
    let dir = temp_dir("telemetry_agents");
    let log = dir.join("events.jsonl");
    let file = dir.join("large.swift");
    let mut src = String::from("import Foundation\n\n");
    for i in 0..30 {
        src.push_str(&format!("// helper {i}\nfunc f{i}() {{\n"));
        for j in 0..8 {
            src.push_str(&format!("    let x{j} = {j}\n"));
        }
        src.push_str("}\n");
    }
    fs::write(&file, &src).unwrap();

    // Codex reads minimal
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .env("TRM_MODEL", "openai/gpt-5.4")
        .args(["read", file.to_str().unwrap(), "--minimal"])
        .output()
        .unwrap();

    // Antigravity reads raw right after -> must NOT count as a miss for Codex!
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "antigravity")
        .env("TRM_MODEL", "google/gemini-pro")
        .args(["read", file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();

    let stats_no_miss = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--json"])
        .output()
        .unwrap();
    let j1 = String::from_utf8(stats_no_miss.stdout).unwrap();
    assert!(
        j1.contains("\"trim_misses\":0"),
        "cross-agent false miss: {j1}"
    );

    // Now Codex itself re-reads raw -> SHOULD count as 1 miss for Codex only
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .env("TRM_MODEL", "openai/gpt-5.4")
        .args(["read", file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();

    let codex_stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--agent", "codex", "--json"])
        .output()
        .unwrap();
    let j2 = String::from_utf8(codex_stats.stdout).unwrap();
    assert!(j2.contains("\"events\":2"), "got: {j2}");
    assert!(j2.contains("\"trim_misses\":1"), "got: {j2}");

    let agy_stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--agent", "antigravity", "--json"])
        .output()
        .unwrap();
    let j3 = String::from_utf8(agy_stats.stdout).unwrap();
    assert!(j3.contains("\"trim_misses\":0"), "got: {j3}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_counts_adoption_and_ignores_pipes_heredocs_and_sed_edits() {
    let dir = temp_dir("audit");
    let log = dir.join("rollout-test.jsonl");
    let lines = [
        r#"{"type":"event_msg","payload":{"thread_settings":{"model":"gpt-5.4"}}}"#,
        r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd:\"trm read Foo.swift && cat Bar.swift && sed -n '1,50p' Baz.swift\"})"}}"#,
        r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd:\"rg foo | head -n 10 && cat <<EOF > out.txt && sed -i '' 's/a/b/' x.txt && git log --format=%h\"})"}}"#,
    ];
    fs::write(&log, lines.join("\n")).unwrap();

    let out = Command::new(trm_bin())
        .env("TRM_AUDIT_DIR", format!("codex:{}", dir.display()))
        .args(["audit", "--since", "all", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json = String::from_utf8(out.stdout).unwrap();
    assert!(json.contains("\"trm_read\":1"), "got: {json}");
    assert!(json.contains("\"cat\":1"), "got: {json}");
    assert!(json.contains("\"sed_read\":1"), "got: {json}");
    assert!(
        json.contains("\"head\":0"),
        "piped head should not count: {json}"
    );
    assert!(
        json.contains("\"rg\":1"),
        "rg should now be tracked as search bypass: {json}"
    );
    assert!(json.contains("\"bypasses\":3"), "got: {json}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn head_tail_and_max_chars_work() {
    let dir = temp_dir("head_tail");
    let file = dir.join("events.jsonl");
    let mut lines = Vec::new();
    for i in 1..=20 {
        lines.push(format!("{{\"line\":{i},\"pad\":\"{}\"}}", "x".repeat(80)));
    }
    fs::write(&file, lines.join("\n")).unwrap();

    let h = Command::new(trm_bin())
        .args([
            "read",
            file.to_str().unwrap(),
            "--head",
            "3",
            "-c",
            "30",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(h.status.success());
    let hs = String::from_utf8(h.stdout).unwrap();
    assert_eq!(hs.lines().count(), 3);
    assert!(hs.contains("\"line\":1"));
    assert!(hs.contains("… [+"));

    let t = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--tail", "2", "--no-stats"])
        .output()
        .unwrap();
    assert!(t.status.success());
    let ts = String::from_utf8(t.stdout).unwrap();
    assert_eq!(ts.lines().count(), 2);
    assert!(ts.contains("\"line\":19"));
    assert!(ts.contains("\"line\":20"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn install_and_uninstall_preserve_existing_user_rules() {
    let fake_home = temp_dir("install_home");
    fs::create_dir_all(fake_home.join(".codex")).unwrap();
    fs::create_dir_all(fake_home.join(".gemini/config")).unwrap();
    fs::create_dir_all(fake_home.join(".claude")).unwrap();

    let codex_rules = fake_home.join(".codex/AGENTS.md");
    fs::write(&codex_rules, "# My existing Codex rules\nKeep me!\n").unwrap();

    // Install
    let inst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["install"])
        .output()
        .unwrap();
    assert!(inst.status.success());

    let codex_after = fs::read_to_string(&codex_rules).unwrap();
    assert!(codex_after.contains("# My existing Codex rules\nKeep me!"));
    assert!(codex_after.contains("<!-- trm:start -->"));
    assert!(fake_home.join(".gemini/config/GEMINI.md").exists());
    assert!(fake_home.join(".claude/CLAUDE.md").exists());

    // Idempotent second install
    let inst2 = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["install"])
        .output()
        .unwrap();
    assert!(inst2.status.success());
    assert_eq!(fs::read_to_string(&codex_rules).unwrap(), codex_after);

    // Uninstall cleanly removes only trm block & files
    let uninst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["uninstall"])
        .output()
        .unwrap();
    assert!(uninst.status.success());

    let codex_restored = fs::read_to_string(&codex_rules).unwrap();
    assert_eq!(codex_restored, "# My existing Codex rules\nKeep me!\n");
    assert!(!fake_home.join(".gemini/config/GEMINI.md").exists());
    assert!(!fake_home.join(".claude/CLAUDE.md").exists());

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn rg_groups_caps_truncates_and_tracks_misses() {
    let dir = temp_dir("rg_search");
    let log = dir.join("events.jsonl");

    // Create 3 files with many matches + one huge minified line
    for f in 1..=3 {
        let mut content = String::new();
        for l in 1..=12 {
            content.push_str(&format!(
                "    func needle_{f}_{l}() -> Int {{ return {l} }}\n"
            ));
        }
        content.push_str(&format!("let wide = \"needle {}\"\n", "x".repeat(400)));
        fs::write(dir.join(format!("File{f}.swift")), content).unwrap();
    }

    // Grouped & capped search (default per-file=6)
    let out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .args(["rg", "needle", dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("(13)"), "expected grouped file match count: {s}");
    assert!(s.contains("… (+7 more)"), "expected per-file cap: {s}");

    // Alias `trm grep` with --max-chars
    let g = Command::new(trm_bin())
        .args([
            "grep",
            "--max-chars",
            "60",
            "--no-stats",
            "wide",
            dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(g.status.success());
    let gs = String::from_utf8(g.stdout).unwrap();
    assert!(gs.contains("… [+"), "expected long-line truncation: {gs}");

    // Follow-up `--all` on same pattern by same agent -> counts as 1 trim miss
    let all = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .args(["rg", "--all", "needle", dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(all.status.success());

    let stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--json"])
        .output()
        .unwrap();
    let sj = String::from_utf8(stats.stdout).unwrap();
    assert!(sj.contains("\"events\":2"), "got: {sj}");
    assert!(sj.contains("\"trim_misses\":1"), "got: {sj}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_tracks_rg_and_grep_adoption_without_counting_piped_grep() {
    let dir = temp_dir("audit_rg");
    let log = dir.join("rollout-rg.jsonl");
    let lines = [
        r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd:\"trm rg foo src/ && rg bar src/ && grep -rn baz src/ && ls -la | grep swift\"})"}}"#,
    ];
    fs::write(&log, lines.join("\n")).unwrap();

    let out = Command::new(trm_bin())
        .env("TRM_AUDIT_DIR", format!("codex:{}", dir.display()))
        .args(["audit", "--since", "all", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let j = String::from_utf8(out.stdout).unwrap();
    assert!(j.contains("\"trm_rg\":1"), "got: {j}");
    assert!(j.contains("\"rg\":1"), "got: {j}");
    assert!(j.contains("\"grep\":1"), "piped grep must not count: {j}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn symbol_jump_and_multi_file_read_work() {
    let dir = temp_dir("sym_multi");
    let f1 = dir.join("Alpha.swift");
    let f2 = dir.join("Beta.swift");
    fs::write(
        &f1,
        "import Foundation\n\nstruct Alpha {\n    func targetOp() -> Int {\n        return 42\n    }\n    func otherOp() -> Int {\n        return 0\n    }\n}\n",
    )
    .unwrap();
    fs::write(&f2, "let beta = 99\n").unwrap();

    // Symbol jump extracts only targetOp raw body
    let sym = Command::new(trm_bin())
        .args(["read", f1.to_str().unwrap(), "-s", "targetOp", "--no-stats"])
        .output()
        .unwrap();
    assert!(sym.status.success());
    let ss = String::from_utf8(sym.stdout).unwrap();
    assert!(ss.contains("[L4-L6]"), "got: {ss}");
    assert!(ss.contains("return 42"), "got: {ss}");
    assert!(!ss.contains("otherOp"), "should not include sibling: {ss}");

    // Multi-file read emits headers and both contents
    let multi = Command::new(trm_bin())
        .args([
            "read",
            f1.to_str().unwrap(),
            f2.to_str().unwrap(),
            "--head",
            "1",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(multi.status.success());
    let ms = String::from_utf8(multi.stdout).unwrap();
    assert!(ms.contains("Alpha.swift <=="), "got: {ms}");
    assert!(ms.contains("Beta.swift <=="), "got: {ms}");
    assert!(ms.contains("import Foundation"));
    assert!(ms.contains("let beta = 99"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stats_recommend_flags_high_miss_rate() {
    let dir = temp_dir("recommend");
    let log = dir.join("events.jsonl");
    let file = dir.join("Mid.swift");
    let mut src = String::from("import Foundation\n\n");
    for i in 0..30 {
        src.push_str(&format!("func f{i}() {{\n"));
        for j in 0..8 {
            src.push_str(&format!("    let v{j} = {j}\n"));
        }
        src.push_str("}\n");
    }
    fs::write(&file, &src).unwrap();

    // Trigger 3 outline -> full raw misses by same agent
    for _ in 0..3 {
        let _ = Command::new(trm_bin())
            .env("TRM_LOG_PATH", &log)
            .env("TRM_AGENT", "codex")
            .args(["read", file.to_str().unwrap()])
            .output()
            .unwrap();
        let _ = Command::new(trm_bin())
            .env("TRM_LOG_PATH", &log)
            .env("TRM_AGENT", "codex")
            .args(["read", file.to_str().unwrap(), "--raw"])
            .output()
            .unwrap();
    }

    let out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--recommend"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Recommendations:"), "got: {s}");
    assert!(s.contains("Agent 'codex' has a"), "got: {s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn copilot_and_pi_install_uninstall_and_audit() {
    let fake_home = temp_dir("copilot_pi");
    fs::create_dir_all(fake_home.join(".copilot")).unwrap();
    fs::create_dir_all(fake_home.join(".pi/agent")).unwrap();

    // Install into Copilot + Pi
    let inst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["install"])
        .output()
        .unwrap();
    assert!(inst.status.success());
    assert!(fake_home.join(".copilot/copilot-instructions.md").exists());
    assert!(fake_home.join(".pi/agent/AGENTS.md").exists());

    // Uninstall cleanly removes both
    let uninst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["uninstall"])
        .output()
        .unwrap();
    assert!(uninst.status.success());
    assert!(!fake_home.join(".copilot/copilot-instructions.md").exists());
    assert!(!fake_home.join(".pi/agent/AGENTS.md").exists());

    // Audit Copilot events.jsonl
    let copilot_logs = fake_home.join("cop_logs");
    fs::create_dir_all(&copilot_logs).unwrap();
    fs::write(
        copilot_logs.join("events.jsonl"),
        r#"{"type":"tool.execution_start","data":{"toolName":"bash","arguments":{"command":"trm read Foo.swift && cat Bar.swift"},"model":"kimi-k3"}}"#,
    )
    .unwrap();
    let c_audit = Command::new(trm_bin())
        .env(
            "TRM_AUDIT_DIR",
            format!("copilot:{}", copilot_logs.display()),
        )
        .args(["audit", "--since", "all", "--json"])
        .output()
        .unwrap();
    let cj = String::from_utf8(c_audit.stdout).unwrap();
    assert!(cj.contains("\"trm_read\":1"), "got: {cj}");
    assert!(cj.contains("\"cat\":1"), "got: {cj}");

    // Audit Pi sessions JSONL
    let pi_logs = fake_home.join("pi_logs");
    fs::create_dir_all(&pi_logs).unwrap();
    fs::write(
        pi_logs.join("session.jsonl"),
        r#"{"type":"message","message":{"role":"assistant","model":"claude-sonnet","content":[{"type":"toolCall","id":"1","name":"bash","arguments":{"command":"trm rg foo src/ && rg bar src/"}}]}}"#,
    )
    .unwrap();
    let p_audit = Command::new(trm_bin())
        .env("TRM_AUDIT_DIR", format!("pi:{}", pi_logs.display()))
        .args(["audit", "--since", "all", "--json"])
        .output()
        .unwrap();
    let pj = String::from_utf8(p_audit.stdout).unwrap();
    assert!(pj.contains("\"trm_rg\":1"), "got: {pj}");
    assert!(pj.contains("\"rg\":1"), "got: {pj}");

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn cargo_test_trimming_works_live_and_preserves_summary() {
    // Use an isolated CARGO_TARGET_DIR so the nested `cargo test` never re-links
    // `target/debug/trm` while sibling integration tests are executing it in parallel.
    let target_dir = temp_dir("cargo_trim_target");
    let out = Command::new(trm_bin())
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["cargo", "test", "--bin", "trm", "--no-stats"])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&target_dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("test result: ok."), "got: {s}");
    assert!(
        !s.contains("Compiling "),
        "should strip Compiling noise: {s}"
    );
    assert!(
        !s.contains("... ok"),
        "should strip passing test lines: {s}"
    );
}

#[test]
fn stats_resolves_unknown_model_from_session_transcript() {
    let fake_home = temp_dir("model_resolve");
    let log = fake_home.join("events.jsonl");
    let session_id = "test-conv-1234";
    let transcript_dir = fake_home
        .join(".gemini/antigravity-cli/brain")
        .join(session_id)
        .join(".system_generated/logs");
    fs::create_dir_all(&transcript_dir).unwrap();
    fs::write(
        transcript_dir.join("transcript.jsonl"),
        "{\"type\":\"PLANNER_RESPONSE\",\"model\":\"gemini-4-neon-medium\"}\n",
    )
    .unwrap();

    // Write an event with model="unknown" and session=session_id
    let sample = fake_home.join("Sample.swift");
    fs::write(&sample, "let a = 1\n").unwrap();
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "antigravity")
        .env("TRM_SESSION", session_id)
        .env_remove("TRM_MODEL")
        .env_remove("GEMINI_MODEL")
        .args(["read", sample.to_str().unwrap()])
        .output()
        .unwrap();

    let stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_HOME", &fake_home)
        .args(["stats", "--by-model"])
        .output()
        .unwrap();
    assert!(stats.status.success());
    let out = String::from_utf8(stats.stdout).unwrap();
    assert!(
        out.contains("google/gemini-4-neon-medium"),
        "expected resolved model in stats: {out}"
    );

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn prefix_aliases_cat_head_tail_sed_work() {
    let dir = temp_dir("prefix_aliases");
    let file = dir.join("lines.txt");
    fs::write(&file, "L1\nL2\nL3\nL4\nL5\n").unwrap();

    let sed = Command::new(trm_bin())
        .args(["sed", "-n", "2,4p", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(sed.status.success());
    assert_eq!(String::from_utf8(sed.stdout).unwrap(), "L2\nL3\nL4\n");

    let head = Command::new(trm_bin())
        .args(["head", "-n", "2", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap(), "L1\nL2\n");

    let tail = Command::new(trm_bin())
        .args(["tail", "-n", "2", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(tail.status.success());
    assert_eq!(String::from_utf8(tail.stdout).unwrap(), "L4\nL5\n");

    let cat = Command::new(trm_bin())
        .args(["cat", file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(cat.status.success());
    assert_eq!(
        String::from_utf8(cat.stdout).unwrap(),
        "L1\nL2\nL3\nL4\nL5\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn audit_filters_commands_by_per_line_timestamp() {
    let dir = temp_dir("audit_ts");
    let log = dir.join("rollout-ts.jsonl");
    let lines = [
        // Very old command (2020) — must be excluded by --since 1h
        r#"{"timestamp":"2020-01-01T00:00:00Z","type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd:\"cat old.swift\"})"}}"#,
        // Future/current command (2030) — must be included by --since 1h
        r#"{"timestamp":"2030-01-01T00:00:00Z","type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd:\"trm sed -n '1,20p' new.swift\"})"}}"#,
    ];
    fs::write(&log, lines.join("\n")).unwrap();

    let out = Command::new(trm_bin())
        .env("TRM_AUDIT_DIR", format!("codex:{}", dir.display()))
        .args(["audit", "--since", "1h", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let j = String::from_utf8(out.stdout).unwrap();
    assert!(
        j.contains("\"trm_read\":1"),
        "expected trm sed counted: {j}"
    );
    assert!(
        j.contains("\"cat\":0"),
        "expected 2020 cat filtered out: {j}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ls_find_and_ui_once_work() {
    let dir = temp_dir("fs_ui");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join(".git/objects")).unwrap();
    fs::write(dir.join("src/app.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn hi() {}\n").unwrap();
    fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(dir.join(".git/config"), "[core]\n").unwrap();
    fs::write(dir.join(".git/objects/abc123"), "blob\n").unwrap();

    let ls = Command::new(trm_bin())
        .args(["ls", "-la", dir.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(ls.status.success());
    let ls_s = String::from_utf8(ls.stdout).unwrap();
    assert!(ls_s.contains("dirs ("), "got: {ls_s}");

    let fnd = Command::new(trm_bin())
        .args(["find", dir.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(fnd.status.success());
    let fnd_s = String::from_utf8(fnd.stdout).unwrap();
    assert!(fnd_s.contains("app.rs"), "got: {fnd_s}");
    assert!(!fnd_s.contains(".git/HEAD"), "should hide .git: {fnd_s}");

    let ui = Command::new(trm_bin())
        .args(["ui", "--once"])
        .output()
        .unwrap();
    assert!(ui.status.success());
    let ui_s = String::from_utf8(ui.stdout).unwrap();
    assert!(ui_s.contains("1 Overview"), "got: {ui_s}");
    assert!(ui_s.contains("5 Context"), "got: {ui_s}");
    assert!(ui_s.contains("Net Saved:"), "got: {ui_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rg_auto_expands_focused_matches_to_enclosing_block() {
    let dir = temp_dir("rg_block");
    let file = dir.join("Calc.swift");
    fs::write(
        &file,
        "import Foundation\n\nstruct Calc {\n    func computeSpecial() -> Int {\n        let secret = 42\n        return secret * 2\n    }\n    func unrelated() -> Int {\n        return 0\n    }\n}\n",
    )
    .unwrap();

    // 1 match -> auto-expands to the enclosing `computeSpecial` function body!
    let out = Command::new(trm_bin())
        .args(["rg", "secret", dir.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("[L4-L7] (match L5)"), "got: {s}");
    assert!(s.contains("func computeSpecial() -> Int"), "got: {s}");
    assert!(s.contains("return secret * 2"), "got: {s}");
    assert!(
        !s.contains("unrelated"),
        "should not include sibling fn: {s}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stats_tracks_mid_session_model_switch_by_timestamp() {
    let fake_home = temp_dir("model_switch");
    let log = fake_home.join("events.jsonl");
    let sid = "codex-switch-999";
    let sess_dir = fake_home.join(".codex/sessions/2026/11/26");
    fs::create_dir_all(&sess_dir).unwrap();

    // Session starts on gpt-6.1-sol at 12:00:00Z, switches to gpt-6-luna at 12:30:00Z
    // 2026-11-26T12:00:00Z = 1795694400; 12:30:00Z = 1795696200
    fs::write(
        sess_dir.join(format!("rollout-{sid}.jsonl")),
        concat!(
            "{\"timestamp\":\"2026-11-26T12:00:00Z\",\"payload\":{\"model\":\"gpt-6.1-sol\",\"model_provider_id\":\"openai\"}}\n",
            "{\"timestamp\":\"2026-11-26T12:30:00Z\",\"payload\":{\"model\":\"gpt-6-luna\",\"model_provider_id\":\"openai\"}}\n",
        ),
    )
    .unwrap();

    // Two events: one at 12:10:00Z (1795695000), one at 12:40:00Z (1795696800)
    let ev1 = format!(
        "{{\"ts\":1795695000,\"cmd\":\"read\",\"agent\":\"codex\",\"session\":\"{sid}\",\"provider\":\"unknown\",\"model\":\"unknown\",\"path\":\"/a.swift\",\"lang\":\"swift\",\"req_mode\":\"auto\",\"eff_mode\":\"outline\",\"range\":null,\"total_lines\":300,\"raw_bytes\":4000,\"out_bytes\":400,\"raw_tokens\":1000,\"out_tokens\":100}}"
    );
    let ev2 = format!(
        "{{\"ts\":1795696800,\"cmd\":\"read\",\"agent\":\"codex\",\"session\":\"{sid}\",\"provider\":\"unknown\",\"model\":\"unknown\",\"path\":\"/b.swift\",\"lang\":\"swift\",\"req_mode\":\"auto\",\"eff_mode\":\"outline\",\"range\":null,\"total_lines\":300,\"raw_bytes\":4000,\"out_bytes\":400,\"raw_tokens\":1000,\"out_tokens\":100}}"
    );
    fs::write(&log, format!("{ev1}\n{ev2}\n")).unwrap();

    let stats = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_HOME", &fake_home)
        .args(["stats", "--by-model"])
        .output()
        .unwrap();
    assert!(stats.status.success());
    let out = String::from_utf8(stats.stdout).unwrap();
    assert!(
        out.contains("openai/gpt-6.1-sol"),
        "missing first model: {out}"
    );
    assert!(
        out.contains("openai/gpt-6-luna"),
        "missing switched model: {out}"
    );

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn raw_strings_and_js_regex_literals_survive_comment_stripping() {
    let dir = temp_dir("lexer_edges");

    // Swift raw string #"..."# containing // and /*
    let swift_file = dir.join("Raw.swift");
    fs::write(
        &swift_file,
        "// strip me\nlet a = #\"keep // and /* inside raw */\"#\nlet b = 2 // trailing\n",
    )
    .unwrap();
    let sw = Command::new(trm_bin())
        .args([
            "read",
            swift_file.to_str().unwrap(),
            "--minimal",
            "--no-stats",
        ])
        .output()
        .unwrap();
    let sw_s = String::from_utf8(sw.stdout).unwrap();
    assert!(sw_s.contains("keep // and /* inside raw */"), "got: {sw_s}");
    assert!(!sw_s.contains("strip me"));
    assert!(!sw_s.contains("trailing"));

    // Rust raw string r#"..."# containing // and /*
    let rs_file = dir.join("raw.rs");
    fs::write(
        &rs_file,
        "// drop\nconst S: &str = r#\"http://x /* keep */\"#;\n",
    )
    .unwrap();
    let rs = Command::new(trm_bin())
        .args(["read", rs_file.to_str().unwrap(), "--minimal", "--no-stats"])
        .output()
        .unwrap();
    let rs_s = String::from_utf8(rs.stdout).unwrap();
    assert!(rs_s.contains("http://x /* keep */"), "got: {rs_s}");
    assert!(!rs_s.contains("// drop"));

    // JS regex literal containing // and /* inside character class
    let js_file = dir.join("re.js");
    fs::write(
        &js_file,
        "// comment\nconst re = /https?:\\/\\/[a-z/*]+/gi; // inline\n",
    )
    .unwrap();
    let js = Command::new(trm_bin())
        .args(["read", js_file.to_str().unwrap(), "--minimal", "--no-stats"])
        .output()
        .unwrap();
    let js_s = String::from_utf8(js.stdout).unwrap();
    assert!(js_s.contains("/https?:\\/\\/[a-z/*]+/gi;"), "got: {js_s}");
    assert!(!js_s.contains("// inline"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn config_toml_doctor_and_cursor_hook_install_work() {
    let fake_home = temp_dir("cfg_doc");
    fs::create_dir_all(fake_home.join(".cursor")).unwrap();
    fs::create_dir_all(fake_home.join(".claude")).unwrap();

    // Custom config.toml sets outline_lines = 10
    let cfg = fake_home.join("custom.toml");
    fs::write(&cfg, "[read]\nsmall_lines = 5\noutline_lines = 10\n").unwrap();

    let swift_file = fake_home.join("Tiny.swift");
    let mut src = String::from("import Foundation\n\nstruct S {\n");
    for i in 0..4 {
        src.push_str(&format!(
            "    func f{i}() {{\n        let x = {i}\n    }}\n"
        ));
    }
    src.push_str("}\n");
    fs::write(&swift_file, &src).unwrap();

    let out = Command::new(trm_bin())
        .env("TRM_CONFIG", &cfg)
        .args(["read", swift_file.to_str().unwrap(), "--no-stats"])
        .output()
        .unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(
        s.contains("[trm: outline of"),
        "config outline_lines=10 should trigger outline: {s}"
    );

    // Install with Cursor + Claude --hook, verify doctor passes, then uninstall
    let inst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["install", "--hook"])
        .output()
        .unwrap();
    assert!(inst.status.success());
    assert!(fake_home.join(".cursor/rules/trm.mdc").exists());
    let hook = fake_home.join(".claude/hooks/trm-rewrite.sh");
    assert!(hook.exists());

    // Verify hook rewrites `&&` chains safely and skips pipes/redirects/heredocs
    let bin_dir = std::path::Path::new(trm_bin()).parent().unwrap();
    let test_path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run_hook = |cmd_json: &str| -> String {
        use std::io::Write;
        let mut child = Command::new(&hook)
            .env("PATH", &test_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(cmd_json.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let rewritten = run_hook(r#"{"tool_input":{"command":"trm git status && ls -la src/"}}"#);
    assert!(
        rewritten.contains("trm git status && trm ls -la src/"),
        "got: {rewritten}"
    );
    let heredoc = run_hook(r#"{"tool_input":{"command":"cat <<EOF > out.txt"}}"#);
    assert!(
        heredoc.trim().is_empty(),
        "heredoc must not be rewritten: {heredoc}"
    );
    let piped = run_hook(r#"{"tool_input":{"command":"rg foo src/ | wc -l"}}"#);
    assert!(
        piped.trim().is_empty(),
        "piped cmd must not be rewritten: {piped}"
    );

    let doc = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .env("TRM_CONFIG", &cfg)
        .args(["doctor"])
        .output()
        .unwrap();
    assert!(doc.status.success());
    let doc_s = String::from_utf8(doc.stdout).unwrap();
    assert!(doc_s.contains("trm doctor"), "got: {doc_s}");
    assert!(doc_s.contains("cursor"), "got: {doc_s}");
    assert!(doc_s.contains("claude hook"), "got: {doc_s}");
    assert!(doc_s.contains("script installed"), "got: {doc_s}");

    let uninst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["uninstall"])
        .output()
        .unwrap();
    assert!(uninst.status.success());
    assert!(!fake_home.join(".cursor/rules/trm.mdc").exists());
    assert!(!fake_home.join(".claude/hooks/trm-rewrite.sh").exists());

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn multi_symbol_and_fuzzy_symbol_fallback_work() {
    let dir = temp_dir("sym_fuzzy");
    let f = dir.join("Recorder.swift");
    fs::write(
        &f,
        r#"struct Recorder {
    func startSession() {
        print("start")
    }

    func finishCurrentRecording() {
        print("finish")
    }

    func cancelRecording() {
        print("cancel")
    }
}
"#,
    )
    .unwrap();

    // Multi-symbol `-s startSession,cancelRecording` returns both raw bodies in 1 call
    let multi = Command::new(trm_bin())
        .args([
            "read",
            f.to_str().unwrap(),
            "-s",
            "startSession,cancelRecording",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(multi.status.success());
    let ms = String::from_utf8(multi.stdout).unwrap();
    assert!(ms.contains("func startSession()"), "got: {ms}");
    assert!(ms.contains("func cancelRecording()"), "got: {ms}");
    assert!(
        !ms.contains("finishCurrentRecording"),
        "should not include unmatched symbol: {ms}"
    );

    // Fuzzy fallback when `-s finishRecording` misses exact name
    let miss = Command::new(trm_bin())
        .args([
            "read",
            f.to_str().unwrap(),
            "-s",
            "finishRecording",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(!miss.status.success());
    let err = String::from_utf8(miss.stderr).unwrap();
    assert!(
        err.contains("Closest symbols:"),
        "expected closest suggestions: {err}"
    );
    assert!(
        err.contains("finishCurrentRecording"),
        "expected finishCurrentRecording: {err}"
    );
    assert!(err.contains("[L"), "expected line range hint: {err}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn plain_log_deduplicates_consecutive_identical_lines() {
    let dir = temp_dir("log_dedup");
    let log_file = dir.join("server.log");
    let mut content = String::from("INFO server started\n");
    for _ in 0..15 {
        content.push_str("WARN retrying connection to 127.0.0.1:5432\n");
    }
    content.push_str("INFO connected\n");
    fs::write(&log_file, &content).unwrap();

    let out = Command::new(trm_bin())
        .args([
            "read",
            log_file.to_str().unwrap(),
            "--minimal",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("INFO server started"), "got: {s}");
    assert!(
        s.contains("WARN retrying connection to 127.0.0.1:5432  [×15]"),
        "got: {s}"
    );
    assert!(s.contains("INFO connected"), "got: {s}");
    assert_eq!(
        s.lines().count(),
        3,
        "expected 3 lines after dedup, got: {s}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn raw_slices_preserve_crlf_line_endings() {
    let dir = temp_dir("crlf_raw");
    let file = dir.join("win.rs");
    let src = "fn alpha() {\r\n    let x = 1;\r\n}\r\n\r\nfn beta() {\r\n    let y = 2;\r\n}\r\n";
    fs::write(&file, src).unwrap();

    // -r slice must preserve exact \r\n bytes
    let range = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-r", "2-3", "--no-stats"])
        .output()
        .unwrap();
    assert!(range.status.success());
    assert_eq!(range.stdout, b"    let x = 1;\r\n}\r\n");

    // --head and --tail must preserve \r\n
    let head = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--head", "2", "--no-stats"])
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(head.stdout, b"fn alpha() {\r\n    let x = 1;\r\n");

    let tail = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--tail", "2", "--no-stats"])
        .output()
        .unwrap();
    assert!(tail.status.success());
    assert_eq!(tail.stdout, b"    let y = 2;\r\n}\r\n");

    // --raw must be byte-for-byte identical to the file
    let raw = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--raw", "--no-stats"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert_eq!(raw.stdout, src.as_bytes());

    // -s symbol jump must preserve \r\n on the returned raw code lines
    let sym = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "beta", "--no-stats"])
        .output()
        .unwrap();
    assert!(sym.status.success());
    assert_eq!(
        sym.stdout,
        b"[L5-L7]\nfn beta() {\r\n    let y = 2;\r\n}\r\n"
    );

    // --outline still works cleanly on CRLF files
    let outline = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--outline", "--no-stats"])
        .output()
        .unwrap();
    assert!(outline.status.success());
    let out_s = String::from_utf8(outline.stdout).unwrap();
    assert!(out_s.contains("fn alpha()"), "got: {out_s}");
    assert!(out_s.contains("fn beta()"), "got: {out_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn small_find_still_hides_git_and_noisy_paths() {
    let dir = temp_dir("small_find_git");
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();

    // Default `trm find` must hide .git even when output is tiny
    let out = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["find", ".", "--no-stats"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("a.rs"), "expected a.rs in output: {s}");
    assert!(!s.contains(".git"), "small find leaked .git: {s}");

    // `--all` should include .git
    let all = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["find", "--all", ".", "--no-stats"])
        .output()
        .unwrap();
    assert!(all.status.success());
    let all_s = String::from_utf8(all.stdout).unwrap();
    assert!(all_s.contains(".git"), "--all should show .git: {all_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn python_outline_and_symbol_include_decorators() {
    let dir = temp_dir("py_decorators");
    let file = dir.join("app.py");
    let src = "\
import functools

@functools.lru_cache
@app.route(
    \"/top\",
    methods=[\"GET\"],
)
def top():
    return 1

class Service:
    @staticmethod
    def ping():
        return \"pong\"
";
    fs::write(&file, src).unwrap();

    // Outline range for `top` must start at L3 (@functools.lru_cache) and `ping` at L12 (@staticmethod)
    let outline = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "--outline", "--no-stats"])
        .output()
        .unwrap();
    assert!(outline.status.success());
    let out_s = String::from_utf8(outline.stdout).unwrap();
    assert!(out_s.contains("L3-9| def top():"), "got: {out_s}");
    assert!(out_s.contains("L12-14|     def ping():"), "got: {out_s}");

    // `-s top` must include both decorators and the function body
    let sym = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "top", "--no-stats"])
        .output()
        .unwrap();
    assert!(sym.status.success());
    let sym_s = String::from_utf8(sym.stdout).unwrap();
    assert!(sym_s.contains("[L3-L9]"), "got: {sym_s}");
    assert!(
        sym_s.contains("@functools.lru_cache"),
        "missing decorator: {sym_s}"
    );
    assert!(
        sym_s.contains("@app.route("),
        "missing multi-line decorator: {sym_s}"
    );
    assert!(sym_s.contains("def top():"), "got: {sym_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_diff_trailing_all_and_raw_return_full_diff() {
    let dir = temp_dir("git_diff_flags");
    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run_git(&["init", "-q"]);
    run_git(&["config", "user.email", "test@example.com"]);
    run_git(&["config", "user.name", "Test"]);

    let file = dir.join("big.txt");
    fs::write(&file, "line0\n").unwrap();
    run_git(&["add", "big.txt"]);
    run_git(&["commit", "-q", "-m", "init"]);

    // Modify with >120 added lines so default `trm git diff` caps it
    let mut updated = String::from("line0\n");
    for i in 1..=150 {
        updated.push_str(&format!("added_line_{i}\n"));
    }
    fs::write(&file, &updated).unwrap();

    let capped = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["git", "diff", "--no-stats"])
        .output()
        .unwrap();
    assert!(capped.status.success());
    let capped_s = String::from_utf8(capped.stdout).unwrap();
    assert!(
        capped_s.contains("pass --all or --raw"),
        "expected cap hint: {capped_s}"
    );
    assert!(!capped_s.contains("added_line_150"));

    // Following the hint: `trm git diff --all` must NOT return empty output
    let all = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["git", "diff", "--all", "--no-stats"])
        .output()
        .unwrap();
    assert!(all.status.success());
    let all_s = String::from_utf8(all.stdout).unwrap();
    assert!(
        all_s.contains("+added_line_150"),
        "git diff --all missing changes: {all_s}"
    );

    // Following the hint: `trm git diff --raw` must return full unified diff, not git --raw format
    let raw = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["git", "diff", "--raw", "--no-stats"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    let raw_s = String::from_utf8(raw.stdout).unwrap();
    assert!(
        raw_s.contains("diff --git"),
        "git diff --raw should be raw unified diff: {raw_s}"
    );
    assert!(
        raw_s.contains("+added_line_150"),
        "git diff --raw missing changes: {raw_s}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn typescript_outline_and_symbol_include_decorators() {
    let dir = temp_dir("ts_decorators");
    let file = dir.join("foo.component.ts");
    let src = "\
@Component({
  selector: 'app-foo',
})
export class Foo {
  @Get('/items')
  list() {
    return [];
  }
}
";
    fs::write(&file, src).unwrap();

    let sym = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "Foo", "--no-stats"])
        .output()
        .unwrap();
    assert!(sym.status.success());
    let sym_s = String::from_utf8(sym.stdout).unwrap();
    assert!(sym_s.contains("[L1-L9]"), "got: {sym_s}");
    assert!(
        sym_s.contains("@Component({"),
        "missing @Component decorator: {sym_s}"
    );
    assert!(sym_s.contains("export class Foo {"), "got: {sym_s}");

    let method = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "list", "--no-stats"])
        .output()
        .unwrap();
    assert!(method.status.success());
    let m_s = String::from_utf8(method.stdout).unwrap();
    assert!(m_s.contains("[L5-L8]"), "got: {m_s}");
    assert!(
        m_s.contains("@Get('/items')"),
        "missing method decorator: {m_s}"
    );

    // Previous decorated field (`@Input() a = 1;` / `@Published var x = 1`) must NOT be pulled into next symbol
    let ts_member = dir.join("d.ts");
    fs::write(
        &ts_member,
        "\
export class Panel {
  @Input() a = 1;
  @HostListener(\"click\", [\"$event\"])
  onClick(e: Event) {
    return e;
  }
}
",
    )
    .unwrap();
    let on_click = Command::new(trm_bin())
        .args([
            "read",
            ts_member.to_str().unwrap(),
            "-s",
            "onClick",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(on_click.status.success());
    let oc_s = String::from_utf8(on_click.stdout).unwrap();
    assert!(oc_s.contains("[L3-L6]"), "got: {oc_s}");
    assert!(
        !oc_s.contains("@Input() a = 1"),
        "pulled previous field: {oc_s}"
    );
    assert!(
        oc_s.contains("@HostListener"),
        "missing own decorator: {oc_s}"
    );

    let swift_member = dir.join("d.swift");
    fs::write(
        &swift_member,
        "\
final class ViewModel {
    @Published var x = 1
    @objc func tap() {
        x += 1
    }
}
",
    )
    .unwrap();
    let tap = Command::new(trm_bin())
        .args([
            "read",
            swift_member.to_str().unwrap(),
            "-s",
            "tap",
            "--no-stats",
        ])
        .output()
        .unwrap();
    assert!(tap.status.success());
    let tap_s = String::from_utf8(tap.stdout).unwrap();
    assert!(tap_s.contains("[L3-L5]"), "got: {tap_s}");
    assert!(
        !tap_s.contains("@Published var x"),
        "pulled previous property: {tap_s}"
    );
    assert!(tap_s.contains("@objc func tap()"), "got: {tap_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn trm_rg_searches_piped_stdin_instead_of_directory() {
    use std::io::Write;
    use std::process::Stdio;

    let dir = temp_dir("rg_stdin");
    fs::write(dir.join("disk.txt"), "beta_from_disk\n").unwrap();

    let mut child = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["rg", "beta", "--no-stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"alpha\nbeta_from_pipe\ngamma\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(
        s.contains("beta_from_pipe"),
        "expected piped match, got: {s}"
    );
    assert!(
        !s.contains("beta_from_disk"),
        "should not search folder when stdin is piped: {s}"
    );
    assert!(
        !s.contains("<stdin>"),
        "piped rg should omit <stdin> prefix: {s}"
    );

    // Multi-match piped input: must omit `<stdin> (N)` header and still cap matches
    let mut multi = Command::new(trm_bin())
        .current_dir(&dir)
        .args(["rg", "hit", "--no-stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let payload: String = (1..=10).map(|i| format!("hit_{i}\n")).collect();
    multi
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let multi_out = multi.wait_with_output().unwrap();
    assert!(multi_out.status.success());
    let ms = String::from_utf8(multi_out.stdout).unwrap();
    assert!(
        !ms.contains("<stdin>"),
        "multi-match piped rg leaked <stdin> header: {ms}"
    );
    assert!(ms.contains("1: hit_1"), "got: {ms}");
    assert!(
        ms.contains("+4 more matches"),
        "expected per-file cap on piped input: {ms}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn piped_stdin_works_for_head_tail_and_go_symbol_includes_doc_comments() {
    use std::io::Write;
    use std::process::Stdio;

    // Piped `trm head -n 2` and `trm tail -n 1` with no file argument
    let mut head = Command::new(trm_bin())
        .args(["head", "-n", "2", "--no-stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    head.stdin
        .as_mut()
        .unwrap()
        .write_all(b"one\ntwo\nthree\n")
        .unwrap();
    let head_out = head.wait_with_output().unwrap();
    assert!(head_out.status.success());
    assert_eq!(String::from_utf8(head_out.stdout).unwrap(), "one\ntwo\n");

    let mut tail = Command::new(trm_bin())
        .args(["tail", "-n", "1", "--no-stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    tail.stdin
        .as_mut()
        .unwrap()
        .write_all(b"one\ntwo\nthree\n")
        .unwrap();
    let tail_out = tail.wait_with_output().unwrap();
    assert!(tail_out.status.success());
    assert_eq!(String::from_utf8(tail_out.stdout).unwrap(), "three\n");

    // Go `-s Sum` includes directly adjacent `//` doc comments, and `-s T` returns type + methods
    let dir = temp_dir("go_symbol_docs");
    let file = dir.join("math.go");
    fs::write(
        &file,
        "\
package math

// Detached package note

// Sum adds two integers.
// Second doc line.
func Sum(a, b int) int {
\treturn a + b
}

// T holds a value.
type T struct {
\tV int
}

// M returns V.
func (t *T) M() int {
\treturn t.V
}
",
    )
    .unwrap();

    let sum = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "Sum", "--no-stats"])
        .output()
        .unwrap();
    assert!(sum.status.success());
    let sum_s = String::from_utf8(sum.stdout).unwrap();
    assert!(sum_s.contains("[L5-L9]"), "got: {sum_s}");
    assert!(
        sum_s.contains("// Sum adds two integers."),
        "missing Go doc: {sum_s}"
    );
    assert!(
        sum_s.contains("// Second doc line."),
        "missing Go doc line 2: {sum_s}"
    );
    assert!(
        !sum_s.contains("Detached package note"),
        "pulled detached comment: {sum_s}"
    );

    let t_sym = Command::new(trm_bin())
        .args(["read", file.to_str().unwrap(), "-s", "T", "--no-stats"])
        .output()
        .unwrap();
    assert!(t_sym.status.success());
    let t_s = String::from_utf8(t_sym.stdout).unwrap();
    assert!(t_s.contains("type T struct"), "got: {t_s}");
    assert!(t_s.contains("func (t *T) M() int"), "got: {t_s}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn export_bundles_untruncated_misses_bypasses_and_redacts_secrets() {
    let dir = temp_dir("export_report");
    let log = dir.join("events.jsonl");
    let audit_dir = dir.join("audit");
    fs::create_dir_all(&audit_dir).unwrap();

    // Generate a real trim miss (minimal -> raw)
    let src_file = dir.join("big.swift");
    let mut src = String::from("import Foundation\n\n");
    for i in 0..30 {
        src.push_str(&format!(
            "// comment {i}\nfunc f{i}() {{\n    let x = {i}\n}}\n"
        ));
    }
    fs::write(&src_file, &src).unwrap();

    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .args(["read", src_file.to_str().unwrap(), "--minimal"])
        .output()
        .unwrap();
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AGENT", "codex")
        .args(["read", src_file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();

    // Synthetic audit log with a long bypass containing $HOME and an API key
    let fake_home = dir.join("fakehome");
    fs::create_dir_all(&fake_home).unwrap();
    let audit_log = audit_dir.join("rollout.jsonl");
    let cmd_str = format!(
        "OPENAI_API_KEY=sk-1234567890abcdef cat {}/very/long/path/that/should/not/be/truncated/file.swift",
        fake_home.display()
    );
    fs::write(
        &audit_log,
        format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"custom_tool_call\",\"name\":\"exec\",\"input\":\"await tools.exec_command({{cmd:\\\"{cmd_str}\\\"}})\"}}}}\n"
        ),
    )
    .unwrap();

    let report_path = dir.join("report.json");
    let out = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .env("TRM_LOG_PATH", &log)
        .env("TRM_AUDIT_DIR", format!("codex:{}", audit_dir.display()))
        .args([
            "export",
            "--since",
            "all",
            "-o",
            report_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = fs::read_to_string(&report_path).unwrap();
    assert!(json.contains("\"misses\": ["), "got: {json}");
    assert!(json.contains("\"bypasses\": ["), "got: {json}");
    assert!(json.contains("\"large_passthroughs\": ["), "got: {json}");
    assert!(json.contains("\"by_mode\": {"), "got: {json}");
    assert!(
        json.contains("\"first_mode\": \"minimal\""),
        "missing miss details: {json}"
    );
    assert!(
        json.contains("not/be/truncated/file.swift"),
        "bypass command was truncated: {json}"
    );
    assert!(
        json.contains("OPENAI_API_KEY=[REDACTED]"),
        "secret not redacted: {json}"
    );
    assert!(
        !json.contains("sk-1234567890abcdef"),
        "leaked secret: {json}"
    );
    assert!(
        json.contains("~/very/long/path"),
        "home not redacted to ~: {json}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stats_splits_raw_into_slice_pass_raw_and_export_lists_large_passthroughs() {
    let dir = temp_dir("raw_mode_split");
    let log = dir.join("events.jsonl");

    // 1. Small file -> auto passthrough ("pass")
    let small_file = dir.join("small.txt");
    fs::write(&small_file, "line 1\nline 2\n").unwrap();
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", small_file.to_str().unwrap()])
        .output()
        .unwrap();

    // 2. Ranged read -> "slice"
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", small_file.to_str().unwrap(), "-r", "1-2"])
        .output()
        .unwrap();

    // 3. Explicit --raw -> "raw"
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", small_file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();

    // 4. Medium markdown (~100 lines, >300 tok, under outline_lines) -> large auto passthrough
    let md_file = dir.join("notes.md");
    let mut md = String::new();
    for i in 0..100 {
        md.push_str(&format!(
            "Paragraph {i}: some prose line that has enough characters to pass 300 tokens.\n"
        ));
    }
    fs::write(&md_file, &md).unwrap();
    let _ = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", md_file.to_str().unwrap()])
        .output()
        .unwrap();

    let stats_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["stats", "--since", "all"])
        .output()
        .unwrap();
    assert!(stats_out.status.success());
    let s = String::from_utf8_lossy(&stats_out.stdout);
    assert!(s.contains("slice"), "expected slice in By mode: {s}");
    assert!(s.contains("pass"), "expected pass in By mode: {s}");
    assert!(s.contains("raw"), "expected explicit raw in By mode: {s}");

    let exp_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["export", "--since", "all"])
        .output()
        .unwrap();
    assert!(exp_out.status.success());
    let e = String::from_utf8_lossy(&exp_out.stdout);
    assert!(e.contains("\"large_passthroughs_count\": 1"), "got: {e}");
    assert!(
        e.contains("notes.md"),
        "expected notes.md in large_passthroughs: {e}"
    );
    assert!(
        !e.contains("\"small.txt\""),
        "small.txt should not be a large passthrough: {e}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn minified_json_triggers_byte_guard_in_auto_and_passes_raw() {
    let dir = temp_dir("minified_guard");
    let log = dir.join("events.jsonl");
    let min_file = dir.join("bundle.min.json");
    let payload = format!("{{\"items\":[{}]}}", "\"abcdef1234567890\",".repeat(1200));
    assert!(payload.len() > 12_288);
    fs::write(&min_file, &payload).unwrap();

    let auto_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", min_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(auto_out.status.success());
    let auto_str = String::from_utf8(auto_out.stdout).unwrap();
    assert!(
        auto_str.contains("[trm: minified/wide file ("),
        "expected minified footer: {auto_str}"
    );
    assert!(auto_str.contains("lines truncated to 300 chars"));
    assert!(
        auto_str.len() < 1000,
        "expected compact output, got {} bytes",
        auto_str.len()
    );

    let raw_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", min_file.to_str().unwrap(), "--raw"])
        .output()
        .unwrap();
    assert!(raw_out.status.success());
    assert_eq!(
        String::from_utf8(raw_out.stdout).unwrap(),
        format!("{payload}\n")
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_show_trims_commit_diff_and_passes_blob_raw() {
    let dir = temp_dir("git_show");
    let log = dir.join("events.jsonl");

    let run_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run_git(&["init"]);
    run_git(&["config", "user.name", "Tester"]);
    run_git(&["config", "user.email", "tester@example.com"]);
    fs::write(
        dir.join("main.rs"),
        "fn main() {\n    println!(\"v1\");\n}\n",
    )
    .unwrap();
    fs::write(dir.join("Cargo.lock"), "line1\nline2\nline3\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "init"]);

    fs::write(
        dir.join("main.rs"),
        "fn main() {\n    println!(\"v2\");\n}\n",
    )
    .unwrap();
    fs::write(dir.join("Cargo.lock"), "line1\nchanged\nline3\nchanged2\n").unwrap();
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "update"]);

    let show_out = Command::new(trm_bin())
        .current_dir(&dir)
        .env("TRM_LOG_PATH", &log)
        .args(["git", "show", "HEAD"])
        .output()
        .unwrap();
    assert!(show_out.status.success());
    let show_str = String::from_utf8_lossy(&show_out.stdout);
    assert!(show_str.contains("=== main.rs ==="), "got: {show_str}");
    assert!(
        show_str.contains("+    println!(\"v2\");"),
        "got: {show_str}"
    );
    assert!(show_str.contains("Cargo.lock (skipped"), "got: {show_str}");
    assert!(
        !show_str.contains("+changed2"),
        "lockfile diff must be skipped: {show_str}"
    );

    let blob_out = Command::new(trm_bin())
        .current_dir(&dir)
        .env("TRM_LOG_PATH", &log)
        .args(["git", "show", "HEAD:main.rs"])
        .output()
        .unwrap();
    assert!(blob_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&blob_out.stdout),
        "fn main() {\n    println!(\"v2\");\n}\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn export_then_inspect_round_trips_report_file() {
    let dir = temp_dir("export_inspect");
    let log = dir.join("events.jsonl");
    let report = dir.join("report.json");
    let sample = dir.join("demo.rs");

    let mut code = String::new();
    for i in 0..110 {
        code.push_str(&format!(
            "// comment line {i}\npub fn f_{i}() -> usize {{ {i} }}\n"
        ));
    }
    fs::write(&sample, &code).unwrap();

    let read_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", sample.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(read_out.status.success());

    let exp_out = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["export", "--since", "all", "-o", report.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(exp_out.status.success());
    assert!(report.exists());

    let inspect_out = Command::new(trm_bin())
        .args(["inspect", report.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(inspect_out.status.success());
    let inspect_str = String::from_utf8_lossy(&inspect_out.stdout);
    assert!(
        inspect_str.contains("trm inspect — report v"),
        "got: {inspect_str}"
    );
    assert!(inspect_str.contains("Events: 1"), "got: {inspect_str}");
    assert!(inspect_str.contains("By lang"), "got: {inspect_str}");
    assert!(inspect_str.contains("rust"), "got: {inspect_str}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pipe_summarizes_json_and_collapses_logs() {
    let dir = temp_dir("pipe_mode");
    let log = dir.join("events.jsonl");

    // 1. Bare `trm` with piped JSON stdin
    let items: Vec<String> = (0..60)
        .map(|i| format!(r#"{{"id":{i},"token":"value_payload_{i}"}}"#))
        .collect();
    let json_input = format!("[{}]", items.join(","));

    let mut child = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(json_input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("array[60 items]"), "got: {stdout}");
    assert!(stdout.contains("[trm: json summary"), "got: {stdout}");

    // 2. `trm pipe` on repeated log lines
    let mut logs = String::new();
    for i in 0..80 {
        logs.push_str(&format!("2026-11-27T12:00:{:02} INFO tick\n", i % 60));
    }
    logs.push_str("2026-11-27T12:02:00 ERROR fatal failure\n");
    let mut child2 = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["pipe"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child2
            .stdin
            .as_mut()
            .unwrap()
            .write_all(logs.as_bytes())
            .unwrap();
    }
    let out2 = child2.wait_with_output().unwrap();
    assert!(out2.status.success());
    let stdout2 = String::from_utf8(out2.stdout).unwrap();
    assert!(stdout2.contains("(x80)"), "got: {stdout2}");
    assert!(stdout2.contains("ERROR fatal failure"), "got: {stdout2}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hooks_rewrite_agy_and_claude_and_install_merges_hooks_json() {
    use std::io::Write;

    let fake_home = temp_dir("hooks_install");
    let gemini_cfg = fake_home.join(".gemini").join("config");
    fs::create_dir_all(&gemini_cfg).unwrap();
    let hooks_file = gemini_cfg.join("hooks.json");
    fs::write(&hooks_file, "{\n  \"other-hook\": {\"Stop\": []}\n}\n").unwrap();

    let inst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["install", "--hook", "--only", "antigravity"])
        .output()
        .unwrap();
    assert!(inst.status.success());
    let merged = fs::read_to_string(&hooks_file).unwrap();
    assert!(
        merged.contains("\"other-hook\""),
        "preserved other hook: {merged}"
    );
    assert!(
        merged.contains("\"trm-rewrite\""),
        "added trm-rewrite: {merged}"
    );
    assert!(
        merged.contains("trm hook agy"),
        "wired trm hook agy: {merged}"
    );

    // Uninstall removes only trm-rewrite
    let uninst = Command::new(trm_bin())
        .env("TRM_HOME", &fake_home)
        .args(["uninstall", "--only", "antigravity"])
        .output()
        .unwrap();
    assert!(uninst.status.success());
    let after = fs::read_to_string(&hooks_file).unwrap();
    assert!(
        after.contains("\"other-hook\""),
        "still preserves other-hook: {after}"
    );
    assert!(
        !after.contains("\"trm-rewrite\""),
        "removed trm-rewrite: {after}"
    );

    // Direct `trm hook agy` execution
    let mut agy = Command::new(trm_bin())
        .args(["hook", "agy"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    agy.stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"toolCall":{"name":"run_command","args":{"CommandLine":"cat README.md"}}}"#)
        .unwrap();
    let agy_out = String::from_utf8(agy.wait_with_output().unwrap().stdout).unwrap();
    assert!(agy_out.contains("trm cat README.md"), "got: {agy_out}");

    // Direct `trm hook claude` execution
    let mut claude = Command::new(trm_bin())
        .args(["hook", "claude"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    claude
        .stdin
        .as_mut()
        .unwrap()
        .write_all(br#"{"tool_input":{"command":"rg foo src/"}}"#)
        .unwrap();
    let claude_out = String::from_utf8(claude.wait_with_output().unwrap().stdout).unwrap();
    assert!(claude_out.contains("trm rg foo src/"), "got: {claude_out}");

    let _ = fs::remove_dir_all(&fake_home);
}

#[test]
fn audit_context_offline_and_export_anon() {
    let dir = temp_dir("context_anon");
    let log = dir.join("events.jsonl");
    let claude_dir = dir.join(".claude");
    let skill_dir = dir.join(".agents").join("skills").join("sample");
    let gemini_cfg = dir.join(".gemini").join("config");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::create_dir_all(&gemini_cfg).unwrap();

    fs::write(
        claude_dir.join("CLAUDE.md"),
        "# Always on instructions\nLine 2\n",
    )
    .unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: sample\ndescription: Test skill metadata.\n---\nBody\n",
    )
    .unwrap();
    fs::write(
        gemini_cfg.join("mcp_config.json"),
        r#"{"mcpServers":{"demo":{"command":"echo","args":["hi"]}}}"#,
    )
    .unwrap();

    let ctx_out = Command::new(trm_bin())
        .env("TRM_HOME", &dir)
        .args(["audit", "--context"])
        .output()
        .unwrap();
    assert!(ctx_out.status.success());
    let ctx_str = String::from_utf8(ctx_out.stdout).unwrap();
    assert!(ctx_str.contains("trm audit --context"), "got: {ctx_str}");
    assert!(ctx_str.contains("CLAUDE.md"), "got: {ctx_str}");
    assert!(ctx_str.contains("sample"), "got: {ctx_str}");
    assert!(ctx_str.contains("demo"), "got: {ctx_str}");

    // Populate a large passthrough event so export --anon hashes the file path
    let secret_file = dir.join("secret-repo").join("payroll.md");
    fs::create_dir_all(secret_file.parent().unwrap()).unwrap();
    let big_md = "Prose line with enough bytes to exceed 300 tokens in telemetry.\n".repeat(40);
    fs::write(&secret_file, &big_md).unwrap();

    let r = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["read", secret_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(r.status.success());

    let exp = Command::new(trm_bin())
        .env("TRM_LOG_PATH", &log)
        .args(["export", "--since", "all", "--anon"])
        .output()
        .unwrap();
    assert!(exp.status.success());
    let exp_str = String::from_utf8(exp.stdout).unwrap();
    assert!(exp_str.contains("\"anonymized\": true"), "got: {exp_str}");
    assert!(!exp_str.contains("payroll"), "leaked filename: {exp_str}");
    assert!(exp_str.contains(".md\""), "lost extension: {exp_str}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn install_dry_run_leaves_user_config_untouched() {
    let dir = temp_dir("install_dry_run");
    let gemini_cfg = dir.join(".gemini").join("config");
    let claude_dir = dir.join(".claude");
    fs::create_dir_all(&gemini_cfg).unwrap();
    fs::create_dir_all(&claude_dir).unwrap();

    let gemini_md = gemini_cfg.join("GEMINI.md");
    let hooks_json = gemini_cfg.join("hooks.json");
    let orig_md = "# Existing user rules\n";
    let orig_hooks = r#"{"hooks":{"PreToolUse":[{"name":"existing"}]}}"#;
    fs::write(&gemini_md, orig_md).unwrap();
    fs::write(&hooks_json, orig_hooks).unwrap();

    let out = Command::new(trm_bin())
        .env("TRM_HOME", &dir)
        .args(["install", "--hook", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("dry-run: no files were modified"),
        "got: {stdout}"
    );
    assert_eq!(fs::read_to_string(&gemini_md).unwrap(), orig_md);
    assert_eq!(fs::read_to_string(&hooks_json).unwrap(), orig_hooks);
    assert!(!claude_dir.join("hooks").join("trm-rewrite.sh").exists());

    let _ = fs::remove_dir_all(&dir);
}
