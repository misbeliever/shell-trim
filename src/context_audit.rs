use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::cli::AuditOptions;
use crate::json::{JsonParser, json_string};
use crate::style::{Theme, fmt_compact};
use crate::telemetry;

struct RuleItem {
    agent: String,
    path: String,
    bytes: usize,
    tokens: usize,
    has_trm: bool,
}

struct SkillItem {
    agent: String,
    source: String,
    name: String,
    bytes: usize,
    tokens: usize,
}

#[derive(Clone)]
enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    Remote {
        url: String,
    },
}

#[derive(Clone)]
struct McpServerConfig {
    agent: String,
    name: String,
    source: String,
    transport: McpTransport,
}

struct McpProbeResult {
    status: String,
    tools_count: Option<usize>,
    schema_tokens: Option<usize>,
    tools: Vec<(String, usize)>, // (tool_name, tokens)
}

struct AgentTax {
    agent: &'static str,
    rules_tok: usize,
    skills_tok: usize,
    mcp_tok: usize,
}

impl AgentTax {
    fn total(&self) -> usize {
        self.rules_tok + self.skills_tok + self.mcp_tok
    }
}

#[derive(Debug, Default, Clone)]
pub struct ContextSnapshot {
    pub by_agent: Vec<(String, usize, usize, usize)>, // (agent, rules_tok, skills_tok, total_tok)
    pub skills_by_source: Vec<(String, usize, usize)>, // (source, count, tokens)
    pub enabled_skills: usize,
    pub disabled_plugins: Vec<String>,
    pub skipped_manual: usize,
    pub mcp_servers: Vec<(String, String, String)>, // (agent, name, transport)
}

pub fn load_snapshot() -> ContextSnapshot {
    let home = crate::config::Config::home_dir();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ancestors = repo_ancestors(&cwd);
    let want_all = |_: &str| true;

    let rules = scan_rules(&home, &ancestors, &want_all);
    let skill_scan = scan_skills(&home, &ancestors, &want_all);
    let servers = scan_mcp_configs(&home, &ancestors, &want_all);
    let probed: Vec<(McpServerConfig, Option<McpProbeResult>)> =
        servers.into_iter().map(|s| (s, None)).collect();

    let taxes = compute_agent_taxes(&home, &rules, &skill_scan.skills, &probed, None);
    let by_agent = taxes
        .into_iter()
        .map(|a| (a.agent.to_string(), a.rules_tok, a.skills_tok, a.total()))
        .collect();

    let mut by_src: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in &skill_scan.skills {
        let entry = by_src.entry(s.source.clone()).or_default();
        entry.0 += 1;
        entry.1 += s.tokens;
    }
    let mut skills_by_source: Vec<(String, usize, usize)> = by_src
        .into_iter()
        .map(|(src, (cnt, tok))| (src, cnt, tok))
        .collect();
    skills_by_source.sort_by_key(|r| std::cmp::Reverse(r.2));

    let mcp_servers = probed
        .into_iter()
        .map(|(srv, _)| {
            let kind = match srv.transport {
                McpTransport::Stdio { command, .. } => format!("stdio: {command}"),
                McpTransport::Remote { .. } => "remote".to_string(),
            };
            (srv.agent, srv.name, kind)
        })
        .collect();

    ContextSnapshot {
        by_agent,
        skills_by_source,
        enabled_skills: skill_scan.skills.len(),
        disabled_plugins: skill_scan.disabled_plugins,
        skipped_manual: skill_scan.skipped_manual,
        mcp_servers,
    }
}

