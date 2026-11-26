use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::style::Theme;

pub fn installed_agents() -> Vec<(&'static str, &'static str)> {
    let home = PathBuf::from(
        env::var("TRM_HOME")
            .or_else(|_| env::var("HOME"))
            .unwrap_or_else(|_| ".".into()),
    );
    let rule_files: &[(&str, PathBuf)] = &[
        ("codex", home.join(".codex/AGENTS.md")),
        ("antigravity", home.join(".gemini/config/GEMINI.md")),
        ("claude", home.join(".claude/CLAUDE.md")),
        ("copilot", home.join(".copilot/copilot-instructions.md")),
        ("pi", home.join(".pi/agent/AGENTS.md")),
        ("cursor", home.join(".cursor/rules/trm.mdc")),
    ];
    rule_files
        .iter()
        .map(|(agent, path)| {
            let status = if !path.exists() {
                "not_installed"
            } else if let Ok(content) = fs::read_to_string(path) {
                let trm_cnt = content.matches("<!-- trm:start -->").count();
                let legacy_cnt = content.matches("<!-- strm:start -->").count();
                if trm_cnt == 1 && legacy_cnt == 0 {
                    "installed"
                } else if trm_cnt > 1 || legacy_cnt > 0 {
                    "needs_fix"
                } else {
                    "no_block"
                }
            } else {
                "unreadable"
            };
            (*agent, status)
        })
        .collect()
}

pub fn run() -> Result<(), String> {
    let t = Theme::detect();
    let home = PathBuf::from(
        env::var("TRM_HOME")
            .or_else(|_| env::var("HOME"))
            .unwrap_or_else(|_| ".".into()),
    );

    println!("{}", t.header_rule("trm doctor", "environment & rules health check"));

    // 1. Dependencies in PATH
    println!("\n{}", t.bold("Binaries:"));
    for bin in ["trm", "rg", "git", "swift", "cargo"] {
        let ok = Command::new("which")
            .arg(bin)
            .output()
            .is_ok_and(|o| o.status.success());
        let badge = if ok { t.green("ok ") } else { t.amber("n/a") };
        println!("  [{badge}] {bin}");
    }

    // 2. Agent rule files
    println!("\n{}", t.bold("Agent Rules:"));
    let rule_files: &[(&str, PathBuf)] = &[
        ("codex", home.join(".codex/AGENTS.md")),
        ("antigravity", home.join(".gemini/config/GEMINI.md")),
        ("claude", home.join(".claude/CLAUDE.md")),
        ("copilot", home.join(".copilot/copilot-instructions.md")),
        ("pi", home.join(".pi/agent/AGENTS.md")),
        ("cursor", home.join(".cursor/rules/trm.mdc")),
    ];
    for (agent, path) in rule_files {
        if !path.exists() {
            println!("  [{}] {:<12} not installed", t.dim("---"), agent);
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            println!("  [{}] {:<12} unreadable", t.red("err"), agent);
            continue;
        };
        let trm_cnt = content.matches("<!-- trm:start -->").count();
        let legacy_cnt = content.matches("<!-- strm:start -->").count();
        if trm_cnt == 1 && legacy_cnt == 0 {
            println!("  [{}] {:<12} installed cleanly", t.green("ok "), agent);
        } else if trm_cnt > 1 || legacy_cnt > 0 {
            println!(
                "  [{}] {:<12} duplicate/legacy markers (trm={trm_cnt}, strm={legacy_cnt}) — run 'trm install'",
                t.amber("fix"),
                agent
            );
        } else {
            println!("  [{}] {:<12} file exists, no trm block — run 'trm install'", t.amber("off"), agent);
        }
    }

    let hook_path = home.join(".claude/hooks/trm-rewrite.sh");
    if hook_path.exists() {
        let executable = is_executable(&hook_path);
        let settings_wired = fs::read_to_string(home.join(".claude/settings.json"))
            .is_ok_and(|s| s.contains("trm-rewrite.sh"));
        if executable && settings_wired {
            println!("  [{}] {:<12} installed & active in settings.json", t.green("ok "), "claude hook");
        } else if !executable {
            println!("  [{}] {:<12} not executable — run 'trm install --hook'", t.amber("fix"), "claude hook");
        } else {
            println!("  [{}] {:<12} script installed (not wired in ~/.claude/settings.json)", t.amber("off"), "claude hook");
        }
    } else {
        println!("  [{}] {:<12} not installed (optional: 'trm install --hook')", t.dim("---"), "claude hook");
    }

    // 3. Config & telemetry
    println!("\n{}", t.bold("Config & Telemetry:"));
    let cfg_path = Config::path();
    let cfg = Config::load();
    if cfg_path.exists() {
        println!(
            "  [{}] config: {} (outline>{}L, rg max={}/{})",
            t.green("ok "),
            cfg_path.display(),
            cfg.outline_lines,
            cfg.rg_max_total,
            cfg.rg_per_file
        );
    } else {
        println!(
            "  [{}] config: defaults (outline>{}L, rg max={}/{}) — optional {}",
            t.dim("def"),
            cfg.outline_lines,
            cfg.rg_max_total,
            cfg.rg_per_file,
            cfg_path.display()
        );
    }

    let log_path = home.join(".local/share/trm/events.jsonl");
    if log_path.exists() {
        let lines = fs::read_to_string(&log_path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        println!("  [{}] telemetry: {} ({lines} events)", t.green("ok "), log_path.display());
    } else {
        println!("  [{}] telemetry: not created yet ({})", t.dim("---"), log_path.display());
    }

    Ok(())
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}
