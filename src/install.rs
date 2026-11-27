use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::cli::InstallOptions;
use crate::config::Config;
use crate::json::{JsonParser, JsonValue};
use crate::style::Theme;

const MARKER_START: &str = "<!-- trm:start -->";
const MARKER_END: &str = "<!-- trm:end -->";

const CORE_RULE_LINES: &str = "\
## Shell Trimming (`trm`) — MANDATORY
NEVER run raw `cat`, `head`, `tail`, `sed -n`, `rg`, `grep`, `ls`, `find`, `git status/diff/log`, `swift`, `swiftc`, or `cargo` — ALWAYS prefix with `trm`:
- Read: `trm cat <f>`, `trm sed -n 'A,Bp' <f>`, `trm head/tail -n N <f>`, or `trm read <f> [-r A-B | -s <sym> | -c 300]`
- Search, FS, Git & Build: `trm rg <pat>`, `trm ls`/`find`, `trm git status|diff|log`, `trm swift`/`swiftc`/`cargo` (`--all` or `--raw` for full)";

const CLAUDE_HOOK_SCRIPT: &str = r#"#!/usr/bin/env bash
# Optional Claude Code PreToolUse hook installed by `trm install --hook`.
exec trm hook claude
"#;

fn rule_block_for(agent: &str) -> String {
    let extra = match agent {
        "agy" => Some(
            "- Native `read_file`: for code files >260L, prefer `trm read <f>` (outline) or `trm read <f> -s <sym>` over `read_file` (keep `read_file` for images/PDFs or small slices)",
        ),
        "claude" => Some(
            "- Native `Read`/`Grep`: for code files >260L or broad searches, prefer `trm read <f>` / `trm rg` over `Read`/`Grep` (keep `Read` for images/PDFs or small slices)",
        ),
        "copilot" => Some(
            "- Native `view`: for code files >260L, prefer `trm read <f>` (outline) or `trm read <f> -s <sym>` over full-file `view`",
        ),
        "pi" => Some(
            "- Native `read`: for code files >260L, prefer `trm read <f>` (outline) or `trm read <f> -s <sym>` over full-file `read`",
        ),
        "cursor" => Some(
            "- Prefer `trm read <f>` (outline) or `trm rg` in terminal commands for large files >260L",
        ),
        _ => None, // codex uses shell + apply_patch directly
    };
    let body = match extra {
        Some(line) => format!("{MARKER_START}\n{CORE_RULE_LINES}\n{line}\n{MARKER_END}"),
        None => format!("{MARKER_START}\n{CORE_RULE_LINES}\n{MARKER_END}"),
    };
    if agent == "cursor" {
        format!(
            "---\ndescription: Always use trm for shell file reads, searches, git, and builds\nalwaysApply: true\n---\n\n{body}"
        )
    } else {
        body
    }
}

fn want(only: &Option<String>, target: &str) -> bool {
    only.as_ref().is_none_or(|o| o == target)
}