pub fn run(opts: &AuditOptions) -> Result<(), String> {
    let home = crate::config::Config::home_dir();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ancestors = repo_ancestors(&cwd);

    let want_agent = |a: &str| opts.agent_filter.as_ref().is_none_or(|f| f == a);

    let rules = scan_rules(&home, &ancestors, &want_agent);
    let skill_scan = scan_skills(&home, &ancestors, &want_agent);
    let skills = skill_scan.skills;
    let disabled_plugins = skill_scan.disabled_plugins;
    let servers = scan_mcp_configs(&home, &ancestors, &want_agent);

    let mut probed: Vec<(McpServerConfig, Option<McpProbeResult>)> =
        Vec::with_capacity(servers.len());
    let mut top_tools: Vec<(String, usize)> = Vec::new();

    for srv in servers {
        let probe = if opts.probe_mcp {
            let res = probe_mcp_server(&srv.transport);
            for (tname, tok) in &res.tools {
                top_tools.push((format!("{}/{tname}", srv.name), *tok));
            }
            Some(res)
        } else {
            None
        };
        probed.push((srv, probe));
    }

    top_tools.sort_by_key(|t| std::cmp::Reverse(t.1));

    let rules_tokens: usize = rules.iter().map(|r| r.tokens).sum();
    let skills_tokens: usize = skills.iter().map(|s| s.tokens).sum();
    let mcp_tokens: usize = probed
        .iter()
        .filter_map(|(_, p)| p.as_ref().and_then(|r| r.schema_tokens))
        .sum();
    let total_tokens = rules_tokens + skills_tokens + mcp_tokens;
    let agent_taxes = compute_agent_taxes(
        &home,
        &rules,
        &skills,
        &probed,
        opts.agent_filter.as_deref(),
    );

    if opts.json {
        print_json(
            total_tokens,
            rules_tokens,
            skills_tokens,
            mcp_tokens,
            &agent_taxes,
            &rules,
            &skills,
            &disabled_plugins,
            &probed,
            &top_tools,
            opts.all,
        );
        return Ok(());
    }

    let t = Theme::detect();
    let subtitle = if opts.probe_mcp {
        "turn-0 rules, skills & live MCP schemas"
    } else {
        "turn-0 rules, skills & MCP configs (offline)"
    };
    println!("{}", t.header_rule("trm audit --context", subtitle));
    println!("\n{}", t.bold("Turn-0 tax by agent:"));
    if agent_taxes.is_empty() {
        println!("  (no configured agents detected)");
    } else {
        println!(
            "  {:<13} {:>7}  {:>7}  {:>7}  {:>9}",
            "agent", "rules", "skills", "mcp", "total"
        );
        for row in &agent_taxes {
            let mcp_cell = if opts.probe_mcp {
                fmt_compact(row.mcp_tok)
            } else {
                "-".to_string()
            };
            println!(
                "  {:<13} {:>7}  {:>7}  {:>7}  {:>9}",
                row.agent,
                fmt_compact(row.rules_tok),
                fmt_compact(row.skills_tok),
                mcp_cell,
                t.bold(&format!("{} tok", fmt_compact(row.total())))
            );
        }
    }
    println!();

    println!(
        "{}",
        t.bold(&format!(
            "Always-on rules ({} files, {} tok):",
            rules.len(),
            fmt_compact(rules_tokens)
        ))
    );
    if rules.is_empty() {
        println!("  (none detected)");
    } else {
        let rule_cap = if opts.all { rules.len() } else { 12 };
        for r in rules.iter().take(rule_cap) {
            let tag = if r.has_trm { " [trm]" } else { "" };
            println!(
                "  {:<12} {:>6} tok   {}{tag}",
                r.agent,
                fmt_compact(r.tokens),
                display_path(&r.path, &home)
            );
        }
        if !opts.all && rules.len() > 12 {
            println!(
                "  ... +{} more rules (use --all to list all)",
                rules.len() - 12
            );
        }
    }

    println!(
        "\n{}",
        t.bold(&format!(
            "Skills metadata ({} enabled skills, {} tok frontmatter):",
            skills.len(),
            fmt_compact(skills_tokens)
        ))
    );
    if !disabled_plugins.is_empty() {
        println!(
            "  (skipped {} disabled plugins: {})",
            disabled_plugins.len(),
            disabled_plugins.join(", ")
        );
    }
    if skill_scan.skipped_manual > 0 || skill_scan.skipped_excluded > 0 {
        let mut notes = Vec::new();
        if skill_scan.skipped_manual > 0 {
            notes.push(format!(
                "{} user-only (disable-model-invocation)",
                skill_scan.skipped_manual
            ));
        }
        if skill_scan.skipped_excluded > 0 {
            notes.push(format!(
                "{} excluded via skills.json",
                skill_scan.skipped_excluded
            ));
        }
        println!("  (skipped {})", notes.join(", "));
    }
    if skills.is_empty() {
        println!("  (none detected)");
    } else {
        let mut by_src: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for s in &skills {
            let entry = by_src.entry(s.source.as_str()).or_default();
            entry.0 += 1;
            entry.1 += s.tokens;
        }
        let mut src_rows: Vec<(&str, usize, usize)> = by_src
            .into_iter()
            .map(|(src, (cnt, tok))| (src, cnt, tok))
            .collect();
        src_rows.sort_by_key(|r| std::cmp::Reverse(r.2));

        println!("  By source:");
        for (src, cnt, tok) in src_rows {
            println!(
                "    {:<28} {:>3} skills  {:>6} tok",
                src,
                cnt,
                fmt_compact(tok)
            );
        }

        println!("  Skills:");
        let skill_cap = if opts.all { skills.len() } else { 8 };
        for s in skills.iter().take(skill_cap) {
            println!(
                "    {:<28} {:>6} tok   {}",
                s.source,
                fmt_compact(s.tokens),
                s.name
            );
        }
        if !opts.all && skills.len() > 8 {
            println!(
                "    ... +{} more skills (use --all to list all)",
                skills.len() - 8
            );
        }
    }

    println!(
        "\n{}",
        t.bold(&format!("MCP servers ({} configured):", probed.len()))
    );
    if probed.is_empty() {
        println!("  (none detected)");
    } else {
        for (srv, probe) in &probed {
            let src = display_path(&srv.source, &home);
            match probe {
                Some(p) => {
                    let tools = p
                        .tools_count
                        .map_or("-".to_string(), |n| format!("{n} tools"));
                    let tok = p
                        .schema_tokens
                        .map_or("-".to_string(), |n| format!("{} tok", fmt_compact(n)));
                    println!(
                        "  {:<12} {:<20} {:>9}  {:>9}  [{}]  ({src})",
                        srv.agent, srv.name, tools, tok, p.status
                    );
                }
                None => {
                    let kind = match &srv.transport {
                        McpTransport::Stdio { command, .. } => format!("stdio: {command}"),
                        McpTransport::Remote { .. } => "remote".to_string(),
                    };
                    println!("  {:<12} {:<20} {:<22} ({src})", srv.agent, srv.name, kind);
                }
            }
        }
        if !opts.probe_mcp {
            println!(
                "  Tip: run `trm audit --context --probe-mcp` to measure exact MCP schema tokens."
            );
        }
    }

    if !top_tools.is_empty() {
        println!("\n{}", t.bold("Top heaviest MCP tools:"));
        let tool_cap = if opts.all { top_tools.len() } else { 10 };
        for (name, tok) in top_tools.iter().take(tool_cap) {
            println!("  {:>6} tok   {name}", fmt_compact(*tok));
        }
    }

    Ok(())
}

