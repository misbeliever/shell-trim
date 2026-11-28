use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::cli::AuditOptions;
use crate::style::{self, Cell, Theme};

#[derive(Debug, Default, Clone)]
struct AuditBucket {
    trm_read: usize,
    cat: usize,
    head: usize,
    tail: usize,
    sed_read: usize,
    trm_rg: usize,
    rg: usize,
    grep: usize,
    trm_build: usize,
    swift: usize,
    cargo: usize,
    trm_git: usize,
    git: usize,
    trm_fs: usize,
    ls: usize,
    find: usize,
}

impl AuditBucket {
    fn read_bypasses(&self) -> usize {
        self.cat + self.head + self.tail + self.sed_read
    }

    fn search_bypasses(&self) -> usize {
        self.rg + self.grep
    }

    fn build_bypasses(&self) -> usize {
        self.swift + self.cargo
    }

    fn fs_bypasses(&self) -> usize {
        self.ls + self.find
    }

    fn adopted(&self) -> usize {
        self.trm_read + self.trm_rg + self.trm_build + self.trm_git + self.trm_fs
    }

    fn bypasses(&self) -> usize {
        self.read_bypasses()
            + self.search_bypasses()
            + self.build_bypasses()
            + self.git
            + self.fs_bypasses()
    }

    fn total(&self) -> usize {
        self.adopted() + self.bypasses()
    }