pub fn run_install(opts: &InstallOptions) -> Result<(), String> {
    if !opts.dry_run && io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none() {
        play_install_banner();
    }

    let home = Config::home_dir();
    let mut actions = 0usize;

    // 1. Codex: ~/.codex/AGENTS.md
    if want(&opts.only, "codex") {
        let codex_dir = home.join(".codex");
        if codex_dir.exists() || opts.only.as_deref() == Some("codex") {
            let target = codex_dir.join("AGENTS.md");
            let status = upsert_rule_block(&target, &rule_block_for("codex"), opts.dry_run)?;
            println!(
                "  [codex]       {status:<9} {}",
                display_path(&target, &home)
            );
            actions += 1;
        }
    }

    // 2. Agy (Antigravity CLI): ~/.gemini/config/GEMINI.md
    if want(&opts.only, "agy") {
        let gemini_dir = home.join(".gemini");
        if gemini_dir.exists() || opts.only.as_deref() == Some("agy") {
            let target = gemini_dir.join("config").join("GEMINI.md");
            let status = upsert_rule_block(&target, &rule_block_for("agy"), opts.dry_run)?;
            println!(
                "  [agy]         {status:<9} {}",
                display_path(&target, &home)
            );
            actions += 1;
        }
    }

    // 3. Claude Code: ~/.claude/CLAUDE.md
    if want(&opts.only, "claude") {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() || opts.only.as_deref() == Some("claude") {
            let target = claude_dir.join("CLAUDE.md");
            let status = upsert_rule_block(&target, &rule_block_for("claude"), opts.dry_run)?;
            println!(
                "  [claude]      {status:<9} {}",
                display_path(&target, &home)
            );
            actions += 1;
        }
    }

    // 4. GitHub Copilot CLI: ~/.copilot/copilot-instructions.md
    if want(&opts.only, "copilot") {
        let copilot_dir = home.join(".copilot");
        if copilot_dir.exists() || opts.only.as_deref() == Some("copilot") {
            let target = copilot_dir.join("copilot-instructions.md");
            let status = upsert_rule_block(&target, &rule_block_for("copilot"), opts.dry_run)?;
            println!(
                "  [copilot]     {status:<9} {}",
                display_path(&target, &home)
            );
            actions += 1;
        }
    }

    // 5. Pi coding agent: ~/.pi/agent/AGENTS.md
    if want(&opts.only, "pi") {
        let pi_dir = home.join(".pi");
        if pi_dir.exists() || opts.only.as_deref() == Some("pi") {
            let target = pi_dir.join("agent").join("AGENTS.md");
            let status = upsert_rule_block(&target, &rule_block_for("pi"), opts.dry_run)?;
            println!(
                "  [pi]          {status:<9} {}",
                display_path(&target, &home)
            );
            actions += 1;
        }
    }

    // 6. Cursor: ~/.cursor/rules/trm.mdc
    if want(&opts.only, "cursor") {
        let cursor_dir = home.join(".cursor");
        if cursor_dir.exists() || opts.only.as_deref() == Some("cursor") {
            let target = cursor_dir.join("rules").join("trm.mdc");
            let status = upsert_rule_block(&target, &rule_block_for("cursor"), opts.dry_run)?;
            println!(
                "  [cursor]      {status:<9} {}",
                display_path(&target, &home)
            );
            println!(
                "    note: Cursor desktop IDE may also need rule pasted into Settings -> Rules -> User Rules."
            );
            actions += 1;
        }
    }

    // Optional Claude Code PreToolUse rewrite helper script
    if opts.hook && want(&opts.only, "claude") {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() || opts.only.as_deref() == Some("claude") {
            let hook_path = claude_dir.join("hooks").join("trm-rewrite.sh");
            let status = if opts.dry_run {
                "installed"
            } else {
                if let Some(parent) = hook_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                fs::write(&hook_path, CLAUDE_HOOK_SCRIPT)
                    .map_err(|e| format!("cannot write {}: {e}", hook_path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755));
                }
                "installed"
            };
            println!(
                "  [claude-hook] {status:<9} {}",
                display_path(&hook_path, &home)
            );
            if !opts.dry_run {
                println!(
                    "    Add to ~/.claude/settings.json:\n    {{\"hooks\":{{\"PreToolUse\":[{{\"matcher\":\"Bash\",\"hooks\":[{{\"type\":\"command\",\"command\":\"~/.claude/hooks/trm-rewrite.sh\"}}]}}]}}}}"
                );
            }
            actions += 1;
        }
    }

    // Optional Agy PreToolUse hook in ~/.gemini/config/hooks.json
    if opts.hook && want(&opts.only, "agy") {
        let gemini_dir = home.join(".gemini");
        if gemini_dir.exists() || opts.only.as_deref() == Some("agy") {
            let hooks_path = gemini_dir.join("config").join("hooks.json");
            let status = upsert_agy_hook(&hooks_path, opts.dry_run)?;
            println!(
                "  [agy-hook]    {status:<9} {}",
                display_path(&hooks_path, &home)
            );
            actions += 1;
        }
    }

    if actions == 0 {
        println!(
            "No supported agent directories detected (~/.codex, ~/.gemini, ~/.claude, ~/.copilot, ~/.pi, ~/.cursor)."
        );
        println!("Use --only codex|agy|claude|copilot|pi|cursor to install explicitly.");
    } else if opts.dry_run {
        println!("\n(dry-run: no files were modified)");
    } else {
        println!("\ntrm rules installed. Run 'trm uninstall' anytime to cleanly remove them.");
    }

    Ok(())
}