fn repo_ancestors(cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(dir) = cur {
        let has_git = dir.join(".git").exists();
        out.push(dir.clone());
        if has_git {
            break;
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    out
}

fn scan_rules<F>(home: &Path, ancestors: &[PathBuf], want_agent: &F) -> Vec<RuleItem>
where
    F: Fn(&str) -> bool,
{
    let mut candidates: Vec<(&str, PathBuf, bool)> = Vec::new();

    if want_agent("codex") {
        candidates.push(("codex", home.join(".codex/AGENTS.md"), false));
    }
    if want_agent("agy") {
        candidates.push(("agy", home.join(".gemini/config/GEMINI.md"), false));
    }
    if want_agent("claude") {
        candidates.push(("claude", home.join(".claude/CLAUDE.md"), false));
    }
    if want_agent("copilot") {
        candidates.push((
            "copilot",
            home.join(".copilot/copilot-instructions.md"),
            false,
        ));
    }
    if want_agent("pi") {
        candidates.push(("pi", home.join(".pi/agent/AGENTS.md"), false));
    }
    if want_agent("cursor") {
        collect_dir_files(
            &home.join(".cursor/rules"),
            "mdc",
            "cursor",
            true,
            &mut candidates,
        );
    }

    if want_agent("repo") {
        for dir in ancestors {
            for file in ["AGENTS.md", "CLAUDE.md", "GEMINI.md"] {
                candidates.push(("repo", dir.join(file), false));
            }
            collect_dir_files(
                &dir.join(".cursor/rules"),
                "mdc",
                "repo",
                true,
                &mut candidates,
            );
            collect_dir_files(
                &dir.join(".agents/rules"),
                "md",
                "repo",
                true,
                &mut candidates,
            );
        }
    }

    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (agent, path, check_frontmatter) in candidates {
        if !path.is_file() {
            continue;
        }
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen.insert(key) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if check_frontmatter && is_opt_out_rule(&content) {
            continue;
        }
        let bytes = content.len();
        let tokens = telemetry::estimate_tokens_for_lang(bytes, "markdown");
        out.push(RuleItem {
            agent: agent.to_string(),
            path: path.to_string_lossy().into_owned(),
            bytes,
            tokens,
            has_trm: content.contains("<!-- trm:start -->"),
        });
    }

    out.sort_by_key(|r| std::cmp::Reverse(r.tokens));
    out
}

fn is_opt_out_rule(content: &str) -> bool {
    let Some(fm) = extract_frontmatter(content) else {
        return false;
    };
    for line in fm.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("alwaysApply:")
            && v.trim().eq_ignore_ascii_case("false")
        {
            return true;
        }
        if let Some(v) = t.strip_prefix("trigger:")
            && v.trim().eq_ignore_ascii_case("model_decision")
        {
            return true;
        }
    }
    false
}

fn collect_dir_files(
    dir: &Path,
    ext: &str,
    agent: &'static str,
    check_frontmatter: bool,
    out: &mut Vec<(&'static str, PathBuf, bool)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push((agent, p, check_frontmatter));
        }
    }
}

fn load_agy_plugin_overrides(home: &Path) -> BTreeMap<String, bool> {
    let mut overrides = BTreeMap::new();
    for rel in [
        ".gemini/config/config.json",
        ".gemini/antigravity-cli/config.json",
    ] {
        let path = home.join(rel);
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(root) = JsonParser::parse(&raw) else {
            continue;
        };
        let Some(plugins) = root.get("plugins").and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, cfg) in plugins {
            if let Some(enabled) = cfg.get("enabled").and_then(|v| v.as_bool()) {
                overrides.insert(name.clone(), enabled);
            }
        }
    }
    overrides
}

fn is_plugin_enabled(
    plugin_dir: &Path,
    dir_name: &str,
    overrides: &BTreeMap<String, bool>,
) -> bool {
    if let Some(&enabled) = overrides.get(dir_name) {
        return enabled;
    }
    for rel in [
        "plugin.json",
        ".antigravity-plugin/plugin.json",
        ".gemini-plugin/plugin.json",
    ] {
        let p = plugin_dir.join(rel);
        if let Ok(raw) = fs::read_to_string(p)
            && let Ok(val) = JsonParser::parse(&raw)
        {
            if val.get("disabled").and_then(|v| v.as_bool()) == Some(true)
                || val.get("enabled").and_then(|v| v.as_bool()) == Some(false)
            {
                return false;
            }
            break;
        }
    }
    true
}

struct SkillScanResult {
    skills: Vec<SkillItem>,
    disabled_plugins: Vec<String>,
    skipped_manual: usize,
    skipped_excluded: usize,
}

fn load_skills_json_excludes(home: &Path, ancestors: &[PathBuf]) -> Vec<String> {
    let mut paths = vec![home.join(".gemini/config/skills.json")];
    for a in ancestors {
        paths.push(a.join(".agents/skills.json"));
    }
    let mut patterns = Vec::new();
    for path in paths {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(root) = JsonParser::parse(&raw) else {
            continue;
        };
        collect_exclude_strings(root.get("exclude"), &mut patterns);
        for key in ["entries", "inherits"] {
            if let Some(arr) = root.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    collect_exclude_strings(item.get("exclude"), &mut patterns);
                }
            }
        }
    }
    patterns
}

fn collect_exclude_strings(val: Option<&crate::json::JsonValue>, out: &mut Vec<String>) {
    if let Some(arr) = val.and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str()
                && !s.is_empty()
            {
                out.push(s.to_string());
            }
        }
    }
}

// ponytail: simple glob/regex-prefix matcher; swap in regex crate only if complex patterns appear
fn matches_pattern(name: &str, pat: &str) -> bool {
    if name == pat {
        return true;
    }
    if let Some(prefix) = pat.strip_suffix(".*").or_else(|| pat.strip_suffix('*')) {
        return name.starts_with(prefix);
    }
    if let Some(suffix) = pat.strip_prefix(".*").or_else(|| pat.strip_prefix('*')) {
        return name.ends_with(suffix);
    }
    false
}