    fn adoption_pct(&self) -> f64 {
        let t = self.total();
        if t == 0 {
            0.0
        } else {
            (self.adopted() as f64 / t as f64) * 100.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadCmdKind {
    TrmRead,
    Cat,
    Head,
    Tail,
    SedRead,
    TrmRg,
    Rg,
    Grep,
    TrmBuild,
    Swift,
    Cargo,
    TrmGit,
    Git,
    TrmFs,
    Ls,
    Find,
}

#[derive(Debug, Clone)]
pub struct BypassSample {
    pub ts: Option<u64>,
    pub agent: String,
    pub model: String,
    pub kind: &'static str,
    pub cmd: String,
}

#[derive(Debug, Default, Clone)]
pub struct AuditSummary {
    pub files_scanned: usize,
    pub adopted: usize,
    pub bypasses: usize,
    pub adoption_pct: f64,
    pub by_agent: Vec<(String, usize, usize, f64)>, // name, adopted, total, pct
    pub by_model: Vec<(String, usize, usize, f64)>, // name, adopted, total, pct
    pub bypass_samples: Vec<BypassSample>,
    pub recent_bypasses: Vec<String>,
}

type ScanSessionsResult = (
    BTreeMap<String, AuditBucket>,
    BTreeMap<String, AuditBucket>,
    Vec<BypassSample>,
    usize,
    u64,
);

fn scan_sessions(since_secs: Option<u64>, agent_filter: Option<&str>) -> ScanSessionsResult {
    let home_path = crate::config::Config::home_dir();
    let now = crate::telemetry::now_secs();
    let cutoff = since_secs.map(|d| now.saturating_sub(d));

    let mut by_agent: BTreeMap<String, AuditBucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, AuditBucket> = BTreeMap::new();
    let mut samples: Vec<BypassSample> = Vec::new();
    let mut files_scanned = 0usize;
    let mut hook_events = crate::telemetry::load_session_cmd_timestamps(since_secs);

    let want_agent = |name: &str| -> bool { agent_filter.is_none_or(|f| f == name) };

    if let Ok(custom) = env::var("TRM_AUDIT_DIR") {
        if let Some((agent, dir)) = custom.split_once(':') {
            let mut files = Vec::new();
            collect_jsonl_files(Path::new(dir), cutoff, &mut files);
            for f in files {
                files_scanned += 1;
                scan_file(
                    agent,
                    &f,
                    cutoff,
                    &mut hook_events,
                    &mut by_agent,
                    &mut by_model,
                    &mut samples,
                );
            }
        }
    } else {
        let dirs: &[(&str, PathBuf)] = &[
            ("codex", home_path.join(".codex").join("sessions")),
            ("claude", home_path.join(".claude").join("projects")),
            ("copilot", home_path.join(".copilot").join("session-state")),
            ("pi", home_path.join(".pi").join("agent").join("sessions")),
        ];
        for (agent, dir) in dirs {
            if want_agent(agent) && dir.exists() {
                let mut files = Vec::new();
                collect_jsonl_files(dir, cutoff, &mut files);
                for f in files {
                    files_scanned += 1;
                    scan_file(
                        agent,
                        &f,
                        cutoff,
                        &mut hook_events,
                        &mut by_agent,
                        &mut by_model,
                        &mut samples,
                    );
                }
            }
        }
        if want_agent("agy") {
            let brain_dir = home_path
                .join(".gemini")
                .join("antigravity-cli")
                .join("brain");
            if let Ok(entries) = fs::read_dir(&brain_dir) {
                for entry in entries.flatten() {
                    let transcript = entry
                        .path()
                        .join(".system_generated")
                        .join("logs")
                        .join("transcript.jsonl");
                    if transcript.exists() && file_passes_cutoff(&transcript, cutoff) {
                        files_scanned += 1;
                        scan_file(
                            "agy",
                            &transcript,
                            cutoff,
                            &mut hook_events,
                            &mut by_agent,
                            &mut by_model,
                            &mut samples,
                        );
                    }
                }
            }
        }
    }

    (by_agent, by_model, samples, files_scanned, now)
}

pub fn load_summary(since_secs: Option<u64>) -> AuditSummary {
    load_summary_inner(since_secs, Some(200))
}

pub fn load_summary_full(since_secs: Option<u64>) -> AuditSummary {
    load_summary_inner(since_secs, None)
}

fn load_summary_inner(since_secs: Option<u64>, sample_limit: Option<usize>) -> AuditSummary {
    let (by_agent, by_model, mut samples, files_scanned, now) = scan_sessions(since_secs, None);

    let mut adopted = 0usize;
    let mut bypasses = 0usize;
    for b in by_agent.values() {
        adopted += b.adopted();
        bypasses += b.bypasses();
    }
    let total = adopted + bypasses;
    let adoption_pct = if total == 0 {
        0.0
    } else {
        (adopted as f64 / total as f64) * 100.0
    };

    let mut agents: Vec<(String, usize, usize, f64)> = by_agent
        .into_iter()
        .filter(|(_, b)| b.total() > 0)
        .map(|(k, b)| (k, b.adopted(), b.total(), b.adoption_pct()))
        .collect();
    agents.sort_by_key(|a| std::cmp::Reverse(a.2));

    let mut models: Vec<(String, usize, usize, f64)> = by_model
        .into_iter()
        .filter(|(_, b)| b.total() > 0)
        .map(|(k, b)| (k, b.adopted(), b.total(), b.adoption_pct()))
        .collect();
    models.sort_by_key(|a| std::cmp::Reverse(a.2));

    samples.sort_by_key(|s| s.ts.unwrap_or(0));
    if let Some(limit) = sample_limit
        && samples.len() > limit
    {
        samples.drain(..samples.len() - limit);
    }
    let recent_bypasses: Vec<String> = samples
        .iter()
        .rev()
        .take(100)
        .map(|s| {
            let age =
                s.ts.map(|t| format!("{:>7} ", crate::telemetry::fmt_age(t, now)))
                    .unwrap_or_default();
            format!("{age}[{}/{}] ({}) {}", s.agent, s.model, s.kind, s.cmd)
        })
        .collect();

    AuditSummary {
        files_scanned,
        adopted,
        bypasses,
        adoption_pct,
        by_agent: agents,
        by_model: models,
        bypass_samples: samples,
        recent_bypasses,
    }
}

pub fn run_audit(opts: &AuditOptions) -> Result<(), String> {
    if opts.context {
        return crate::context_audit::run(opts);
    }
    let (by_agent, by_model, mut samples, files_scanned, now) =
        scan_sessions(opts.since_secs, opts.agent_filter.as_deref());

    let mut total = AuditBucket::default();
    for b in by_agent.values() {
        total.trm_read += b.trm_read;
        total.cat += b.cat;
        total.head += b.head;
        total.tail += b.tail;
        total.sed_read += b.sed_read;
        total.trm_rg += b.trm_rg;
        total.rg += b.rg;
        total.grep += b.grep;
        total.trm_build += b.trm_build;
        total.swift += b.swift;
        total.cargo += b.cargo;
        total.trm_git += b.trm_git;
        total.git += b.git;
        total.trm_fs += b.trm_fs;
        total.ls += b.ls;
        total.find += b.find;
    }

    if opts.json {
        println!(
            "{{\"files_scanned\":{},\"trm_read\":{},\"cat\":{},\"head\":{},\"tail\":{},\"sed_read\":{},\"trm_rg\":{},\"rg\":{},\"grep\":{},\"trm_build\":{},\"swift\":{},\"cargo\":{},\"trm_git\":{},\"git\":{},\"trm_fs\":{},\"ls\":{},\"find\":{},\"adopted\":{},\"bypasses\":{},\"total\":{},\"adoption_pct\":{:.1}}}",
            files_scanned,
            total.trm_read,
            total.cat,
            total.head,
            total.tail,
            total.sed_read,
            total.trm_rg,
            total.rg,
            total.grep,
            total.trm_build,
            total.swift,
            total.cargo,
            total.trm_git,
            total.git,
            total.trm_fs,
            total.ls,
            total.find,
            total.adopted(),
            total.bypasses(),
            total.total(),
            total.adoption_pct()
        );
        return Ok(());
    }

    let theme = Theme::detect();
    let adopt_pct = total.adoption_pct();
    println!(
        "{}",
        theme.header_rule(
            "trm audit",
            &format!(
                "{} session logs scanned, {} tracked shell commands",
                files_scanned,
                total.total()
            )
        )
    );
    println!(
        "  {:<21} {:>6}  {}  {}  {}",
        theme.bold("Adopted (trm):"),
        theme.bold(&total.adopted().to_string()),
        theme.pct_color(adopt_pct, &format!("{adopt_pct:>5.1}%")),
        theme.bar(adopt_pct, 12),
        theme.dim(&format!(
            "[read: {}, rg: {}, fs: {}, git: {}, build: {}]",
            total.trm_read, total.trm_rg, total.trm_fs, total.trm_git, total.trm_build
        ))
    );
    println!(
        "  {:<21} {:>6}  {}",
        theme.dim("Bypassed (read):"),
        total.read_bypasses(),
        theme.dim(&format!(
            "(cat: {}, head: {}, tail: {}, sed -n: {})",
            total.cat, total.head, total.tail, total.sed_read
        ))
    );
    println!(
        "  {:<21} {:>6}  {}",
        theme.dim("Bypassed (search/fs):"),
        total.search_bypasses() + total.fs_bypasses(),
        theme.dim(&format!(
            "(rg: {}, grep: {}, ls: {}, find: {})",
            total.rg, total.grep, total.ls, total.find
        ))
    );
    println!(
        "  {:<21} {:>6}  {}",
        theme.dim("Bypassed (git/build):"),
        total.git + total.build_bypasses(),
        theme.dim(&format!(
            "(git: {}, swift/swiftc: {}, cargo: {})",
            total.git, total.swift, total.cargo
        ))
    );

    if !by_agent.is_empty() {
        print_audit_table(&theme, "By agent", "agent", &by_agent);
    }

    let has_known_model = by_model
        .keys()
        .any(|k| !k.ends_with("/unknown") && k != "unknown");
    if opts.by_model || (has_known_model && by_model.len() > 1) {
        print_audit_table(&theme, "By agent/model", "agent/model", &by_model);
    }

    if opts.bypasses {
        println!("\nRecent bypassed commands:");
        if samples.is_empty() {
            println!("  None found.");
        } else {
            samples.sort_by_key(|s| s.ts.unwrap_or(0));
            for s in samples.iter().rev().take(20) {
                let preview = if s.cmd.len() > 96 {
                    let mut end = 93;
                    while end > 0 && !s.cmd.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &s.cmd[..end])
                } else {
                    s.cmd.clone()
                };
                let age =
                    s.ts.map(|t| format!("{:>7} ", crate::telemetry::fmt_age(t, now)))
                        .unwrap_or_default();
                println!(
                    "  - {age}[{}/{}] ({}) {}",
                    s.agent, s.model, s.kind, preview
                );
            }
        }
    }

    Ok(())
}

fn print_audit_table(
    theme: &Theme,
    title: &str,
    col_name: &str,
    map: &BTreeMap<String, AuditBucket>,
) {
    println!("\n{}", theme.bold(&format!("{title}:")));
    let mut rows_sorted: Vec<(&String, &AuditBucket)> =
        map.iter().filter(|(_, b)| b.total() > 0).collect();
    rows_sorted.sort_by_key(|a| std::cmp::Reverse(a.1.total()));
    if rows_sorted.is_empty() {
        println!("  (no tracked commands)");
        return;
    }

    let headers: &[(&str, bool)] = &[
        (col_name, false),
        ("t.rd", true),
        ("cat", true),
        ("hd", true),
        ("tl", true),
        ("sed", true),
        ("t.rg", true),
        ("rg", true),
        ("grp", true),
        ("t.fs", true),
        ("ls", true),
        ("fnd", true),
        ("t.gt", true),
        ("git", true),
        ("t.bd", true),
        ("swft", true),
        ("crgo", true),
        ("adopt%", false),
    ];

    let mut table_rows: Vec<Vec<Cell>> = Vec::with_capacity(rows_sorted.len());
    for (k, b) in rows_sorted {
        let p = b.adoption_pct();
        let p_plain = format!("{p:>5.1}%");
        let adopt_rendered = format!("{} {}", theme.pct_color(p, &p_plain), theme.bar(p, 8));
        let adopt_cell = Cell::styled(adopt_rendered, p_plain.chars().count() + 9, false);

        table_rows.push(vec![
            Cell::left(k.clone()),
            Cell::right(b.trm_read.to_string()),
            Cell::right(b.cat.to_string()),
            Cell::right(b.head.to_string()),
            Cell::right(b.tail.to_string()),
            Cell::right(b.sed_read.to_string()),
            Cell::right(b.trm_rg.to_string()),
            Cell::right(b.rg.to_string()),
            Cell::right(b.grep.to_string()),
            Cell::right(b.trm_fs.to_string()),
            Cell::right(b.ls.to_string()),
            Cell::right(b.find.to_string()),
            Cell::right(b.trm_git.to_string()),
            Cell::right(b.git.to_string()),
            Cell::right(b.trm_build.to_string()),
            Cell::right(b.swift.to_string()),
            Cell::right(b.cargo.to_string()),
            adopt_cell,
        ]);
    }

    for line in style::render_table(theme, headers, &table_rows) {
        println!("{line}");
    }
}

fn file_passes_cutoff(path: &Path, cutoff: Option<u64>) -> bool {
    let Some(c) = cutoff else {
        return true;
    };
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return true;
    };
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs >= c
}

fn collect_jsonl_files(dir: &Path, cutoff: Option<u64>, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_jsonl_files(&p, cutoff, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && file_passes_cutoff(&p, cutoff)
        {
            out.push(p);
        }
    }
}

fn session_id_for_transcript(agent: &str, path: &Path) -> Option<String> {
    match agent {
        "agy" | "antigravity" => path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned()),
        "claude" => path.file_stem().map(|s| s.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn shell_words(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            } else if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn event_matches_sub_cmd(expected_cmd: &str, ev_path: &str, sub_cmd: &str) -> bool {
    let words = shell_words(sub_cmd);
    let mut idx = 0;
    while idx < words.len() {
        let w = words[idx].as_str();
        if (w.contains('=') && !w.starts_with('-')) || matches!(w, "env" | "command" | "time") {
            idx += 1;
        } else {
            break;
        }
    }
    if words.get(idx).map(|s| s.rsplit('/').next().unwrap_or(s)) == Some("trm") {
        idx += 1;
    }
    let args = words.get(idx + 1..).unwrap_or(&[]);
    match expected_cmd {
        "rg" => {
            if !ev_path.starts_with("rg:") {
                return true;
            }
            // Filter trm-only flags so `extract_query_key` matches what `search.rs` logged.
            let rg_args: Vec<String> = args
                .iter()
                .filter(|a| {
                    !matches!(
                        a.as_str(),
                        "--raw" | "--all" | "--block" | "--no-block" | "--no-stats"
                    )
                })
                .cloned()
                .collect();
            crate::search::extract_query_key(&rg_args) == ev_path
        }
        "git" | "cargo" | "swift" => ev_path
            .split_once(':')
            .is_none_or(|(_, sub)| sub.is_empty() || args.iter().any(|a| a == sub)),
        _ => true,
    }
}

fn consume_hook_event(
    hook_events: &mut std::collections::HashMap<(String, String), Vec<(u64, String)>>,
    sid: &str,
    expected_cmd: &str,
    sub_cmd: &str,
    call_ts: u64,
) -> bool {
    let key = (sid.to_string(), expected_cmd.to_string());
    if let Some(entries) = hook_events.get_mut(&key) {
        // Telemetry is logged after execution completes (`ev_ts >= call_ts`), so never
        // match backwards into an earlier command's event; also verify query/subcmd identity.
        let hi = call_ts.saturating_add(60);
        if let Some(pos) = entries.iter().position(|(t, path)| {
            *t >= call_ts && *t <= hi && event_matches_sub_cmd(expected_cmd, path, sub_cmd)
        }) {
            entries.remove(pos);
            return true;
        }
    }
    false
}

fn explicit_trm_telemetry_cmd(kind: ReadCmdKind, sub_cmd: &str) -> Option<&'static str> {
    match kind {
        ReadCmdKind::TrmRead => Some("read"),
        ReadCmdKind::TrmRg => Some("rg"),
        ReadCmdKind::TrmFs => Some("fs"),
        ReadCmdKind::TrmGit => Some("git"),
        ReadCmdKind::TrmBuild => {
            fn base(t: &str) -> &str {
                t.rsplit('/').next().unwrap_or(t)
            }
            let mut toks = sub_cmd.split_whitespace();
            let first = toks.next().map(base);
            let exec = if first == Some("trm") {
                toks.next().map(base)
            } else {
                first
            };
            match exec {
                Some("swift" | "swiftc") => Some("swift"),
                _ => Some("cargo"),
            }
        }
        _ => None,
    }
}

fn promote_if_hook_rewritten(
    agent: &str,
    kind: ReadCmdKind,
    raw_cmd: &str,
    sub_cmd: &str,
    session_id: Option<&str>,
    ts: Option<u64>,
    hook_events: &mut std::collections::HashMap<(String, String), Vec<(u64, String)>>,
) -> ReadCmdKind {
    if !matches!(agent, "agy" | "antigravity" | "claude") {
        return kind;
    }
    let (expected_cmd, promoted) = match kind {
        ReadCmdKind::Cat | ReadCmdKind::Head | ReadCmdKind::Tail | ReadCmdKind::SedRead => {
            ("read", ReadCmdKind::TrmRead)
        }
        ReadCmdKind::Rg | ReadCmdKind::Grep => ("rg", ReadCmdKind::TrmRg),
        ReadCmdKind::Ls | ReadCmdKind::Find => ("fs", ReadCmdKind::TrmFs),
        ReadCmdKind::Git => ("git", ReadCmdKind::TrmGit),
        ReadCmdKind::Swift => ("swift", ReadCmdKind::TrmBuild),
        ReadCmdKind::Cargo => ("cargo", ReadCmdKind::TrmBuild),
        _ => return kind,
    };
    let (Some(sid), Some(call_ts)) = (session_id, ts) else {
        return kind;
    };
    // Both the full command AND this individual segment must be rewritten by the hook.
    if crate::hook::rewrite_shell_command(raw_cmd).is_none()
        || crate::hook::rewrite_shell_command(sub_cmd).is_none()
    {
        return kind;
    }
    if consume_hook_event(hook_events, sid, expected_cmd, sub_cmd, call_ts) {
        return promoted;
    }
    kind
}

fn scan_file(
    agent: &str,
    path: &Path,
    cutoff: Option<u64>,
    hook_events: &mut std::collections::HashMap<(String, String), Vec<(u64, String)>>,
    by_agent: &mut BTreeMap<String, AuditBucket>,
    by_model: &mut BTreeMap<String, AuditBucket>,
    samples: &mut Vec<BypassSample>,
) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    let session_id = session_id_for_transcript(agent, path);
    let mut current_model = "unknown".to_string();
    let mut current_ts: Option<u64> = None;
    let mut parsed_cmds: Vec<(Option<u64>, String, String)> = Vec::new();

    for line in content.lines() {
        if let Some(m) = extract_json_string_field(line, "\"model\"")
            && !m.is_empty()
        {
            current_model = m;
        }
        if let Some(ts_str) = extract_json_string_field(line, "\"timestamp\"")
            .or_else(|| extract_json_string_field(line, "\"created_at\""))
            && let Some(secs) = parse_iso8601_utc(&ts_str)
        {
            current_ts = Some(secs);
        }

        if let (Some(c), Some(ts)) = (cutoff, current_ts)
            && ts < c
        {
            continue;
        }

        let cmds = match agent {
            "agy" | "antigravity" => extract_antigravity_cmds(line),
            "codex" => extract_codex_cmds(line),
            "claude" => extract_claude_cmds(line),
            "copilot" => extract_copilot_cmds(line),
            "pi" => extract_pi_cmds(line),
            _ => Vec::new(),
        };

        for raw_cmd in cmds {
            parsed_cmds.push((current_ts, current_model.clone(), raw_cmd));
        }
    }

    // Pass 1: consume telemetry events belonging to explicit `trm *` calls first
    // so an explicit `trm rg` can never mask a nearby raw `rg` bypass.
    if let Some(sid) = session_id.as_deref()
        && matches!(agent, "agy" | "antigravity" | "claude")
    {
        for (ts, _, raw_cmd) in &parsed_cmds {
            let Some(call_ts) = *ts else {
                continue;
            };
            for (kind, sub_cmd) in classify_shell_command(raw_cmd) {
                if let Some(trm_cmd) = explicit_trm_telemetry_cmd(kind, &sub_cmd) {
                    consume_hook_event(hook_events, sid, trm_cmd, &sub_cmd, call_ts);
                }
            }
        }
    }

    // Pass 2: classify commands and promote only hook-rewritten segments using remaining events.
    for (current_ts, current_model, raw_cmd) in parsed_cmds {
        for (raw_kind, sub_cmd) in classify_shell_command(&raw_cmd) {
            let kind = promote_if_hook_rewritten(
                agent,
                raw_kind,
                &raw_cmd,
                &sub_cmd,
                session_id.as_deref(),
                current_ts,
                hook_events,
            );
            let ab = by_agent.entry(agent.to_string()).or_default();
            let mb = by_model
                .entry(format!("{agent}/{current_model}"))
                .or_default();
            match kind {
                ReadCmdKind::TrmRead => {
                    ab.trm_read += 1;
                    mb.trm_read += 1;
                }
                ReadCmdKind::Cat => {
                    ab.cat += 1;
                    mb.cat += 1;
                    record_sample(samples, current_ts, agent, &current_model, "cat", sub_cmd);
                }
                ReadCmdKind::Head => {
                    ab.head += 1;
                    mb.head += 1;
                    record_sample(samples, current_ts, agent, &current_model, "head", sub_cmd);
                }
                ReadCmdKind::Tail => {
                    ab.tail += 1;
                    mb.tail += 1;
                    record_sample(samples, current_ts, agent, &current_model, "tail", sub_cmd);
                }
                ReadCmdKind::SedRead => {
                    ab.sed_read += 1;
                    mb.sed_read += 1;
                    record_sample(
                        samples,
                        current_ts,
                        agent,
                        &current_model,
                        "sed -n",
                        sub_cmd,
                    );
                }
                ReadCmdKind::TrmRg => {
                    ab.trm_rg += 1;
                    mb.trm_rg += 1;
                }
                ReadCmdKind::Rg => {
                    ab.rg += 1;
                    mb.rg += 1;
                    record_sample(samples, current_ts, agent, &current_model, "rg", sub_cmd);
                }
                ReadCmdKind::Grep => {
                    ab.grep += 1;
                    mb.grep += 1;
                    record_sample(samples, current_ts, agent, &current_model, "grep", sub_cmd);
                }
                ReadCmdKind::TrmBuild => {
                    ab.trm_build += 1;
                    mb.trm_build += 1;
                }
                ReadCmdKind::Swift => {
                    ab.swift += 1;
                    mb.swift += 1;
                    record_sample(samples, current_ts, agent, &current_model, "swift", sub_cmd);
                }
                ReadCmdKind::Cargo => {
                    ab.cargo += 1;
                    mb.cargo += 1;
                    record_sample(samples, current_ts, agent, &current_model, "cargo", sub_cmd);
                }
                ReadCmdKind::TrmGit => {
                    ab.trm_git += 1;
                    mb.trm_git += 1;
                }
                ReadCmdKind::Git => {
                    ab.git += 1;
                    mb.git += 1;
                    record_sample(samples, current_ts, agent, &current_model, "git", sub_cmd);
                }
                ReadCmdKind::TrmFs => {
                    ab.trm_fs += 1;
                    mb.trm_fs += 1;
                }
                ReadCmdKind::Ls => {
                    ab.ls += 1;
                    mb.ls += 1;
                    record_sample(samples, current_ts, agent, &current_model, "ls", sub_cmd);
                }
                ReadCmdKind::Find => {
                    ab.find += 1;
                    mb.find += 1;
                    record_sample(samples, current_ts, agent, &current_model, "find", sub_cmd);
                }
            }
        }
    }
}

fn record_sample(
    samples: &mut Vec<BypassSample>,
    ts: Option<u64>,
    agent: &str,
    model: &str,
    kind: &'static str,
    cmd: String,
) {
    samples.push(BypassSample {
        ts,
        agent: agent.to_string(),
        model: model.to_string(),
        kind,
        cmd,
    });
}

fn extract_antigravity_cmds(line: &str) -> Vec<String> {
    if !line.contains("\"run_command\"") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(idx) = line[pos..].find("\"CommandLine\"") {
        let abs = pos + idx + "\"CommandLine\"".len();
        if let Some((val, next_pos)) = parse_json_value_after_colon(line, abs) {
            // Antigravity sometimes double-quotes the string inside CommandLine
            let unquoted = val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(&val)
                .to_string();
            out.push(unquoted);
            pos = next_pos;
        } else {
            break;
        }
    }
    out
}

fn extract_codex_cmds(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    // 1. custom_tool_call with JS exec wrapper: tools.exec_command({cmd:"..."})
    if line.contains("\"custom_tool_call\"") && line.contains("exec_command") {
        if let Some(input) = extract_json_string_field(line, "\"input\"") {
            extract_js_cmd_literals(&input, &mut out);
        }
        return out;
    }
    // 2. function_call with exec_command / shell_command / shell
    if line.contains("\"function_call\"")
        && (line.contains("\"exec_command\"")
            || line.contains("\"shell_command\"")
            || line.contains("\"shell\""))
        && let Some(args_str) = extract_json_string_field(line, "\"arguments\"")
        && let Some(cmd) = extract_json_string_field(&args_str, "\"cmd\"")
            .or_else(|| extract_json_string_field(&args_str, "\"command\""))
    {
        out.push(cmd);
    }
    out
}

fn extract_js_cmd_literals(js: &str, out: &mut Vec<String>) {
    let mut pos = 0;
    while let Some(idx) = js[pos..].find("cmd:") {
        let mut i = pos + idx + 4;
        let bytes = js.as_bytes();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if (quote == b'"' || quote == b'\'' || quote == b'`')
            && let Some((s, next_i)) = parse_quoted_str(js, i, quote)
        {
            out.push(s);
            pos = next_i;
            continue;
        }
        pos = i + 1;
    }
}

fn extract_claude_cmds(line: &str) -> Vec<String> {
    if !line.contains("\"Bash\"") || !line.contains("\"tool_use\"") {
        return Vec::new();
    }
    extract_all_command_fields(line)
}

fn extract_copilot_cmds(line: &str) -> Vec<String> {
    if !line.contains("\"tool.execution_start\"") {
        return Vec::new();
    }
    if !(line.contains("\"toolName\":\"bash\"")
        || line.contains("\"toolName\":\"shell\"")
        || line.contains("\"toolName\":\"powershell\""))
    {
        return Vec::new();
    }
    extract_all_command_fields(line)
}

fn extract_pi_cmds(line: &str) -> Vec<String> {
    if !line.contains("\"toolCall\"") || !line.contains("\"bash\"") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(idx) = line[pos..].find("\"type\":\"toolCall\"") {
        let start = pos + idx;
        // Find next `"name"` and verify it's `"bash"` before extracting `"command"`
        let window_end = (start + 4096).min(line.len());
        let mut slice_end = window_end;
        while slice_end > start && !line.is_char_boundary(slice_end) {
            slice_end -= 1;
        }
        let slice = &line[start..slice_end];
        if slice.contains("\"name\":\"bash\"")
            && let Some(cmd_idx) = slice.find("\"command\"")
            && let Some((val, _)) =
                parse_json_value_after_colon(slice, cmd_idx + "\"command\"".len())
        {
            out.push(val);
        }
        pos = start + "\"type\":\"toolCall\"".len();
    }
    out
}

fn extract_all_command_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(idx) = line[pos..].find("\"command\"") {
        let abs = pos + idx + "\"command\"".len();
        if let Some((val, next_pos)) = parse_json_value_after_colon(line, abs) {
            out.push(val);
            pos = next_pos;
        } else {
            break;
        }
    }
    out
}