/// Fast (~350ms) TTY-only ASCII trim-sweep animation for interactive `trm install`.
fn play_install_banner() {
    const LOGO: [&str; 5] = [
        "  _                  ",
        " | |_ _ __ _ __ ___  ",
        " | __| '__| '_ ` _ \\ ",
        " | |_| |  | | | | | |",
        "  \\__|_|  |_| |_| |_|",
    ];
    const TAGLINE: &str = "  cut shell noise. keep the signal.";
    let theme = Theme::detect();
    let width = LOGO[0].len();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Hide cursor during sweep; restore before returning
    let _ = write!(out, "\x1b[?25l");
    let steps = 11usize;
    for step in 0..=steps {
        let reveal = (step * width) / steps;
        if step > 0 {
            let _ = write!(out, "\x1b[{}A", LOGO.len());
        }
        for (row_idx, line) in LOGO.iter().enumerate() {
            let clean = &line[..reveal];
            let noise_len = width.saturating_sub(reveal);
            let noise: String = (0..noise_len)
                .map(|j| match (j + step + row_idx) % 4 {
                    0 => '░',
                    1 => '▒',
                    2 => '·',
                    _ => ' ',
                })
                .collect();
            let suffix = if step == steps && row_idx == LOGO.len() - 1 {
                theme.dim(TAGLINE)
            } else {
                String::new()
            };
            let _ = writeln!(
                out,
                "{}{}{}\x1b[K",
                theme.brand(clean),
                theme.dim(&noise),
                suffix
            );
        }
        let _ = out.flush();
        if step < steps {
            thread::sleep(Duration::from_millis(32));
        }
    }
    let _ = writeln!(out, "\x1b[?25h");
    let _ = out.flush();
}