fn scan_skills<F>(home: &Path, ancestors: &[PathBuf], want_agent: &F) -> SkillScanResult
where
    F: Fn(&str) -> bool,
{
    let mut dirs: Vec<(&str, String, PathBuf)> = Vec::new();
    let mut disabled_plugins = Vec::new();

    if want_agent("codex") || want_agent("agy") {
        dirs.push((
            "agents",
            "~/.agents".to_string(),
            home.join(".agents/skills"),
        ));
    }
    if want_agent("codex") {
        dirs.push(("codex", "~/.codex".to_string(), home.join(".codex/skills")));
    }
    if want_agent("agy") {
        dirs.push((
            "agy",
            "~/.gemini/config".to_string(),
            home.join(".gemini/config/skills"),
        ));
        dirs.push((
            "agy",
            "~/.gemini/skills".to_string(),
            home.join(".gemini/skills"),
        ));
        let overrides = load_agy_plugin_overrides(home);
        if let Ok(plugins) = fs::read_dir(home.join(".gemini/config/plugins")) {
            for p in plugins.flatten() {
                let path = p.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = p.file_name().to_string_lossy().into_owned();
                if is_plugin_enabled(&path, &dir_name, &overrides) {
                    dirs.push(("agy", format!("plugin:{dir_name}"), path.join("skills")));
                } else {
                    disabled_plugins.push(dir_name);
                }
            }
        }
    }
    if want_agent("claude") {
        dirs.push((
            "claude",
            "~/.claude".to_string(),
            home.join(".claude/skills"),
        ));
    }
    if want_agent("cursor") {
        dirs.push((
            "cursor",
            "~/.cursor".to_string(),
            home.join(".cursor/skills"),
        ));
    }
    if want_agent("copilot") {
        dirs.push((
            "copilot",
            "~/.copilot".to_string(),
            home.join(".copilot/skills"),
        ));
    }
    if want_agent("pi") {
        dirs.push((
            "pi",
            "~/.pi/agent".to_string(),
            home.join(".pi/agent/skills"),
        ));
    }
    if want_agent("repo") {
        for a in ancestors {
            dirs.push(("repo", "repo:.agents".to_string(), a.join(".agents/skills")));
            dirs.push(("repo", "repo:.github".to_string(), a.join(".github/skills")));
        }
    }

    let excludes = load_skills_json_excludes(home, ancestors);
    let mut seen_paths = BTreeSet::new();
    let mut seen_names: BTreeSet<(&str, String)> = BTreeSet::new();
    let mut out = Vec::new();
    let mut skipped_manual = 0usize;
    let mut skipped_excluded = 0usize;

    for (agent, source, root) in dirs {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let skill_file = entry.path().join("SKILL.md");
            if !skill_file.is_file() {
                continue;
            }
            let key = fs::canonicalize(&skill_file).unwrap_or_else(|_| skill_file.clone());
            if !seen_paths.insert(key) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&skill_file) else {
                continue;
            };
            let fallback_name = entry.file_name().to_string_lossy().into_owned();
            let (name, desc, model_invocable) = parse_skill_frontmatter(&content, &fallback_name);
            if !model_invocable {
                skipped_manual += 1;
                continue;
            }
            if excludes
                .iter()
                .any(|p| matches_pattern(&fallback_name, p) || matches_pattern(&name, p))
            {
                skipped_excluded += 1;
                continue;
            }
            if !seen_names.insert((agent, name.to_lowercase())) {
                continue;
            }
            let meta_bytes = name.len() + desc.len() + 3;
            let tokens = telemetry::estimate_tokens_for_lang(meta_bytes, "markdown");
            out.push(SkillItem {
                agent: agent.to_string(),
                source: source.clone(),
                name,
                bytes: meta_bytes,
                tokens,
            });
        }
    }

    out.sort_by_key(|s| std::cmp::Reverse(s.tokens));
    disabled_plugins.sort();
    disabled_plugins.dedup();
    SkillScanResult {
        skills: out,
        disabled_plugins,
        skipped_manual,
        skipped_excluded,
    }
}

fn extract_frontmatter(content: &str) -> Option<&str> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    if let Some(end) = rest.find("\n---") {
        Some(&rest[..end])
    } else {
        None
    }
}

fn parse_skill_frontmatter(content: &str, fallback_name: &str) -> (String, String, bool) {
    let Some(fm) = extract_frontmatter(content) else {
        return (fallback_name.to_string(), String::new(), true);
    };
    let mut name = fallback_name.to_string();
    let mut desc = String::new();
    let mut model_invocable = true;
    let lines: Vec<&str> = fm.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("name:") {
            let v = val.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                name = v.to_string();
            }
        } else if let Some(val) = trimmed
            .strip_prefix("disable-model-invocation:")
            .or_else(|| trimmed.strip_prefix("disable_model_invocation:"))
        {
            if val.trim().eq_ignore_ascii_case("true") {
                model_invocable = false;
            }
        } else if let Some(val) = trimmed.strip_prefix("model_invocation:") {
            if val.trim().eq_ignore_ascii_case("false") {
                model_invocable = false;
            }
        } else if let Some(val) = trimmed.strip_prefix("description:") {
            let first = val.trim();
            if matches!(first, ">" | ">-" | "|" | "|-") {
                let mut parts = Vec::new();
                while i + 1 < lines.len() {
                    let next = lines[i + 1];
                    if next.starts_with(' ') || next.starts_with('\t') || next.trim().is_empty() {
                        if !next.trim().is_empty() {
                            parts.push(next.trim());
                        }
                        i += 1;
                    } else {
                        break;
                    }
                }
                desc = parts.join(" ");
            } else {
                desc = first.trim_matches('"').trim_matches('\'').to_string();
            }
        }
        i += 1;
    }
    (name, desc, model_invocable)
}

