use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::cli::InstallOptions;
use crate::style::Theme;

const MARKER_START: &str = "<!-- trm:start -->";
const MARKER_END: &str = "<!-- trm:end -->";
const LEGACY_START: &str = "<!-- strm:start -->";
const LEGACY_END: &str = "<!-- strm:end -->";

const CORE_RULE_LINES: &str = "\
## Shell Trimming (`trm`) — MANDATORY
NEVER run raw `cat`, `head`, `tail`, `sed -n`, `rg`, `grep`, `ls`, `find`, `git status/diff/log`, `swift`, `swiftc`, or `cargo` — ALWAYS prefix with `trm`:
- Read: `trm cat <f>`, `trm sed -n 'A,Bp' <f>`, `trm head/tail -n N <f>`, or `trm read <f> [-r A-B | -s <sym> | -c 300]`
- Search, FS, Git & Build: `trm rg <pat>`, `trm ls`/`find`, `trm git status|diff|log`, `trm swift`/`swiftc`/`cargo` (`--all` or `--raw` for full)";

const CLAUDE_HOOK_SCRIPT: &str = r#"#!/usr/bin/env bash
# Optional Claude Code PreToolUse hook installed by `trm install --hook`.
# Safely rewrites raw cat/head/tail/sed -n/rg/grep/ls/find/git/swift/cargo calls
# (including simple `&&` chains) to `trm`, while skipping pipes, redirects, and heredocs.
set -euo pipefail
python3 -c '
import json, sys

try:
    cmd = json.load(sys.stdin).get("tool_input", {}).get("command", "")
except Exception:
    sys.exit(0)

if not cmd or any(ch in cmd for ch in ("\n", "|", ">", "<", "`", "$(")):
    sys.exit(0)

# Split on && outside quotes
parts, buf, quote, i = [], [], None, 0
while i < len(cmd):
    c = cmd[i]
    if quote:
        buf.append(c)
        if c == "\\" and quote == "\"" and i + 1 < len(cmd):
            i += 1
            buf.append(cmd[i])
        elif c == quote:
            quote = None
    elif c in ("\"", "\x27"):
        quote = c
        buf.append(c)
    elif c == "&" and i + 1 < len(cmd) and cmd[i + 1] == "&":
        parts.append("".join(buf))
        buf = []
        i += 1
    else:
        buf.append(c)
    i += 1
if quote:
    sys.exit(0)
parts.append("".join(buf))

PREFIXES = (
    "cat ", "head ", "tail ", "sed -n ", "rg ", "grep ", "ls ", "find ",
    "git status", "git diff", "git log",
    "swift build", "swift test", "swiftc ",
    "cargo build", "cargo test", "cargo check", "cargo clippy",
)

def should_wrap(seg: str) -> bool:
    s = seg.strip()
    if not s or s.startswith("trm ") or ";" in s:
        return False
    if s == "ls" or s == "find":
        return True
    if not s.startswith(PREFIXES):
        return False
    if s.startswith("find ") and any(f in s for f in (" -delete", " -exec", " -execdir", " -ok")):
        return False
    if s.startswith("git diff") and any(f in s for f in ("--stat", "--name-only", "--numstat")):
        return False
    if s.startswith("git status") and any(f in s for f in (" -s", "--short", "--porcelain")):
        return False
    if s.startswith("git log") and "--oneline" in s:
        return False
    return True

changed = False
new_parts = []
for p in parts:
    s = p.strip()
    if should_wrap(s):
        new_parts.append("trm " + s)
        changed = True
    else:
        new_parts.append(s)

if changed:
    out = " && ".join(new_parts)
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": {"command": out}}}))
'
"#;