/// Split a shell command line into segments and classify file-reading commands.
/// Distinguishes file reads (`cat foo.rs`, `head -n 50 foo.rs`, `sed -n '1,50p' foo.rs`)
/// from piped filters (`rg foo | head -n 10`), heredoc writes (`cat <<EOF > f`), and edits (`sed -i`).
fn classify_shell_command(cmd: &str) -> Vec<(ReadCmdKind, String)> {
    let mut results = Vec::new();
    // Split on top-level `&&`, `||`, `;`, `\n` (keeping pipelines intact so we know what's piped!)
    for stmt in split_shell_statements(cmd) {
        // Only inspect the FIRST command of each pipeline (piped `| head` / `| tail` / `| sed`
        // is filtering stdout of a previous command, not reading a file!).
        let first_in_pipe = split_first_pipe(&stmt);
        let trimmed = first_in_pipe.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(kind) = classify_single_command(trimmed) {
            results.push((kind, trimmed.to_string()));
        }
    }
    results
}

fn classify_single_command(cmd: &str) -> Option<ReadCmdKind> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // Skip leading env assignments (`FOO=bar`), `env`, `command`, `time`
    let mut idx = 0;
    while idx < tokens.len() {
        let t = tokens[idx];
        if (t.contains('=') && !t.starts_with('-')) || t == "env" || t == "command" || t == "time" {
            idx += 1;
            continue;
        }
        break;
    }
    if idx >= tokens.len() {
        return None;
    }

    let bin = Path::new(tokens[idx])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(tokens[idx]);
    let rest = &tokens[idx + 1..];

    match bin {
        "trm" | "strm" => match rest.first().copied() {
            Some("read") | Some("cat") | Some("head") | Some("tail") | Some("sed") => {
                Some(ReadCmdKind::TrmRead)
            }
            Some("rg") | Some("grep") => Some(ReadCmdKind::TrmRg),
            Some("ls") | Some("find") => Some(ReadCmdKind::TrmFs),
            Some("swift") | Some("swiftc") | Some("cargo") => Some(ReadCmdKind::TrmBuild),
            Some("git") => Some(ReadCmdKind::TrmGit),
            _ => None,
        },
        "ls" => Some(ReadCmdKind::Ls),
        "find" => {
            // Ignore mutating find invocations (`-delete`, `-exec`, `-execdir`)
            let mutating = rest
                .iter()
                .any(|t| matches!(*t, "-delete" | "-exec" | "-execdir" | "-ok"));
            if mutating {
                None
            } else {
                Some(ReadCmdKind::Find)
            }
        }
        "git" => match rest.first().copied() {
            Some("status") => {
                let already_compact = rest[1..]
                    .iter()
                    .any(|a| *a == "-s" || *a == "--short" || a.starts_with("--porcelain"));
                if already_compact {
                    None
                } else {
                    Some(ReadCmdKind::Git)
                }
            }
            Some("log") => {
                let already_compact = rest[1..].iter().any(|a| {
                    *a == "--oneline" || a.starts_with("--format") || a.starts_with("--pretty")
                });
                if already_compact {
                    None
                } else {
                    Some(ReadCmdKind::Git)
                }
            }
            Some("diff") => {
                let already_compact = rest[1..].iter().any(|a| {
                    matches!(
                        *a,
                        "--stat" | "--shortstat" | "--numstat" | "--name-only" | "--name-status"
                    )
                });
                if already_compact {
                    None
                } else {
                    Some(ReadCmdKind::Git)
                }
            }
            _ => None,
        },
        "swift" => match rest.first().copied() {
            Some("build") | Some("test") => Some(ReadCmdKind::Swift),
            _ => None,
        },
        "swiftc" => Some(ReadCmdKind::Swift),
        "cargo" => match rest.first().copied() {
            Some("build") | Some("test") | Some("check") | Some("clippy") | Some("b")
            | Some("c") => Some(ReadCmdKind::Cargo),
            _ => None,
        },
        "rg" => Some(ReadCmdKind::Rg),
        "grep" | "egrep" | "fgrep" => {
            if !rest.is_empty() {
                Some(ReadCmdKind::Grep)
            } else {
                None
            }
        }
        "cat" => {
            // Ignore heredoc / redirection writes like `cat <<EOF > file`
            if rest
                .iter()
                .any(|t| t.starts_with("<<") || *t == ">" || *t == ">>")
            {
                return None;
            }
            // Must have at least one non-flag file argument
            if rest.iter().any(|t| !t.starts_with('-')) {
                Some(ReadCmdKind::Cat)
            } else {
                None
            }
        }
        "head" => {
            if has_file_arg_for_head_tail(rest) {
                Some(ReadCmdKind::Head)
            } else {
                None
            }
        }
        "tail" => {
            if has_file_arg_for_head_tail(rest) {
                Some(ReadCmdKind::Tail)
            } else {
                None
            }
        }
        "sed" => {
            // Only count `sed -n ...` file reading (not `sed -i` edits)
            let has_n = rest
                .iter()
                .any(|t| *t == "-n" || *t == "--quiet" || *t == "--silent");
            let has_i = rest.iter().any(|t| t.starts_with("-i"));
            if has_n && !has_i && rest.len() >= 3 {
                Some(ReadCmdKind::SedRead)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn has_file_arg_for_head_tail(rest: &[&str]) -> bool {
    let mut i = 0;
    while i < rest.len() {
        let t = rest[i];
        if t == "-n" || t == "-c" {
            i += 2;
            continue;
        }
        if t.starts_with('-') {
            i += 1;
            continue;
        }
        return true;
    }
    false
}

fn split_shell_statements(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let bytes = cmd.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            let ch = cmd[i..].chars().next().unwrap();
            cur.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if in_double {
            if b == b'\\' && i + 1 < bytes.len() {
                let ch = cmd[i + 1..].chars().next().unwrap();
                cur.push('\\');
                cur.push(ch);
                i += 1 + ch.len_utf8();
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            let ch = cmd[i..].chars().next().unwrap();
            cur.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if b == b'\'' {
            in_single = true;
            cur.push('\'');
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double = true;
            cur.push('"');
            i += 1;
            continue;
        }
        if b == b';' || b == b'\n' {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            i += 1;
            continue;
        }
        if i + 1 < bytes.len()
            && ((b == b'&' && bytes[i + 1] == b'&') || (b == b'|' && bytes[i + 1] == b'|'))
        {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            i += 2;
            continue;
        }
        let ch = cmd[i..].chars().next().unwrap();
        cur.push(ch);
        i += ch.len_utf8();
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn split_first_pipe(stmt: &str) -> &str {
    let bytes = stmt.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' {
            in_single = true;
        } else if b == b'"' {
            in_double = true;
        } else if b == b'|' {
            return &stmt[..i];
        }
        i += 1;
    }
    stmt
}

fn extract_json_string_field(line: &str, key_quoted: &str) -> Option<String> {
    let idx = line.find(key_quoted)?;
    let (val, _) = parse_json_value_after_colon(line, idx + key_quoted.len())?;
    Some(val)
}

fn parse_json_value_after_colon(s: &str, mut i: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    parse_quoted_str(s, i, b'"')
}

fn parse_quoted_str(s: &str, start: usize, quote: u8) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&quote) {
        return None;
    }
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            return Some((out, i + 1));
        }
        if b == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return None;
            }
            match bytes[i] {
                b'"' => out.push('"'),
                b'\'' => out.push('\''),
                b'`' => out.push('`'),
                b'\\' => out.push('\\'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' if i + 4 < bytes.len() => {
                    let hex = &s[i + 1..i + 5];
                    if let Ok(code) = u32::from_str_radix(hex, 16)
                        && let Some(ch) = char::from_u32(code)
                    {
                        out.push(ch);
                    }
                    i += 4;
                }
                other => out.push(other as char),
            }
            i += 1;
            continue;
        }
        let ch = s[i..].chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    None
}