fn scan_mcp_configs<F>(home: &Path, ancestors: &[PathBuf], want_agent: &F) -> Vec<McpServerConfig>
where
    F: Fn(&str) -> bool,
{
    let mut json_files: Vec<(&str, PathBuf)> = Vec::new();
    if want_agent("agy") {
        json_files.push(("agy", home.join(".gemini/config/mcp_config.json")));
        json_files.push(("agy", home.join(".gemini/antigravity/mcp_config.json")));
    }
    if want_agent("claude") {
        json_files.push(("claude", home.join(".claude.json")));
        json_files.push(("claude", home.join(".claude/settings.json")));
    }
    if want_agent("cursor") {
        json_files.push(("cursor", home.join(".cursor/mcp.json")));
    }
    if want_agent("copilot") {
        json_files.push(("copilot", home.join(".copilot/mcp-config.json")));
    }
    if want_agent("pi") {
        json_files.push(("pi", home.join(".pi/agent/mcp.json")));
    }
    if want_agent("repo") {
        for dir in ancestors {
            json_files.push(("repo", dir.join(".mcp.json")));
            json_files.push(("repo", dir.join(".cursor/mcp.json")));
        }
    }

    let mut out = Vec::new();
    let mut seen_files = BTreeSet::new();

    for (agent, path) in json_files {
        if !path.is_file() {
            continue;
        }
        let canon = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !seen_files.insert((agent, canon)) {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        parse_mcp_json(agent, &path.to_string_lossy(), &raw, &mut out);
    }

    if want_agent("codex") {
        let codex_toml = home.join(".codex/config.toml");
        if codex_toml.is_file()
            && let Ok(raw) = fs::read_to_string(&codex_toml)
        {
            parse_codex_mcp_toml(&codex_toml.to_string_lossy(), &raw, &mut out);
        }
    }

    out
}

fn parse_mcp_json(agent: &str, source: &str, raw: &str, out: &mut Vec<McpServerConfig>) {
    let Ok(root) = JsonParser::parse(raw) else {
        return;
    };
    let Some(servers) = root.get("mcpServers").and_then(|v| v.as_object()) else {
        return;
    };
    for (name, cfg) in servers {
        if cfg.get("disabled").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        if let Some(cmd) = cfg.get("command").and_then(|v| v.as_str()) {
            let args: Vec<String> = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let env: Vec<(String, String)> = cfg
                .get("env")
                .and_then(|v| v.as_object())
                .map(|pairs| {
                    pairs
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            out.push(McpServerConfig {
                agent: agent.to_string(),
                name: name.clone(),
                source: source.to_string(),
                transport: McpTransport::Stdio {
                    command: cmd.to_string(),
                    args,
                    env,
                },
            });
        } else if let Some(url) = cfg
            .get("url")
            .or_else(|| cfg.get("serverUrl"))
            .and_then(|v| v.as_str())
        {
            out.push(McpServerConfig {
                agent: agent.to_string(),
                name: name.clone(),
                source: source.to_string(),
                transport: McpTransport::Remote {
                    url: url.to_string(),
                },
            });
        }
    }
}

fn parse_codex_mcp_toml(source: &str, raw: &str, out: &mut Vec<McpServerConfig>) {
    let mut current_name: Option<String> = None;
    let mut command: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut url: Option<String> = None;
    let mut disabled = false;

    let mut flush = |name: &mut Option<String>,
                     cmd: &mut Option<String>,
                     args: &mut Vec<String>,
                     url: &mut Option<String>,
                     disabled: &mut bool| {
        if let Some(srv_name) = name.take()
            && !*disabled
        {
            if let Some(c) = cmd.take() {
                out.push(McpServerConfig {
                    agent: "codex".to_string(),
                    name: srv_name,
                    source: source.to_string(),
                    transport: McpTransport::Stdio {
                        command: c,
                        args: std::mem::take(args),
                        env: Vec::new(),
                    },
                });
            } else if let Some(u) = url.take() {
                out.push(McpServerConfig {
                    agent: "codex".to_string(),
                    name: srv_name,
                    source: source.to_string(),
                    transport: McpTransport::Remote { url: u },
                });
            }
        }
        *cmd = None;
        args.clear();
        *url = None;
        *disabled = false;
    };

    for raw_line in raw.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let hdr = line[1..line.len() - 1].trim();
            if let Some(rest) = hdr.strip_prefix("mcp_servers.") {
                if !rest.contains('.') {
                    flush(
                        &mut current_name,
                        &mut command,
                        &mut args,
                        &mut url,
                        &mut disabled,
                    );
                    current_name = Some(rest.trim_matches('"').trim_matches('\'').to_string());
                }
            } else {
                flush(
                    &mut current_name,
                    &mut command,
                    &mut args,
                    &mut url,
                    &mut disabled,
                );
            }
            continue;
        }
        if current_name.is_none() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "command" => command = Some(val.trim_matches('"').trim_matches('\'').to_string()),
            "url" => url = Some(val.trim_matches('"').trim_matches('\'').to_string()),
            "enabled" if val == "false" => disabled = true,
            "args" => args = parse_toml_string_array(val),
            _ => {}
        }
    }
    flush(
        &mut current_name,
        &mut command,
        &mut args,
        &mut url,
        &mut disabled,
    );
}

fn parse_toml_string_array(val: &str) -> Vec<String> {
    let inner = val.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut esc = false;
    for ch in inner.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        if let Some(q) = quote {
            if ch == '\\' && q == '"' {
                esc = true;
            } else if ch == q {
                quote = None;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        }
    }
    out
}