pub fn run_uninstall(opts: &InstallOptions) -> Result<(), String> {
    let home = Config::home_dir();

    if want(&opts.only, "codex") {
        let target = home.join(".codex").join("AGENTS.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!(
                "  [codex]       {status:<9} {}",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "agy") {
        let target = home.join(".gemini").join("config").join("GEMINI.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!(
                "  [agy]         {status:<9} {}",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "claude") {
        let target = home.join(".claude").join("CLAUDE.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!(
                "  [claude]      {status:<9} {}",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "copilot") {
        let target = home.join(".copilot").join("copilot-instructions.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!(
                "  [copilot]     {status:<9} {}",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "pi") {
        let target = home.join(".pi").join("agent").join("AGENTS.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!(
                "  [pi]          {status:<9} {}",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "cursor") {
        let target = home.join(".cursor").join("rules").join("trm.mdc");
        if target.exists() {
            if !opts.dry_run {
                let _ = fs::remove_file(&target);
            }
            println!(
                "  [cursor]      {:<9} {}",
                "removed",
                display_path(&target, &home)
            );
        }
    }

    if want(&opts.only, "claude") {
        let hook_path = home.join(".claude").join("hooks").join("trm-rewrite.sh");
        if hook_path.exists() {
            if !opts.dry_run {
                let _ = fs::remove_file(&hook_path);
            }
            println!(
                "  [claude-hook] {:<9} {}",
                "removed",
                display_path(&hook_path, &home)
            );
        }
    }

    if want(&opts.only, "agy") {
        let hooks_path = home.join(".gemini").join("config").join("hooks.json");
        if hooks_path.exists() {
            let status = remove_agy_hook(&hooks_path, opts.dry_run)?;
            if status != "unchanged" {
                println!(
                    "  [agy-hook]    {status:<9} {}",
                    display_path(&hooks_path, &home)
                );
            }
        }
    }

    if opts.dry_run {
        println!("\n(dry-run: no files were modified)");
    } else {
        println!("\ntrm rules removed.");
    }

    Ok(())
}

fn upsert_rule_block(path: &Path, block: &str, dry_run: bool) -> Result<&'static str, String> {
    let existing = if path.exists() {
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
    } else {
        String::new()
    };

    let (new_content, status) =
        if let (Some(s), Some(e)) = (existing.find(MARKER_START), existing.find(MARKER_END)) {
            if e >= s {
                let end_pos = e + MARKER_END.len();
                let updated = format!("{}{}{}", &existing[..s], block, &existing[end_pos..]);
                if updated == existing {
                    return Ok("unchanged");
                }
                (updated, "updated")
            } else {
                (append_block(&existing, block), "installed")
            }
        } else {
            (append_block(&existing, block), "installed")
        };

    if dry_run {
        return Ok(status);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, new_content).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(status)
}

fn append_block(existing: &str, block: &str) -> String {
    let trimmed = existing.trim_end();
    if trimmed.is_empty() {
        format!("{block}\n")
    } else {
        format!("{trimmed}\n\n{block}\n")
    }
}

fn remove_rule_block(path: &Path, dry_run: bool) -> Result<&'static str, String> {
    let existing =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let (Some(s), Some(e)) = (existing.find(MARKER_START), existing.find(MARKER_END)) else {
        return Ok("absent");
    };
    if e < s {
        return Ok("absent");
    }

    let before = existing[..s].trim_end();
    let after = existing[e + MARKER_END.len()..].trim_start();
    let cleaned = match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("{after}\n"),
        (false, true) => format!("{before}\n"),
        (false, false) => format!("{before}\n\n{after}\n"),
    };

    if dry_run {
        return Ok("removed");
    }

    if cleaned.is_empty() {
        fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    } else {
        fs::write(path, cleaned).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok("removed")
}

fn display_path(path: &Path, home: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(home) {
        format!("~/{}", rel.display())
    } else {
        path.display().to_string()
    }
}

fn agy_hook_entry() -> JsonValue {
    JsonValue::Object(vec![(
        "PreToolUse".to_string(),
        JsonValue::Array(vec![JsonValue::Object(vec![
            (
                "matcher".to_string(),
                JsonValue::Str("run_command".to_string()),
            ),
            (
                "hooks".to_string(),
                JsonValue::Array(vec![JsonValue::Object(vec![
                    ("type".to_string(), JsonValue::Str("command".to_string())),
                    (
                        "command".to_string(),
                        JsonValue::Str("trm hook agy".to_string()),
                    ),
                    ("timeout".to_string(), JsonValue::Num(5.0)),
                ])]),
            ),
        ])]),
    )])
}

fn upsert_agy_hook(path: &Path, dry_run: bool) -> Result<&'static str, String> {
    let entry = agy_hook_entry();
    let (mut root, status) = if path.exists() {
        let existing =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut parsed = if existing.trim().is_empty() {
            JsonValue::Object(Vec::new())
        } else {
            JsonParser::parse(&existing)
                .map_err(|e| format!("cannot parse {}: {e}", path.display()))?
        };
        let Some(entries) = parsed.as_object_mut() else {
            return Err(format!("{} root must be a JSON object", path.display()));
        };
        if let Some((_, val)) = entries.iter_mut().find(|(k, _)| k == "trm-rewrite") {
            if *val == entry {
                return Ok("unchanged");
            }
            *val = entry;
            (parsed, "updated")
        } else {
            entries.push(("trm-rewrite".to_string(), entry));
            (parsed, "updated")
        }
    } else {
        (
            JsonValue::Object(vec![("trm-rewrite".to_string(), entry)]),
            "created",
        )
    };

    if !dry_run {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let _ = &mut root;
        fs::write(path, root.to_pretty_string(2))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(status)
}

fn remove_agy_hook(path: &Path, dry_run: bool) -> Result<&'static str, String> {
    let existing =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let Ok(mut parsed) = JsonParser::parse(&existing) else {
        return Ok("unchanged");
    };
    let Some(entries) = parsed.as_object_mut() else {
        return Ok("unchanged");
    };
    let before = entries.len();
    entries.retain(|(k, _)| k != "trm-rewrite");
    if entries.len() == before {
        return Ok("unchanged");
    }
    if entries.is_empty() {
        if !dry_run {
            let _ = fs::remove_file(path);
        }
        Ok("removed")
    } else {
        if !dry_run {
            fs::write(path, parsed.to_pretty_string(2))
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        Ok("updated")
    }
}