/// Parse ISO-8601 UTC timestamp like `2026-11-26T11:15:30Z` or `2026-11-26T11:15:30.123Z`.
pub(crate) fn parse_iso8601_utc(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    // Howard Hinnant's days-from-civil algorithm (days since 1970-01-01)
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = month as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + (day as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + (doe as i64) - 719_468;
    if days < 0 {
        return None;
    }
    Some((days as u64) * 86_400 + hour * 3_600 + min * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn promotes_hook_rewritten_bypasses_when_telemetry_matches() {
        let mut events: HashMap<(String, String), Vec<(u64, String)>> = HashMap::new();
        events.insert(
            ("sess-1".into(), "cargo".into()),
            vec![(1_005, "cargo:test".into())],
        );
        events.insert(
            ("sess-1".into(), "rg".into()),
            vec![(1_005, "rg:token".into())],
        );

        // Pre-hook `cargo test --offline` with matching `trm cargo` event -> adopted
        assert_eq!(
            promote_if_hook_rewritten(
                "agy",
                ReadCmdKind::Cargo,
                "cargo test --offline",
                "cargo test --offline",
                Some("sess-1"),
                Some(1_000),
                &mut events,
            ),
            ReadCmdKind::TrmBuild
        );

        // Second `cargo test --offline` cannot reuse the already-consumed event -> stays bypass
        assert_eq!(
            promote_if_hook_rewritten(
                "agy",
                ReadCmdKind::Cargo,
                "cargo test --offline",
                "cargo test --offline",
                Some("sess-1"),
                Some(1_002),
                &mut events,
            ),
            ReadCmdKind::Cargo
        );

        // Partial chain: `grep --include=... && rg token src` only rewrites `rg`, NOT `grep`
        let chain = "grep --include=*.rs token src && rg token src";
        assert_eq!(
            promote_if_hook_rewritten(
                "agy",
                ReadCmdKind::Grep,
                chain,
                "grep --include=*.rs token src",
                Some("sess-1"),
                Some(1_000),
                &mut events,
            ),
            ReadCmdKind::Grep
        );
        assert_eq!(
            promote_if_hook_rewritten(
                "agy",
                ReadCmdKind::Rg,
                chain,
                "rg token src",
                Some("sess-1"),
                Some(1_000),
                &mut events,
            ),
            ReadCmdKind::TrmRg
        );

        // No matching telemetry event in another session -> stays bypass
        assert_eq!(
            promote_if_hook_rewritten(
                "agy",
                ReadCmdKind::Cargo,
                "cargo test --offline",
                "cargo test --offline",
                Some("other-sess"),
                Some(1_000),
                &mut events,
            ),
            ReadCmdKind::Cargo
        );

        // Executable-token classification: `trm cargo test swift_fixture` is cargo, not swift
        assert_eq!(
            explicit_trm_telemetry_cmd(ReadCmdKind::TrmBuild, "trm cargo test swift_fixture"),
            Some("cargo")
        );
        assert_eq!(
            explicit_trm_telemetry_cmd(ReadCmdKind::TrmBuild, "trm swift test"),
            Some("swift")
        );

        // Same-second query isolation & no backwards stealing:
        // explicit `trm rg alphabet` (0 hits) at t=1000 cannot steal `rg:alpha` from t=1000
        let mut scoped: HashMap<(String, String), Vec<(u64, String)>> = HashMap::new();
        scoped.insert(
            ("sess-1".into(), "rg".into()),
            vec![(1_000, "rg:alpha".into())],
        );
        assert!(!consume_hook_event(
            &mut scoped,
            "sess-1",
            "rg",
            "trm rg alphabet src/",
            1_000
        ));
        assert!(!consume_hook_event(
            &mut scoped,
            "sess-1",
            "rg",
            "rg alpha src/",
            1_002
        ));
        assert!(consume_hook_event(
            &mut scoped,
            "sess-1",
            "rg",
            "rg alpha src/",
            1_000
        ));
    }
}