fn probe_mcp_server(transport: &McpTransport) -> McpProbeResult {
    let (command, args, env_pairs) = match transport {
        // ponytail: stdio-only MCP probe; add HTTP/SSE transport if needed
        McpTransport::Remote { url } => {
            return McpProbeResult {
                status: format!("remote skipped ({url})"),
                tools_count: None,
                schema_tokens: None,
                tools: Vec::new(),
            };
        }
        McpTransport::Stdio { command, args, env } => (command, args, env),
    };

    let mut cmd = Command::new(command);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return McpProbeResult {
                status: format!("spawn err: {e}"),
                tools_count: None,
                schema_tokens: None,
                tools: Vec::new(),
            };
        }
    };

    let mut stdin = child.stdin.take();
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return McpProbeResult {
            status: "no stdout".into(),
            tools_count: None,
            schema_tokens: None,
            tools: Vec::new(),
        };
    };

    let (tx, rx) = mpsc::channel::<Result<Vec<(String, usize)>, String>>();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(l) = line else {
                break;
            };
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(val) = JsonParser::parse(trimmed) else {
                continue;
            };
            if val.get("id").and_then(|v| v.as_u64()) != Some(2) {
                continue;
            }
            if let Some(err) = val.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("tools/list error")
                    .to_string();
                let _ = tx.send(Err(msg));
                return;
            }
            if let Some(tools) = val
                .get("result")
                .and_then(|r| r.get("tools"))
                .and_then(|t| t.as_array())
            {
                let parsed: Vec<(String, usize)> = tools
                    .iter()
                    .map(|tool| {
                        let name = tool
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("?")
                            .to_string();
                        let bytes = tool.to_compact_string().len();
                        (name, telemetry::estimate_tokens_for_lang(bytes, "json"))
                    })
                    .collect();
                let _ = tx.send(Ok(parsed));
                return;
            }
        }
        let _ = tx.send(Err("closed before tools/list".into()));
    });

    if let Some(ref mut w) = stdin {
        let reqs = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"trm\",\"version\":\"0.2.0\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
        );
        let _ = w.write_all(reqs.as_bytes());
        let _ = w.flush();
    }

    let started = Instant::now();
    let recv = rx.recv_timeout(Duration::from_secs(3));
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    match recv {
        Ok(Ok(tools)) => {
            let count = tools.len();
            let total_tok: usize = tools.iter().map(|(_, t)| *t).sum();
            McpProbeResult {
                status: format!("ok {}ms", started.elapsed().as_millis()),
                tools_count: Some(count),
                schema_tokens: Some(total_tok),
                tools,
            }
        }
        Ok(Err(e)) => McpProbeResult {
            status: e,
            tools_count: None,
            schema_tokens: None,
            tools: Vec::new(),
        },
        Err(_) => McpProbeResult {
            status: "timeout 3s".into(),
            tools_count: None,
            schema_tokens: None,
            tools: Vec::new(),
        },
    }
}

fn display_path(path: &str, home: &Path) -> String {
    let h = home.to_string_lossy();
    if !h.is_empty() && path.starts_with(h.as_ref()) {
        format!("~{}", &path[h.len()..])
    } else {
        path.to_string()
    }
}

fn agent_dir_exists(home: &Path, agent: &str) -> bool {
    match agent {
        "agy" => home.join(".gemini").is_dir(),
        "codex" => home.join(".codex").is_dir(),
        "claude" => home.join(".claude").is_dir(),
        "cursor" => home.join(".cursor").is_dir(),
        "copilot" => home.join(".copilot").is_dir(),
        "pi" => home.join(".pi").is_dir(),
        _ => false,
    }
}

fn compute_agent_taxes(
    home: &Path,
    rules: &[RuleItem],
    skills: &[SkillItem],
    probed: &[(McpServerConfig, Option<McpProbeResult>)],
    filter: Option<&str>,
) -> Vec<AgentTax> {
    const AGENTS: &[&str] = &["agy", "codex", "claude", "cursor", "copilot", "pi"];
    let mut rows = Vec::new();

    for &agent in AGENTS {
        if let Some(f) = filter
            && f != agent
        {
            continue;
        }
        let has_own = rules.iter().any(|r| r.agent == agent)
            || skills.iter().any(|s| s.agent == agent)
            || probed.iter().any(|(srv, _)| srv.agent == agent)
            || agent_dir_exists(home, agent);
        if !has_own && filter != Some(agent) {
            continue;
        }

        let rules_tok = rules
            .iter()
            .filter(|r| r.agent == agent || r.agent == "repo")
            .map(|r| r.tokens)
            .sum();
        let mut seen_skill_names = BTreeSet::new();
        let skills_tok = skills
            .iter()
            .filter(|s| {
                s.agent == agent
                    || s.agent == "repo"
                    || (s.agent == "agents" && matches!(agent, "agy" | "codex"))
            })
            .filter(|s| seen_skill_names.insert(s.name.to_lowercase()))
            .map(|s| s.tokens)
            .sum();
        let mcp_tok = probed
            .iter()
            .filter(|(srv, _)| srv.agent == agent || srv.agent == "repo")
            .filter_map(|(_, p)| p.as_ref().and_then(|res| res.schema_tokens))
            .sum();
        rows.push(AgentTax {
            agent,
            rules_tok,
            skills_tok,
            mcp_tok,
        });
    }

    if rows.is_empty() && filter.is_none_or(|f| f == "repo") {
        let rules_tok: usize = rules
            .iter()
            .filter(|r| r.agent == "repo")
            .map(|r| r.tokens)
            .sum();
        let skills_tok: usize = skills
            .iter()
            .filter(|s| s.agent == "repo")
            .map(|s| s.tokens)
            .sum();
        let mcp_tok: usize = probed
            .iter()
            .filter(|(srv, _)| srv.agent == "repo")
            .filter_map(|(_, p)| p.as_ref().and_then(|res| res.schema_tokens))
            .sum();
        if rules_tok + skills_tok + mcp_tok > 0 {
            rows.push(AgentTax {
                agent: "repo",
                rules_tok,
                skills_tok,
                mcp_tok,
            });
        }
    }

    rows.sort_by_key(|row| std::cmp::Reverse(row.total()));
    rows
}

