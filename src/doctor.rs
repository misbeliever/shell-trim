use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::style::Theme;

fn agent_rule_files(home: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("codex", home.join(".codex/AGENTS.md")),
        ("agy", home.join(".gemini/config/GEMINI.md")),
        ("claude", home.join(".claude/CLAUDE.md")),
        ("copilot", home.join(".copilot/copilot-instructions.md")),
        ("pi", home.join(".pi/agent/AGENTS.md")),
        ("cursor", home.join(".cursor/rules/trm.mdc")),
    ]
}

fn rule_status(path: &std::path::Path) -> (&'static str, usize) {
    if !path.exists() {
        return ("not_installed", 0);
    }
    let Ok(content) = fs::read_to_string(path) else {
        return ("unreadable", 0);
    };
    let trm_cnt = content.matches("<!-- trm:start -->").count();
    let status = if trm_cnt == 1 {
        "installed"
    } else if trm_cnt > 1 {
        "needs_fix"
    } else {
        "no_block"
    };
    (status, trm_cnt)
}

pub fn installed_agents() -> Vec<(&'static str, &'static str)> {
    let home = Config::home_dir();
    agent_rule_files(&home)
        .into_iter()
        .map(|(agent, path)| (agent, rule_status(&path).0))
        .collect()
}

pub fn run() -> Result<(), String> {
    let t = Theme::detect();
    let home = Config::home_dir();

    println!(
        "{}",
        t.header_rule("trm doctor", "environment & rules health check")
    );

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
    for (agent, path) in agent_rule_files(&home) {
        let (status, trm_cnt) = rule_status(&path);
        match status {
            "not_installed" => println!("  [{}] {:<12} not installed", t.dim("---"), agent),
            "unreadable" => println!("  [{}] {:<12} unreadable", t.red("err"), agent),
            "installed" => println!("  [{}] {:<12} installed cleanly", t.green("ok "), agent),
            "needs_fix" => println!(
                "  [{}] {:<12} duplicate markers (trm={trm_cnt}) — run 'trm install'",
                t.amber("fix"),
                agent
            ),
            _ => println!(
                "  [{}] {:<12} file exists, no trm block — run 'trm install'",
                t.amber("off"),
                agent
            ),
        }
    }

    let hook_path = home.join(".claude/hooks/trm-rewrite.sh");
    if hook_path.exists() {
        let executable = is_executable(&hook_path);
        let settings_wired = fs::read_to_string(home.join(".claude/settings.json"))
            .is_ok_and(|s| s.contains("trm-rewrite.sh"));
        if executable && settings_wired {
            println!(
                "  [{}] {:<12} installed & active in settings.json",
                t.green("ok "),
                "claude hook"
            );
        } else if !executable {
            println!(
                "  [{}] {:<12} not executable — run 'trm install --hook'",
                t.amber("fix"),
                "claude hook"
            );
        } else {
            println!(
                "  [{}] {:<12} script installed (not wired in ~/.claude/settings.json)",
                t.amber("off"),
                "claude hook"
            );
        }
    } else {
        println!(
            "  [{}] {:<12} not installed (optional: 'trm install --hook')",
            t.dim("---"),
            "claude hook"
        );
    }

    let agy_hooks_path = home.join(".gemini/config/hooks.json");
    if agy_hooks_path.exists()
        && fs::read_to_string(&agy_hooks_path).is_ok_and(|s| s.contains("trm-rewrite"))
    {
        println!(
            "  [{}] {:<12} active in ~/.gemini/config/hooks.json",
            t.green("ok "),
            "agy hook"
        );
    } else {
        println!(
            "  [{}] {:<12} not installed (optional: 'trm install --hook --only agy')",
            t.dim("---"),
            "agy hook"
        );
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
        println!(
            "  [{}] telemetry: {} ({lines} events)",
            t.green("ok "),
            log_path.display()
        );
    } else {
        println!(
            "  [{}] telemetry: not created yet ({})",
            t.dim("---"),
            log_path.display()
        );
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
