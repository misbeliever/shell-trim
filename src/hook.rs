use std::io::{self, Read, Write};

use crate::json::{JsonParser, JsonValue, json_string};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookTarget {
    Agy,
    Claude,
}

const PREFIXES: &[&str] = &[
    "cat ",
    "head ",
    "tail ",
    "sed -n ",
    "rg ",
    "grep ",
    "ls ",
    "find ",
    "git status",
    "git diff",
    "git log",
    "git show",
    "swift build",
    "swift test",
    "swiftc ",
    "cargo build",
    "cargo test",
    "cargo check",
    "cargo clippy",
];

pub fn run(target: HookTarget) -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("failed to read hook stdin: {e}"))?;

    if input.trim().is_empty() {
        return Ok(());
    }

    let out = render_hook_output(target, &input);
    if !out.is_empty() {
        let mut stdout = io::stdout();
        stdout
            .write_all(out.as_bytes())
            .and_then(|_| stdout.write_all(b"\n"))
            .map_err(|e| format!("write failed: {e}"))?;
    }
    Ok(())
}

pub(crate) fn render_hook_output(target: HookTarget, input: &str) -> String {
    let Ok(val) = JsonParser::parse(input) else {
        return match target {
            HookTarget::Agy => "{\"decision\":\"allow\"}".to_string(),
            HookTarget::Claude => String::new(),
        };
    };

    match target {
        HookTarget::Claude => {
            let cmd = val
                .get("tool_input")
                .and_then(|v| v.get("command"))
                .and_then(JsonValue::as_str);
            match cmd.and_then(rewrite_shell_command) {
                Some(rewritten) => format!(
                    "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{{\"command\":{}}}}}}}",
                    json_string(&rewritten)
                ),
                None => String::new(),
            }
        }
        HookTarget::Agy => {
            let cmd = val
                .get("toolCall")
                .and_then(|v| v.get("args"))
                .and_then(|v| v.get("CommandLine"))
                .and_then(JsonValue::as_str);
            match cmd.and_then(rewrite_shell_command) {
                Some(rewritten) => format!(
                    "{{\"decision\":\"allow\",\"overwrite\":{{\"CommandLine\":{}}}}}",
                    json_string(&rewritten)
                ),
                None => "{\"decision\":\"allow\"}".to_string(),
            }
        }
    }
}

pub fn rewrite_shell_command(cmd: &str) -> Option<String> {
    if cmd.is_empty() || cmd.contains('\n') || cmd.contains('`') || cmd.contains("$(") {
        return None;
    }

    let parts = split_and_outside_quotes(cmd)?;
    let mut changed = false;
    let mut new_parts = Vec::with_capacity(parts.len());

    for part in parts {
        let s = part.trim();
        if should_wrap(s) {
            new_parts.push(format!("trm {s}"));
            changed = true;
        } else {
            new_parts.push(s.to_string());
        }
    }

    if changed {
        Some(new_parts.join(" && "))
    } else {
        None
    }
}

fn split_and_outside_quotes(cmd: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();

    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            buf.push(ch);
            if ch == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    buf.push(next);
                }
            } else if ch == q {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
            buf.push(ch);
        } else if matches!(ch, '|' | '>' | '<' | ';') {
            return None;
        } else if ch == '&' && chars.peek() == Some(&'&') {
            chars.next();
            parts.push(std::mem::take(&mut buf));
        } else {
            buf.push(ch);
        }
    }

    if quote.is_some() {
        return None;
    }
    parts.push(buf);
    Some(parts)
}