fn rule_block_for(agent: &str) -> String {
    let extra = match agent {
        "antigravity" => Some(
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
        format!("---\ndescription: Always use trm for shell file reads, searches, git, and builds\nalwaysApply: true\n---\n\n{body}")
    } else {
        body
    }
}

fn home_dir() -> PathBuf {
    if let Ok(h) = env::var("TRM_HOME") {
        return PathBuf::from(h);
    }
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn want(only: &Option<String>, target: &str) -> bool {
    only.as_ref().is_none_or(|o| o == target)
}

pub fn run_install(opts: &InstallOptions) -> Result<(), String> {
    if !opts.dry_run && io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none() {
        play_install_banner();
    }

    let home = home_dir();
    let mut actions = 0usize;

    // 1. Codex: ~/.codex/AGENTS.md
    if want(&opts.only, "codex") {
        let codex_dir = home.join(".codex");
        if codex_dir.exists() || opts.only.as_deref() == Some("codex") {
            let target = codex_dir.join("AGENTS.md");
            let status = upsert_rule_block(&target, &rule_block_for("codex"), opts.dry_run)?;
            println!("  [codex]       {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // 2. Antigravity: ~/.gemini/config/GEMINI.md
    if want(&opts.only, "antigravity") {
        let gemini_dir = home.join(".gemini");
        if gemini_dir.exists() || opts.only.as_deref() == Some("antigravity") {
            let target = gemini_dir.join("config").join("GEMINI.md");
            let status = upsert_rule_block(&target, &rule_block_for("antigravity"), opts.dry_run)?;
            println!("  [antigravity] {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // 3. Claude Code: ~/.claude/CLAUDE.md
    if want(&opts.only, "claude") {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() || opts.only.as_deref() == Some("claude") {
            let target = claude_dir.join("CLAUDE.md");
            let status = upsert_rule_block(&target, &rule_block_for("claude"), opts.dry_run)?;
            println!("  [claude]      {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // 4. GitHub Copilot CLI: ~/.copilot/copilot-instructions.md
    if want(&opts.only, "copilot") {
        let copilot_dir = home.join(".copilot");
        if copilot_dir.exists() || opts.only.as_deref() == Some("copilot") {
            let target = copilot_dir.join("copilot-instructions.md");
            let status = upsert_rule_block(&target, &rule_block_for("copilot"), opts.dry_run)?;
            println!("  [copilot]     {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // 5. Pi coding agent: ~/.pi/agent/AGENTS.md
    if want(&opts.only, "pi") {
        let pi_dir = home.join(".pi");
        if pi_dir.exists() || opts.only.as_deref() == Some("pi") {
            let target = pi_dir.join("agent").join("AGENTS.md");
            let status = upsert_rule_block(&target, &rule_block_for("pi"), opts.dry_run)?;
            println!("  [pi]          {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // 6. Cursor: ~/.cursor/rules/trm.mdc
    if want(&opts.only, "cursor") {
        let cursor_dir = home.join(".cursor");
        if cursor_dir.exists() || opts.only.as_deref() == Some("cursor") {
            let target = cursor_dir.join("rules").join("trm.mdc");
            let status = upsert_rule_block(&target, &rule_block_for("cursor"), opts.dry_run)?;
            println!("  [cursor]      {status:<9} {}", display_path(&target, &home));
            actions += 1;
        }
    }

    // Optional Claude Code PreToolUse rewrite helper script
    if opts.hook && want(&opts.only, "claude") {
        let hook_path = home.join(".claude").join("hooks").join("trm-rewrite.sh");
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
        println!("  [claude-hook] {status:<9} {}", display_path(&hook_path, &home));
        actions += 1;
    }

    if actions == 0 {
        println!("No supported agent directories detected (~/.codex, ~/.gemini, ~/.claude, ~/.copilot, ~/.pi, ~/.cursor).");
        println!("Use --only codex|antigravity|claude|copilot|pi|cursor to install explicitly.");
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
    let home = home_dir();

    if want(&opts.only, "codex") {
        let target = home.join(".codex").join("AGENTS.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!("  [codex]       {status:<9} {}", display_path(&target, &home));
        }
    }

    if want(&opts.only, "antigravity") {
        let target = home.join(".gemini").join("config").join("GEMINI.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!("  [antigravity] {status:<9} {}", display_path(&target, &home));
        }
    }

    if want(&opts.only, "claude") {
        let target = home.join(".claude").join("CLAUDE.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!("  [claude]      {status:<9} {}", display_path(&target, &home));
        }
    }

    if want(&opts.only, "copilot") {
        let target = home.join(".copilot").join("copilot-instructions.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!("  [copilot]     {status:<9} {}", display_path(&target, &home));
        }
    }

    if want(&opts.only, "pi") {
        let target = home.join(".pi").join("agent").join("AGENTS.md");
        if target.exists() {
            let status = remove_rule_block(&target, opts.dry_run)?;
            println!("  [pi]          {status:<9} {}", display_path(&target, &home));
        }
    }

    if want(&opts.only, "cursor") {
        let target = home.join(".cursor").join("rules").join("trm.mdc");
        if target.exists() {
            if !opts.dry_run {
                let _ = fs::remove_file(&target);
            }
            println!("  [cursor]      {:<9} {}", "removed", display_path(&target, &home));
        }
    }

    if want(&opts.only, "claude") {
        let hook_path = home.join(".claude").join("hooks").join("trm-rewrite.sh");
        if hook_path.exists() {
            if !opts.dry_run {
                let _ = fs::remove_file(&hook_path);
            }
            println!("  [claude-hook] {:<9} {}", "removed", display_path(&hook_path, &home));
        }
    }

    if opts.dry_run {
        println!("\n(dry-run: no files were modified)");
    } else {
        println!("\ntrm rules removed.");
    }

    Ok(())
}

fn strip_fenced(existing: &str, start_m: &str, end_m: &str) -> Option<String> {
    let (Some(s), Some(e)) = (existing.find(start_m), existing.find(end_m)) else {
        return None;
    };
    if e < s {
        return None;
    }
    let before = existing[..s].trim_end();
    let after = existing[e + end_m.len()..].trim_start();
    let cleaned = match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("{after}\n"),
        (false, true) => format!("{before}\n"),
        (false, false) => format!("{before}\n\n{after}\n"),
    };
    Some(cleaned)
}

fn upsert_rule_block(path: &Path, block: &str, dry_run: bool) -> Result<&'static str, String> {
    let orig = if path.exists() {
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
    } else {
        String::new()
    };

    // Migrate away any legacy <!-- strm:start -->...<!-- strm:end --> block automatically
    let had_legacy = orig.contains(LEGACY_START);
    let existing = strip_fenced(&orig, LEGACY_START, LEGACY_END).unwrap_or_else(|| orig.clone());

    let (new_content, status) = if let (Some(s), Some(e)) =
        (existing.find(MARKER_START), existing.find(MARKER_END))
    {
        if e >= s {
            let end_pos = e + MARKER_END.len();
            let updated = format!("{}{}{}", &existing[..s], block, &existing[end_pos..]);
            if updated == orig {
                return Ok("unchanged");
            }
            (updated, if had_legacy { "migrated" } else { "updated" })
        } else {
            (
                append_block(&existing, block),
                if had_legacy { "migrated" } else { "installed" },
            )
        }
    } else {
        (
            append_block(&existing, block),
            if had_legacy { "migrated" } else { "installed" },
        )
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