#[allow(clippy::too_many_arguments)]
fn print_json(
    total_tokens: usize,
    rules_tokens: usize,
    skills_tokens: usize,
    mcp_tokens: usize,
    agent_taxes: &[AgentTax],
    rules: &[RuleItem],
    skills: &[SkillItem],
    disabled_plugins: &[String],
    probed: &[(McpServerConfig, Option<McpProbeResult>)],
    top_tools: &[(String, usize)],
    show_all: bool,
) {
    println!("{{");
    println!("  \"total_tokens_est\": {total_tokens},");
    println!("  \"rules_tokens_est\": {rules_tokens},");
    println!("  \"skills_tokens_est\": {skills_tokens},");
    println!("  \"mcp_tokens_est\": {mcp_tokens},");
    println!("  \"by_agent\": [");
    for (i, a) in agent_taxes.iter().enumerate() {
        let comma = if i + 1 < agent_taxes.len() { "," } else { "" };
        println!(
            "    {{\"agent\": {}, \"rules_tokens_est\": {}, \"skills_tokens_est\": {}, \"mcp_tokens_est\": {}, \"total_tokens_est\": {}}}{comma}",
            json_string(a.agent),
            a.rules_tok,
            a.skills_tok,
            a.mcp_tok,
            a.total(),
        );
    }
    println!("  ],");
    println!("  \"rules\": [");
    for (i, r) in rules.iter().enumerate() {
        let comma = if i + 1 < rules.len() { "," } else { "" };
        println!(
            "    {{\"agent\": {}, \"path\": {}, \"bytes\": {}, \"tokens_est\": {}, \"has_trm\": {}}}{comma}",
            json_string(&r.agent),
            json_string(&r.path),
            r.bytes,
            r.tokens,
            r.has_trm,
        );
    }
    println!("  ],");
    let disabled_json: Vec<String> = disabled_plugins.iter().map(|s| json_string(s)).collect();
    println!("  \"disabled_plugins\": [{}],", disabled_json.join(", "));
    println!("  \"skills_count\": {},", skills.len());
    let skill_limit = if show_all {
        skills.len()
    } else {
        skills.len().min(20)
    };
    println!("  \"skills\": [");
    for (i, s) in skills.iter().take(skill_limit).enumerate() {
        let comma = if i + 1 < skill_limit { "," } else { "" };
        println!(
            "    {{\"agent\": {}, \"source\": {}, \"name\": {}, \"bytes\": {}, \"tokens_est\": {}}}{comma}",
            json_string(&s.agent),
            json_string(&s.source),
            json_string(&s.name),
            s.bytes,
            s.tokens,
        );
    }
    println!("  ],");
    println!("  \"mcp_servers\": [");
    for (i, (srv, probe)) in probed.iter().enumerate() {
        let comma = if i + 1 < probed.len() { "," } else { "" };
        let tools_json = probe
            .as_ref()
            .and_then(|p| p.tools_count)
            .map_or("null".into(), |n| n.to_string());
        let tok_json = probe
            .as_ref()
            .and_then(|p| p.schema_tokens)
            .map_or("null".into(), |n| n.to_string());
        let status_json = probe
            .as_ref()
            .map_or("null".into(), |p| json_string(&p.status));
        println!(
            "    {{\"agent\": {}, \"name\": {}, \"source\": {}, \"tools\": {tools_json}, \"schema_tokens_est\": {tok_json}, \"status\": {status_json}}}{comma}",
            json_string(&srv.agent),
            json_string(&srv.name),
            json_string(&srv.source),
        );
    }
    println!("  ],");
    let tool_limit = if show_all {
        top_tools.len()
    } else {
        top_tools.len().min(15)
    };
    println!("  \"top_mcp_tools\": [");
    for (i, (name, tok)) in top_tools.iter().take(tool_limit).enumerate() {
        let comma = if i + 1 < tool_limit { "," } else { "" };
        println!(
            "    {{\"tool\": {}, \"tokens_est\": {tok}}}{comma}",
            json_string(name)
        );
    }
    println!("  ]");
    println!("}}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_skill_frontmatter_and_mcp_configs() {
        let skill = "---\nname: demo-skill\ndescription: >-\n  Multi line description\n  for testing.\n---\n# Body\n";
        let (name, desc, invocable) = parse_skill_frontmatter(skill, "fallback");
        assert_eq!(name, "demo-skill");
        assert_eq!(desc, "Multi line description for testing.");
        assert!(invocable);

        let manual =
            "---\nname: slash-only\ndisable-model-invocation: true\ndescription: hidden\n---\n";
        assert!(!parse_skill_frontmatter(manual, "x").2);

        let mut servers = Vec::new();
        parse_mcp_json(
            "claude",
            "~/.claude.json",
            r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","server"]},"off":{"command":"x","disabled":true}}}"#,
            &mut servers,
        );
        parse_codex_mcp_toml(
            "~/.codex/config.toml",
            "[mcp_servers.gh]\ncommand = \"gh-mcp\"\nargs = [\"stdio\"]\n",
            &mut servers,
        );
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "fs");
        assert_eq!(servers[1].name, "gh");
    }

    #[test]
    fn probes_mock_stdio_mcp_server() {
        let script = r#"read line; read line; read line; echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"alpha","description":"Runs alpha","inputSchema":{"type":"object"}}]}}'"#;
        let res = probe_mcp_server(&McpTransport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: Vec::new(),
        });
        assert!(res.status.starts_with("ok "), "status={}", res.status);
        assert_eq!(res.tools_count, Some(1));
        assert!(res.schema_tokens.unwrap_or(0) > 0);
        assert_eq!(res.tools[0].0, "alpha");
    }

    #[test]
    fn filters_disabled_plugins_with_config_overrides() {
        let dir = std::env::temp_dir().join(format!("trm_plugin_filter_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cfg_dir = dir.join(".gemini/config");
        let plugins = cfg_dir.join("plugins");
        fs::create_dir_all(plugins.join("off_default/skills/hidden")).unwrap();
        fs::create_dir_all(plugins.join("on_override/skills/shown")).unwrap();
        fs::create_dir_all(plugins.join("off_override/skills/muted")).unwrap();

        fs::write(
            plugins.join("off_default/plugin.json"),
            r#"{"disabled":true}"#,
        )
        .unwrap();
        fs::write(
            plugins.join("on_override/plugin.json"),
            r#"{"disabled":true}"#,
        )
        .unwrap();
        fs::write(
            plugins.join("off_override/plugin.json"),
            r#"{"name":"off_override"}"#,
        )
        .unwrap();
        fs::write(
            cfg_dir.join("config.json"),
            r#"{"plugins":{"on_override":{"enabled":true},"off_override":{"enabled":false}}}"#,
        )
        .unwrap();

        let skill_md = "---\nname: s\ndescription: d\n---\n";
        fs::write(plugins.join("off_default/skills/hidden/SKILL.md"), skill_md).unwrap();
        fs::write(plugins.join("on_override/skills/shown/SKILL.md"), skill_md).unwrap();
        fs::write(plugins.join("off_override/skills/muted/SKILL.md"), skill_md).unwrap();

        let scan = scan_skills(&dir, &[], &|a| a == "agy");
        assert_eq!(scan.skills.len(), 1);
        assert_eq!(scan.skills[0].source, "plugin:on_override");
        assert_eq!(
            scan.disabled_plugins,
            vec!["off_default".to_string(), "off_override".to_string()]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_manual_excluded_and_duplicate_named_skills() {
        let dir = std::env::temp_dir().join(format!("trm_skill_truth_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cfg_dir = dir.join(".gemini/config");
        let p1 = cfg_dir.join("plugins/p1/skills");
        let p2 = cfg_dir.join("plugins/p2/skills");
        fs::create_dir_all(p1.join("uv")).unwrap();
        fs::create_dir_all(p2.join("uv")).unwrap();
        fs::create_dir_all(p1.join("manual")).unwrap();
        fs::create_dir_all(p1.join("exp-beta")).unwrap();

        fs::write(cfg_dir.join("skills.json"), r#"{"exclude":["exp-*"]}"#).unwrap();
        fs::write(
            p1.join("uv/SKILL.md"),
            "---\nname: uv\ndescription: dup1\n---\n",
        )
        .unwrap();
        fs::write(
            p2.join("uv/SKILL.md"),
            "---\nname: uv\ndescription: dup2\n---\n",
        )
        .unwrap();
        fs::write(
            p1.join("manual/SKILL.md"),
            "---\nname: manual\ndisable-model-invocation: true\ndescription: hidden\n---\n",
        )
        .unwrap();
        fs::write(
            p1.join("exp-beta/SKILL.md"),
            "---\nname: exp-beta\ndescription: skip\n---\n",
        )
        .unwrap();

        let scan = scan_skills(&dir, &[], &|a| a == "agy");
        assert_eq!(
            scan.skills.len(),
            1,
            "expected only 1 deduplicated uv skill"
        );
        assert_eq!(scan.skills[0].name, "uv");
        assert_eq!(scan.skipped_manual, 1);
        assert_eq!(scan.skipped_excluded, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn attributes_turn_zero_tax_by_agent() {
        let rules = vec![
            RuleItem {
                agent: "agy".into(),
                path: "GEMINI.md".into(),
                bytes: 400,
                tokens: 100,
                has_trm: true,
            },
            RuleItem {
                agent: "codex".into(),
                path: "AGENTS.md".into(),
                bytes: 400,
                tokens: 100,
                has_trm: true,
            },
            RuleItem {
                agent: "claude".into(),
                path: "CLAUDE.md".into(),
                bytes: 400,
                tokens: 100,
                has_trm: true,
            },
        ];
        let skills = vec![
            SkillItem {
                agent: "agents".into(),
                source: "~/.agents".into(),
                name: "shared".into(),
                bytes: 800,
                tokens: 200,
            },
            SkillItem {
                agent: "agy".into(),
                source: "plugin:demo".into(),
                name: "agy-only".into(),
                bytes: 2000,
                tokens: 500,
            },
            SkillItem {
                agent: "claude".into(),
                source: "~/.claude".into(),
                name: "claude-only".into(),
                bytes: 80,
                tokens: 20,
            },
        ];
        let taxes = compute_agent_taxes(Path::new("/nonexistent"), &rules, &skills, &[], None);
        assert_eq!(taxes.len(), 3);
        assert_eq!(taxes[0].agent, "agy");
        assert_eq!(taxes[0].skills_tok, 700);
        assert_eq!(taxes[1].agent, "codex");
        assert_eq!(taxes[1].skills_tok, 200);
        assert_eq!(taxes[2].agent, "claude");
        assert_eq!(taxes[2].skills_tok, 20);
    }
}