fn should_wrap(seg: &str) -> bool {
    if seg.is_empty() || seg.starts_with("trm ") {
        return false;
    }
    if seg == "ls" || seg == "find" {
        return true;
    }
    if !PREFIXES.iter().any(|p| seg.starts_with(p)) {
        return false;
    }
    if seg.starts_with("grep ")
        && [
            "--include",
            "--exclude",
            "--exclude-dir",
            "--exclude-from",
            "--line-buffered",
        ]
        .iter()
        .any(|f| seg.contains(f))
    {
        return false;
    }
    if seg.starts_with("find ")
        && [" -delete", " -exec", " -execdir", " -ok"]
            .iter()
            .any(|f| seg.contains(f))
    {
        return false;
    }
    if seg.starts_with("git diff")
        && [
            "--stat",
            "--name-only",
            "--numstat",
            "--shortstat",
            "--name-status",
        ]
        .iter()
        .any(|f| seg.contains(f))
    {
        return false;
    }
    if seg.starts_with("git status")
        && [" -s", "--short", "--porcelain"]
            .iter()
            .any(|f| seg.contains(f))
    {
        return false;
    }
    if seg.starts_with("git log")
        && [
            "--oneline",
            "--stat",
            "--format",
            "--pretty",
            "-p",
            "--patch",
        ]
        .iter()
        .any(|f| seg.contains(f))
    {
        return false;
    }
    if let Some(after_show) = seg.strip_prefix("git show") {
        let rest: Vec<&str> = after_show.split_whitespace().collect();
        if rest.iter().any(|a| {
            matches!(
                *a,
                "-s" | "--no-patch"
                    | "--oneline"
                    | "--stat"
                    | "--shortstat"
                    | "--numstat"
                    | "--name-only"
                    | "--name-status"
            ) || a.starts_with("--format")
                || a.starts_with("--pretty")
                || (!a.starts_with('-') && a.contains(':'))
        }) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_raw_commands_and_chains_safely() {
        assert_eq!(
            rewrite_shell_command("cat src/main.rs && cargo test"),
            Some("trm cat src/main.rs && trm cargo test".to_string())
        );
        assert_eq!(
            rewrite_shell_command("cat \"résumé.txt\" && cargo test"),
            Some("trm cat \"résumé.txt\" && trm cargo test".to_string())
        );
        assert_eq!(
            rewrite_shell_command("git show HEAD"),
            Some("trm git show HEAD".to_string())
        );
        assert_eq!(rewrite_shell_command("git show HEAD:src/main.rs"), None);
        assert_eq!(rewrite_shell_command("cat a.txt | trm"), None);
        assert_eq!(
            rewrite_shell_command("rg \"foo|bar\" src/"),
            Some("trm rg \"foo|bar\" src/".to_string())
        );
        assert_eq!(
            rewrite_shell_command("grep -rnE \"unwrap|panic\" src --include=*.rs"),
            None
        );
        assert_eq!(rewrite_shell_command("trm rg foo src/"), None);
    }

    #[test]
    fn renders_claude_and_agy_hook_contracts() {
        let claude_in = r#"{"tool_input":{"command":"rg foo src/"}}"#;
        let claude_out = render_hook_output(HookTarget::Claude, claude_in);
        assert!(claude_out.contains("updatedInput"));
        assert!(claude_out.contains("trm rg foo src/"));

        let agy_in = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"git diff"}}}"#;
        let agy_out = render_hook_output(HookTarget::Agy, agy_in);
        assert!(agy_out.contains("\"decision\":\"allow\""));
        assert!(agy_out.contains("\"CommandLine\":\"trm git diff\""));

        let unchanged = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"echo ok"}}}"#;
        assert_eq!(
            render_hook_output(HookTarget::Agy, unchanged),
            "{\"decision\":\"allow\"}"
        );
    }

    #[test]
    fn hook_fails_open_on_malformed_or_unexpected_json() {
        for bad in ["{not valid json", "null", "[]", r#"{"toolCall":{}}"#] {
            assert_eq!(
                render_hook_output(HookTarget::Agy, bad),
                "{\"decision\":\"allow\"}",
                "agy hook must fail open for input: {bad}"
            );
            assert_eq!(
                render_hook_output(HookTarget::Claude, bad),
                "",
                "claude hook must fail open for input: {bad}"
            );
        }
    }
}
