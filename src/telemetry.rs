use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::{LineRange, StatsOptions};
use crate::style::{self, Cell, Theme};

#[derive(Debug, Clone)]
pub struct Event {
    pub ts: u64,
    pub cmd: String,
    pub agent: String,
    pub session: Option<String>,
    pub provider: String,
    pub model: String,
    pub path: String,
    pub lang: String,
    pub req_mode: String,
    pub eff_mode: String,
    pub range: Option<String>,
    pub total_lines: usize,
    pub raw_bytes: usize,
    pub out_bytes: usize,
    pub raw_tokens: usize,
    pub out_tokens: usize,
}

impl Event {
    fn model_key(&self) -> String {
        if self.provider == "unknown" && self.model == "unknown" {
            "unknown".to_string()
        } else if self.provider == "unknown" {
            self.model.clone()
        } else if self.model == "unknown" {
            format!("{}/unknown", self.provider)
        } else {
            format!("{}/{}", self.provider, self.model)
        }
    }

    /// Distinguishes explicit `--raw` from ranged slices (`slice`) and auto
    /// small/never-expand passthroughs (`pass`) for display and `by_mode` rollups.
    pub fn display_mode(&self) -> &str {
        if self.eff_mode == "raw" {
            if self.range.is_some() {
                "slice"
            } else if self.req_mode == "raw" {
                "raw"
            } else {
                "pass"
            }
        } else {
            &self.eff_mode
        }
    }
}

pub const LARGE_PASSTHROUGH_MIN_TOKENS: usize = 300;

#[derive(Debug, Clone)]
pub struct LargePassthrough {
    pub ts: u64,
    pub cmd: String,
    pub lang: String,
    pub path: String,
    pub agent: String,
    pub model_key: String,
    pub total_lines: usize,
    pub raw_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct TrimMiss {
    pub ts: u64,
    pub cmd: String,
    pub path: String,
    pub agent: String,
    pub model_key: String,
    pub first_mode: String,
    pub followup_mode: String,
    pub followup_range: Option<String>,
    pub total_lines: usize,
    pub delta_secs: u64,
    pub wasted_tokens: usize,
}

#[derive(Debug, Default, Clone)]
pub struct Bucket {
    pub reads: usize,
    pub raw_tokens: usize,
    pub out_tokens: usize,
    pub misses: usize,
    pub wasted_tokens: usize,
    pub drilldowns: usize,
}

#[derive(Debug, Default, Clone)]
pub struct StatsSnapshot {
    pub events_count: usize,
    pub raw_tokens: usize,
    pub out_tokens: usize,
    pub gross_saved: isize,
    pub net_saved: isize,
    pub gross_pct: f64,
    pub net_pct: f64,
    pub wasted_tokens: usize,
    pub misses_count: usize,
    pub drilldowns: usize,
    pub turns_saved: usize,
    pub daily_saved: Vec<usize>, // last 7 days net tokens saved (oldest -> newest)
    pub by_cmd: BTreeMap<String, Bucket>,
    pub by_mode: BTreeMap<String, Bucket>,
    pub by_agent: BTreeMap<String, Bucket>,
    pub by_model: BTreeMap<String, Bucket>,
    pub by_lang: BTreeMap<String, Bucket>,
    pub misses: Vec<TrimMiss>,
    pub large_passthroughs: Vec<LargePassthrough>,
    pub recent_misses: Vec<String>,
    pub recent_events: Vec<(u64, String, String, String, usize, usize)>, // ts, agent, cmd, path, raw_tok, out_tok
}

enum StatsLoad {
    EmptyLog,
    EmptyFiltered,
    Ready {
        events: Vec<Event>,
        snap: Box<StatsSnapshot>,
        raw_bytes: usize,
        out_bytes: usize,
    },
}

fn compute_stats(
    since_secs: Option<u64>,
    agent_filter: Option<&str>,
    model_filter: Option<&str>,
    reread_window_secs: u64,
) -> StatsLoad {
    flush_fallback_into_primary();
    let lp = primary_log_path();
    let fb = fallback_log_path();
    let mut raw = String::new();
    if lp.exists()
        && let Ok(s) = fs::read_to_string(&lp)
    {
        raw.push_str(&s);
    }
    if let Some(ref f) = fb
        && f.exists()
        && let Ok(s) = fs::read_to_string(f)
    {
        raw.push_str(&s);
    }
    if raw.trim().is_empty() {
        return StatsLoad::EmptyLog;
    }

    let now = now_secs();
    let cutoff = since_secs.map(|d| now.saturating_sub(d));
    let mut events: Vec<Event> = raw
        .lines()
        .filter_map(parse_event)
        .filter(|e| cutoff.is_none_or(|c| e.ts >= c))
        .collect();
    enrich_models_from_sessions(&mut events);
    events.retain(|e| {
        agent_filter.is_none_or(|af| e.agent.to_lowercase() == af)
            && model_filter.is_none_or(|mf| e.model_key().to_lowercase().contains(mf))
    });
    if events.is_empty() {
        return StatsLoad::EmptyFiltered;
    }
    events.sort_by_key(|e| e.ts);

    let mut total_raw_bytes = 0usize;
    let mut total_out_bytes = 0usize;
    let mut snap = StatsSnapshot {
        events_count: events.len(),
        ..StatsSnapshot::default()
    };

    for e in &events {
        total_raw_bytes += e.raw_bytes;
        total_out_bytes += e.out_bytes;
        snap.raw_tokens += e.raw_tokens;
        snap.out_tokens += e.out_tokens;
        let dmode = e.display_mode().to_string();
        for b in [
            snap.by_cmd.entry(e.cmd.clone()).or_default(),
            snap.by_mode.entry(dmode).or_default(),
            snap.by_agent.entry(e.agent.clone()).or_default(),
            snap.by_model.entry(e.model_key()).or_default(),
            snap.by_lang.entry(e.lang.clone()).or_default(),
        ] {
            b.reads += 1;
            b.raw_tokens += e.raw_tokens;
            b.out_tokens += e.out_tokens;
        }
        if e.display_mode() == "pass" && e.raw_tokens >= LARGE_PASSTHROUGH_MIN_TOKENS {
            snap.large_passthroughs.push(LargePassthrough {
                ts: e.ts,
                cmd: e.cmd.clone(),
                lang: e.lang.clone(),
                path: e.path.clone(),
                agent: e.agent.clone(),
                model_key: e.model_key(),
                total_lines: e.total_lines,
                raw_tokens: e.raw_tokens,
            });
        }
    }
    snap.large_passthroughs.sort_by(|a, b| {
        b.ts.cmp(&a.ts)
            .then_with(|| b.raw_tokens.cmp(&a.raw_tokens))
    });

    for (i, e) in events.iter().enumerate() {
        if e.eff_mode == "raw"
            || e.eff_mode == "all"
            || e.eff_mode == "symbol"
            || e.path == "<stdin>"
        {
            continue;
        }
        for next in &events[i + 1..] {
            let delta = next.ts.saturating_sub(e.ts);
            if delta > reread_window_secs {
                break;
            }
            if next.cmd != e.cmd || next.path != e.path || !same_actor(e, next) {
                continue;
            }
            if e.cmd != "read" {
                // Only count explicit --all / --raw retries as misses (not auto never-expand fallbacks)
                if next.req_mode == "all" || next.req_mode == "raw" {
                    snap.misses_count += 1;
                    snap.wasted_tokens += e.out_tokens;
                    for b in [
                        snap.by_cmd.entry(e.cmd.clone()).or_default(),
                        snap.by_mode
                            .entry(e.display_mode().to_string())
                            .or_default(),
                        snap.by_agent.entry(e.agent.clone()).or_default(),
                        snap.by_model.entry(e.model_key()).or_default(),
                        snap.by_lang.entry(e.lang.clone()).or_default(),
                    ] {
                        b.misses += 1;
                        b.wasted_tokens += e.out_tokens;
                    }
                    snap.misses.push(TrimMiss {
                        ts: e.ts,
                        cmd: e.cmd.clone(),
                        path: e.path.clone(),
                        agent: e.agent.clone(),
                        model_key: e.model_key(),
                        first_mode: e.eff_mode.clone(),
                        followup_mode: next.req_mode.clone(),
                        followup_range: next.range.clone(),
                        total_lines: e.total_lines,
                        delta_secs: delta,
                        wasted_tokens: e.out_tokens,
                    });
                    snap.recent_misses.push(format!(
                        "{:>7} [{}] {} ({} -> {} in {}s, -{} tok)",
                        fmt_age(e.ts, now),
                        e.agent,
                        short_path(&e.path),
                        e.eff_mode,
                        next.req_mode,
                        delta,
                        e.out_tokens
                    ));
                    break;
                }
                continue;
            }
            if next.eff_mode != "raw" {
                continue;
            }
            let cov = range_coverage(next.range.as_deref(), e.total_lines);
            if e.eff_mode == "outline" && cov < 0.85 {
                snap.drilldowns += 1;
                snap.by_cmd.entry(e.cmd.clone()).or_default().drilldowns += 1;
                snap.by_mode
                    .entry(e.display_mode().to_string())
                    .or_default()
                    .drilldowns += 1;
                snap.by_agent.entry(e.agent.clone()).or_default().drilldowns += 1;
                snap.by_model.entry(e.model_key()).or_default().drilldowns += 1;
                snap.by_lang.entry(e.lang.clone()).or_default().drilldowns += 1;
                break;
            }
            if next.range.is_none() || cov >= 0.85 {
                snap.misses_count += 1;
                snap.wasted_tokens += e.out_tokens;
                for b in [
                    snap.by_cmd.entry(e.cmd.clone()).or_default(),
                    snap.by_mode
                        .entry(e.display_mode().to_string())
                        .or_default(),
                    snap.by_agent.entry(e.agent.clone()).or_default(),
                    snap.by_model.entry(e.model_key()).or_default(),
                    snap.by_lang.entry(e.lang.clone()).or_default(),
                ] {
                    b.misses += 1;
                    b.wasted_tokens += e.out_tokens;
                }
                snap.misses.push(TrimMiss {
                    ts: e.ts,
                    cmd: e.cmd.clone(),
                    path: e.path.clone(),
                    agent: e.agent.clone(),
                    model_key: e.model_key(),
                    first_mode: e.eff_mode.clone(),
                    followup_mode: "raw".to_string(),
                    followup_range: next.range.clone(),
                    total_lines: e.total_lines,
                    delta_secs: delta,
                    wasted_tokens: e.out_tokens,
                });
                snap.recent_misses.push(format!(
                    "{:>7} [{}] {} ({} -> raw in {}s, -{} tok)",
                    fmt_age(e.ts, now),
                    e.agent,
                    short_path(&e.path),
                    e.eff_mode,
                    delta,
                    e.out_tokens
                ));
                break;
            }
        }
    }

    snap.gross_saved = snap.raw_tokens as isize - snap.out_tokens as isize;
    snap.net_saved = snap.gross_saved - snap.wasted_tokens as isize;
    snap.gross_pct = pct(snap.gross_saved, snap.raw_tokens);
    snap.net_pct = pct(snap.net_saved, snap.raw_tokens);

    // Count verified `rg:block` turns saved (not followed by a `read` within window)
    for (i, e) in events.iter().enumerate() {
        if e.cmd == "rg" && e.eff_mode == "block" {
            let followed_by_read = events[i + 1..].iter().any(|n| {
                n.ts.saturating_sub(e.ts) <= reread_window_secs
                    && n.cmd == "read"
                    && same_actor(e, n)
            });
            if !followed_by_read {
                snap.turns_saved += 1;
            }
        }
    }

    // 7-day savings sparkline buckets (oldest -> newest)
    let mut daily = vec![0usize; 7];
    for e in &events {
        let age = now.saturating_sub(e.ts);
        let days_ago = (age / 86_400) as usize;
        if days_ago < 7 {
            let saved = e.raw_tokens.saturating_sub(e.out_tokens);
            daily[6 - days_ago] += saved;
        }
    }
    snap.daily_saved = daily;

    snap.recent_misses.reverse();
    snap.recent_events = events
        .iter()
        .rev()
        .take(120)
        .map(|e| {
            let mode_label = format!("{}:{}", e.cmd, e.display_mode());
            let actor = if e.model != "unknown" {
                format!("{}/{}", e.agent, e.model)
            } else {
                e.agent.clone()
            };
            (
                e.ts,
                actor,
                mode_label,
                short_path(&e.path),
                e.raw_tokens,
                e.out_tokens,
            )
        })
        .collect();

    StatsLoad::Ready {
        events,
        snap: Box::new(snap),
        raw_bytes: total_raw_bytes,
        out_bytes: total_out_bytes,
    }
}

pub fn load_snapshot(since_secs: Option<u64>) -> StatsSnapshot {
    let window = crate::config::Config::load().reread_window_secs;
    match compute_stats(since_secs, None, None, window) {
        StatsLoad::Ready { snap, .. } => *snap,
        _ => StatsSnapshot::default(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn detect_agent() -> String {
    if let Some(a) = non_empty_env("TRM_AGENT") {
        let lower = a.to_lowercase();
        return if lower == "antigravity" {
            "agy".into()
        } else {
            lower
        };
    }
    if non_empty_env("ANTIGRAVITY_AGENT").is_some()
        || non_empty_env("ANTIGRAVITY_CONVERSATION_ID").is_some()
    {
        return "agy".into();
    }
    if non_empty_env("CODEX_THREAD_ID").is_some()
        || non_empty_env("CODEX_SANDBOX").is_some()
        || non_empty_env("CODEX_CI").is_some()
        || non_empty_env("CODEX_CLI").is_some()
    {
        return "codex".into();
    }
    if non_empty_env("CLAUDECODE").is_some() || non_empty_env("CLAUDE_CODE_ENTRYPOINT").is_some() {
        return "claude".into();
    }
    if non_empty_env("COPILOT_CLI").is_some()
        || non_empty_env("GITHUB_COPILOT_CLI").is_some()
        || non_empty_env("COPILOT_SESSION_ID").is_some()
    {
        return "copilot".into();
    }
    if non_empty_env("PI_CODING_AGENT").is_some()
        || non_empty_env("PI_SESSION_ID").is_some()
        || non_empty_env("PI_AGENT_DIR").is_some()
    {
        return "pi".into();
    }
    if non_empty_env("CURSOR_AGENT").is_some() || non_empty_env("CURSOR_TRACE_ID").is_some() {
        return "cursor".into();
    }
    if non_empty_env("AIDER_MODEL").is_some() {
        return "aider".into();
    }
    if let Some(tp) = non_empty_env("TERM_PROGRAM") {
        return tp.to_lowercase();
    }
    "shell".into()
}

fn detect_session() -> Option<String> {
    non_empty_env("TRM_SESSION")
        .or_else(|| non_empty_env("ANTIGRAVITY_CONVERSATION_ID"))
        .or_else(|| non_empty_env("CODEX_THREAD_ID"))
        .or_else(|| non_empty_env("CLAUDE_SESSION_ID"))
        .or_else(|| non_empty_env("CLAUDE_CODE_SESSION_ID"))
        .or_else(|| non_empty_env("COPILOT_SESSION_ID"))
        .or_else(|| non_empty_env("PI_SESSION_ID"))
}

fn detect_provider_and_model() -> (String, String) {
    let explicit_provider = non_empty_env("TRM_PROVIDER").map(|s| s.to_lowercase());
    if let Some(m) = non_empty_env("TRM_MODEL") {
        if let Some((p, rest)) = m.split_once('/') {
            return (
                explicit_provider.unwrap_or_else(|| p.to_lowercase()),
                rest.to_string(),
            );
        }
        let prov = explicit_provider.unwrap_or_else(|| infer_provider(&m));
        return (prov, m);
    }

    let candidates: &[(&str, &str)] = &[
        ("CODEX_MODEL", "openai"),
        ("OPENAI_MODEL", "openai"),
        ("ANTHROPIC_MODEL", "anthropic"),
        ("CLAUDE_MODEL", "anthropic"),
        ("GEMINI_MODEL", "google"),
        ("GOOGLE_MODEL", "google"),
        ("AIDER_MODEL", "unknown"),
        ("OLLAMA_MODEL", "ollama"),
    ];
    for &(var, default_prov) in candidates {
        if let Some(m) = non_empty_env(var) {
            if let Some((p, rest)) = m.split_once('/') {
                return (
                    explicit_provider.unwrap_or_else(|| p.to_lowercase()),
                    rest.to_string(),
                );
            }
            let prov = explicit_provider.unwrap_or_else(|| {
                if default_prov == "unknown" {
                    infer_provider(&m)
                } else {
                    default_prov.to_string()
                }
            });
            return (prov, m);
        }
    }

    (
        explicit_provider.unwrap_or_else(|| "unknown".into()),
        "unknown".into(),
    )
}

fn infer_provider(model: &str) -> String {
    let low = model.to_lowercase();
    if low.contains("claude")
        || low.contains("sonnet")
        || low.contains("opus")
        || low.contains("haiku")
    {
        "anthropic".into()
    } else if low.starts_with("gpt")
        || low.starts_with("o1")
        || low.starts_with("o3")
        || low.starts_with("o4")
        || low.contains("codex")
    {
        "openai".into()
    } else if low.contains("gemini") {
        "google".into()
    } else if low.contains("kimi") {
        "moonshot".into()
    } else if low.contains("deepseek") {
        "deepseek".into()
    } else if low.contains("qwen") {
        "qwen".into()
    } else {
        "unknown".into()
    }
}

fn primary_log_path() -> PathBuf {
    if let Ok(p) = env::var("TRM_LOG_PATH") {
        return PathBuf::from(p);
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("trm")
        .join("events.jsonl")
}

fn fallback_log_path() -> Option<PathBuf> {
    if env::var("TRM_LOG_PATH").is_ok() {
        return None;
    }
    let user = env::var("USER").unwrap_or_else(|_| "default".into());
    Some(PathBuf::from(format!("/tmp/trm-events-{user}.jsonl")))
}

fn append_line(path: &PathBuf, line: &str) -> bool {
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        return writeln!(file, "{line}").is_ok();
    }
    false
}

/// If sandboxed agents wrote to `/tmp/trm-events-<user>.jsonl`, flush those lines
/// into the persistent primary log whenever the primary log is writable.
/// Uses an atomic `rename` to claim the fallback file so concurrent readers
/// (e.g. `trm ui` + another command) can never flush the same events twice.
fn flush_fallback_into_primary() {
    let Some(fb) = fallback_log_path() else {
        return;
    };
    if !fb.exists() {
        return;
    }
    let primary = primary_log_path();
    if let Some(parent) = primary.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&primary) else {
        return;
    };
    let claimed = fb.with_extension(format!("jsonl.{}.flushing", std::process::id()));
    if fs::rename(&fb, &claimed).is_err() {
        return;
    }
    if let Ok(content) = fs::read_to_string(&claimed)
        && !content.trim().is_empty()
        && file.write_all(content.as_bytes()).is_err()
    {
        // Best-effort restore if write failed after claim
        let _ = fs::rename(&claimed, &fb);
        return;
    }
    let _ = fs::remove_file(&claimed);
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Empirical bytes-per-token factor scaled by 10 for integer fixed-point math.
pub fn bytes_per_token_x10(lang: &str) -> usize {
    match lang {
        "json" => 32,
        "yaml" | "toml" | "sql" => 35,
        "git" | "search" | "fs" | "cargo" | "swiftc" => 37,
        "swift" | "rust" | "ts" | "js" | "go" | "c-like" | "python" | "shell" => 38,
        "markdown" | "text" => 42,
        _ => 40,
    }
}

pub fn estimate_tokens_for_lang(bytes: usize, lang: &str) -> usize {
    if bytes == 0 {
        return 0;
    }
    (bytes * 10).div_ceil(bytes_per_token_x10(lang))
}

#[allow(clippy::too_many_arguments)]
pub fn log_read_event(
    path: &str,
    lang: &str,
    req_mode: &str,
    eff_mode: &str,
    range: Option<String>,
    total_lines: usize,
    raw_bytes: usize,
    out_bytes: usize,
) {
    let (provider, model) = detect_provider_and_model();
    let ev = Event {
        ts: now_secs(),
        cmd: "read".into(),
        agent: detect_agent(),
        session: detect_session(),
        provider,
        model,
        path: path.into(),
        lang: lang.into(),
        req_mode: req_mode.into(),
        eff_mode: eff_mode.into(),
        range,
        total_lines,
        raw_bytes,
        out_bytes,
        raw_tokens: estimate_tokens_for_lang(raw_bytes, lang),
        out_tokens: estimate_tokens_for_lang(out_bytes, lang),
    };

    let line = serialize_event(&ev);
    let primary = primary_log_path();
    if !append_line(&primary, &line)
        && let Some(fb) = fallback_log_path()
    {
        let _ = append_line(&fb, &line);
    }
}

pub fn log_search_event(
    query_key: &str,
    req_mode: &str,
    eff_mode: &str,
    total_lines: usize,
    raw_bytes: usize,
    out_bytes: usize,
) {
    log_cmd_event(
        "rg",
        query_key,
        "search",
        req_mode,
        eff_mode,
        total_lines,
        raw_bytes,
        out_bytes,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn log_cmd_event(
    cmd: &str,
    path: &str,
    lang: &str,
    req_mode: &str,
    eff_mode: &str,
    total_lines: usize,
    raw_bytes: usize,
    out_bytes: usize,
) {
    let (provider, model) = detect_provider_and_model();
    let ev = Event {
        ts: now_secs(),
        cmd: cmd.into(),
        agent: detect_agent(),
        session: detect_session(),
        provider,
        model,
        path: path.into(),
        lang: lang.into(),
        req_mode: req_mode.into(),
        eff_mode: eff_mode.into(),
        range: None,
        total_lines,
        raw_bytes,
        out_bytes,
        raw_tokens: estimate_tokens_for_lang(raw_bytes, lang),
        out_tokens: estimate_tokens_for_lang(out_bytes, lang),
    };

    let line = serialize_event(&ev);
    let primary = primary_log_path();
    if !append_line(&primary, &line)
        && let Some(fb) = fallback_log_path()
    {
        let _ = append_line(&fb, &line);
    }
}

fn same_actor(a: &Event, b: &Event) -> bool {
    if a.agent != b.agent {
        return false;
    }
    match (&a.session, &b.session) {
        (Some(sa), Some(sb)) => sa == sb,
        _ => true,
    }
}

pub fn run_stats(opts: &StatsOptions) -> Result<(), String> {
    let lp = primary_log_path();
    let fb = fallback_log_path();

    if opts.reset {
        if lp.exists() {
            let _ = fs::remove_file(&lp);
        }
        if let Some(ref f) = fb
            && f.exists()
        {
            let _ = fs::remove_file(f);
        }
        println!("trm stats reset ({})", lp.display());
        return Ok(());
    }

    let (events, snap, total_raw_bytes, total_out_bytes) = match compute_stats(
        opts.since_secs,
        opts.agent_filter.as_deref(),
        opts.model_filter.as_deref(),
        opts.reread_window_secs,
    ) {
        StatsLoad::EmptyLog => {
            if opts.json {
                println!("{{\"events\":0,\"gross_saved_tokens\":0,\"net_saved_tokens\":0}}");
            } else {
                println!("No telemetry recorded yet ({}).", lp.display());
                println!("Run 'trm read <file>' to start tracking savings.");
            }
            return Ok(());
        }
        StatsLoad::EmptyFiltered => {
            if opts.json {
                println!("{{\"events\":0,\"gross_saved_tokens\":0,\"net_saved_tokens\":0}}");
            } else {
                println!("No events in selected window/filter.");
            }
            return Ok(());
        }
        StatsLoad::Ready {
            events,
            snap,
            raw_bytes,
            out_bytes,
        } => (events, snap, raw_bytes, out_bytes),
    };

    let now = now_secs();

    if opts.json {
        println!(
            "{{\"events\":{},\"raw_bytes\":{},\"out_bytes\":{},\"raw_tokens\":{},\"out_tokens\":{},\"gross_saved_tokens\":{},\"gross_saved_pct\":{:.1},\"trim_misses\":{},\"wasted_tokens\":{},\"outline_drilldowns\":{},\"net_saved_tokens\":{},\"net_saved_pct\":{:.1}}}",
            snap.events_count,
            total_raw_bytes,
            total_out_bytes,
            snap.raw_tokens,
            snap.out_tokens,
            snap.gross_saved,
            snap.gross_pct,
            snap.misses_count,
            snap.wasted_tokens,
            snap.drilldowns,
            snap.net_saved,
            snap.net_pct
        );
        return Ok(());
    }

    let theme = Theme::detect();
    println!(
        "{}",
        theme.header_rule("trm stats", &format!("{} commands", snap.events_count))
    );
    println!(
        "  {:<16} {:>9} tokens  {}",
        theme.dim("Raw input:"),
        fmt_num(snap.raw_tokens),
        theme.dim(&format!("({})", crate::style::fmt_bytes(total_raw_bytes)))
    );
    println!(
        "  {:<16} {:>9} tokens  {}",
        theme.dim("Trimmed output:"),
        fmt_num(snap.out_tokens),
        theme.dim(&format!("({})", crate::style::fmt_bytes(total_out_bytes)))
    );
    println!(
        "  {:<16} {:>9} tokens  {}  {}",
        theme.dim("Gross saved:"),
        fmt_signed(snap.gross_saved),
        theme.pct_color(snap.gross_pct, &format!("{:>5.1}%", snap.gross_pct)),
        theme.bar(snap.gross_pct, 12)
    );
    let waste_str = if snap.wasted_tokens > 0 {
        theme.red(&fmt_num(snap.wasted_tokens))
    } else {
        fmt_num(snap.wasted_tokens)
    };
    println!(
        "  {:<16} {:>9} tokens  {}",
        theme.dim("Re-read waste:"),
        waste_str,
        theme.dim(&format!(
            "({} trim misses, {} outline drill-downs)",
            snap.misses_count, snap.drilldowns
        ))
    );
    println!(
        "  {:<16} {:>9} tokens  {}  {}",
        theme.bold("Net saved:"),
        theme.bold(&fmt_signed(snap.net_saved)),
        theme.pct_color(snap.net_pct, &format!("{:>5.1}%", snap.net_pct)),
        theme.bar(snap.net_pct, 12)
    );

    if snap.by_cmd.len() > 1 {
        print_bucket_table(&theme, "By command", "cmd", &snap.by_cmd);
    }

    print_bucket_table(&theme, "By mode", "mode", &snap.by_mode);

    if opts.by_agent || snap.by_agent.len() > 1 {
        print_bucket_table(&theme, "By agent", "agent", &snap.by_agent);
    }

    let has_known_model = snap.by_model.keys().any(|k| k != "unknown");
    if opts.by_model || has_known_model {
        print_bucket_table(&theme, "By model", "provider/model", &snap.by_model);
    }

    if opts.by_lang {
        print_bucket_table(&theme, "By language", "lang", &snap.by_lang);
    }

    if opts.misses {
        println!(
            "\nTrim misses (same agent/session re-read raw within {}s):",
            opts.reread_window_secs
        );
        if snap.misses.is_empty() {
            println!("  None detected.");
        } else {
            for m in snap.misses.iter().rev().take(20) {
                let rng = m.followup_range.as_deref().unwrap_or("full");
                let who = if m.model_key != "unknown" {
                    format!("{} ({})", m.agent, m.model_key)
                } else {
                    m.agent.clone()
                };
                println!(
                    "  - {:>7} [{}] {} [{} -> raw({}) after {}s, -{} tok]",
                    fmt_age(m.ts, now),
                    who,
                    short_path(&m.path),
                    m.first_mode,
                    rng,
                    m.delta_secs,
                    m.wasted_tokens
                );
            }
        }
    }

    if opts.recommend {
        let hints = collect_recommendations(&events, &snap.misses, &snap.by_agent, &snap.by_model);
        println!("\nRecommendations:");
        if events.len() < 10 {
            println!(
                "  (low sample size: {} commands — collect 20+ commands for reliable tuning)",
                events.len()
            );
        }
        if hints.is_empty() {
            println!("  All metrics look healthy — no threshold adjustments needed.");
        } else {
            for h in hints {
                println!("  - {h}");
            }
        }
    }

    Ok(())
}

fn collect_recommendations(
    events: &[Event],
    misses: &[TrimMiss],
    by_agent: &BTreeMap<String, Bucket>,
    by_model: &BTreeMap<String, Bucket>,
) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();

    // 1. Agent-level high miss rate
    for (agent, b) in by_agent {
        let trimmed_reads = b.reads;
        if trimmed_reads >= 5 && b.misses > 0 {
            let miss_rate = (b.misses as f64 / trimmed_reads as f64) * 100.0;
            if miss_rate >= 20.0 {
                hints.push(format!(
                    "Agent '{agent}' has a {:.0}% miss rate ({}/{} calls, -{} wasted tok) — check `trm stats --agent {agent} --misses`.",
                    miss_rate,
                    b.misses,
                    trimmed_reads,
                    fmt_num(b.wasted_tokens)
                ));
            }
        }
    }

    // 2. Model-level high miss rate
    for (model, b) in by_model {
        if model == "unknown" {
            continue;
        }
        if b.reads >= 5 && b.misses > 0 {
            let miss_rate = (b.misses as f64 / b.reads as f64) * 100.0;
            if miss_rate >= 20.0 {
                hints.push(format!(
                    "Model '{model}' has a {:.0}% miss rate ({}/{} calls) — consider wider ranges or higher outline threshold for this model.",
                    miss_rate, b.misses, b.reads
                ));
            }
        }
    }

    // 3. Outline threshold check (201-260L files)
    let mut mid_outline_total = 0usize;
    let mut mid_outline_misses = 0usize;
    let miss_paths_outline: Vec<&str> = misses
        .iter()
        .filter(|m| m.first_mode == "outline")
        .map(|m| m.path.as_str())
        .collect();
    for e in events {
        if e.cmd == "read" && e.eff_mode == "outline" && e.total_lines <= 260 {
            mid_outline_total += 1;
            if miss_paths_outline.contains(&e.path.as_str()) {
                mid_outline_misses += 1;
            }
        }
    }
    if mid_outline_total >= 4 && (mid_outline_misses as f64 / mid_outline_total as f64) >= 0.25 {
        hints.push(format!(
            "Outline on 201–260L files had {mid_outline_misses}/{mid_outline_total} misses — consider raising OUTLINE_THRESHOLD_LINES from 200 to 260."
        ));
    }

    // 4. Low-yield minimal mode per language
    let mut min_by_lang: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for e in events {
        if e.cmd == "read" && e.eff_mode == "minimal" {
            let entry = min_by_lang.entry(e.lang.as_str()).or_default();
            entry.0 += 1;
            entry.1 += e.raw_tokens;
            entry.2 += e.out_tokens;
        }
    }
    for (lang, (cnt, rtok, otok)) in min_by_lang {
        if cnt >= 5 && rtok > 0 {
            let saved_pct = ((rtok as f64 - otok as f64) / rtok as f64) * 100.0;
            if saved_pct < 5.0 {
                hints.push(format!(
                    "`minimal` mode on '{lang}' saves only {saved_pct:.1}% across {cnt} reads — your '{lang}' files are already clean; outline or raw gives better ROI."
                ));
            }
        }
    }

    // 5. `trm rg` cap pressure (--all / --raw escalations)
    let rg_total = events.iter().filter(|e| e.cmd == "rg").count();
    let rg_misses = misses
        .iter()
        .filter(|m| m.first_mode == "grouped" || m.first_mode == "files")
        .count();
    if rg_total >= 5 && (rg_misses as f64 / rg_total as f64) >= 0.15 {
        let rate = (rg_misses as f64 / rg_total as f64) * 100.0;
        hints.push(format!(
            "`trm rg` escalated to --all/--raw on {rg_misses}/{rg_total} ({rate:.0}%) searches — consider raising default --per-file (6) or --max (50)."
        ));
    }

    // 6. Hot repeat-read raw files
    let mut raw_counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for e in events {
        if e.cmd == "read" && e.eff_mode == "raw" && e.range.is_none() && e.path != "<stdin>" {
            let c = raw_counts.entry(e.path.as_str()).or_default();
            c.0 += 1;
            c.1 += e.out_tokens;
        }
    }
    let mut hot: Vec<(&str, usize, usize)> = raw_counts
        .into_iter()
        .filter(|(_, (cnt, _))| *cnt >= 4)
        .map(|(p, (cnt, tok))| (p, cnt, tok))
        .collect();
    hot.sort_by_key(|a| std::cmp::Reverse(a.2));
    if let Some((path, cnt, tok)) = hot.first() {
        hints.push(format!(
            "Hot repeat raw file: {} read raw {} times ({} tok total) — use `-s <symbol>` or `-r A-B`.",
            short_path(path),
            cnt,
            fmt_num(*tok)
        ));
    }

    // 7. Per-language high miss rate tuning
    let mut lang_reads: BTreeMap<&str, usize> = BTreeMap::new();
    let mut path_lang: BTreeMap<&str, &str> = BTreeMap::new();
    for e in events {
        if e.cmd == "read" {
            path_lang.insert(e.path.as_str(), e.lang.as_str());
            if e.eff_mode != "raw" && e.eff_mode != "pass" && e.eff_mode != "slice" {
                *lang_reads.entry(e.lang.as_str()).or_insert(0) += 1;
            }
        }
    }
    let mut lang_misses: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for m in misses {
        if m.cmd == "read"
            && let Some(&lang) = path_lang.get(m.path.as_str())
        {
            let entry = lang_misses.entry(lang).or_default();
            entry.0 += 1;
            entry.1 += m.wasted_tokens;
        }
    }
    for (lang, reads) in lang_reads {
        if reads >= 5
            && let Some(&(mcnt, wasted)) = lang_misses.get(lang)
            && mcnt > 0
        {
            let rate = (mcnt as f64 / reads as f64) * 100.0;
            if rate >= 20.0 {
                hints.push(format!(
                    "Language '{lang}' has a {rate:.0}% miss rate ({mcnt}/{reads} trimmed reads, -{} wasted tok) — raise `outline_lines` or inspect `trm stats --by-lang --misses`.",
                    fmt_num(wasted)
                ));
            }
        }
    }

    // 8. `rg --block` effectiveness check
    let rg_block_total = events
        .iter()
        .filter(|e| e.cmd == "rg" && e.eff_mode == "block")
        .count();
    let rg_block_misses = misses
        .iter()
        .filter(|m| m.cmd == "rg" && m.first_mode == "block")
        .count();
    if rg_block_total >= 5 {
        let block_rate = (rg_block_misses as f64 / rg_block_total as f64) * 100.0;
        if block_rate >= 25.0 {
            hints.push(format!(
                "`rg --block` triggered follow-up raw reads on {rg_block_misses}/{rg_block_total} ({block_rate:.0}%) searches — try `--no-block` or `-C`."
            ));
        } else if block_rate < 10.0 {
            hints.push(format!(
                "`rg --block` is working cleanly ({rg_block_total} expansions, {block_rate:.0}% follow-up rate)."
            ));
        }
    }

    hints
}

type SessionModelTimeline = Vec<(u64, String, String)>;

fn enrich_models_from_sessions(events: &mut [Event]) {
    let home_path = crate::config::Config::home_dir();
    let mut cache: BTreeMap<(String, String), SessionModelTimeline> = BTreeMap::new();

    for e in events.iter_mut() {
        if e.model != "unknown" {
            continue;
        }
        let Some(ref sid) = e.session else {
            continue;
        };
        let key = (e.agent.clone(), sid.clone());
        let timeline = cache
            .entry(key)
            .or_insert_with(|| resolve_session_model_timeline(&home_path, &e.agent, sid));
        if let Some((prov, model)) = pick_model_at_ts(timeline, e.ts) {
            e.provider = prov.clone();
            e.model = model.clone();
        }
    }
}

fn pick_model_at_ts(timeline: &[(u64, String, String)], ts: u64) -> Option<(&String, &String)> {
    if timeline.is_empty() {
        return None;
    }
    // Latest model transition whose timestamp is <= event.ts (fallback to earliest if ts precedes all)
    let mut chosen = &timeline[0];
    for entry in timeline {
        if entry.0 <= ts {
            chosen = entry;
        } else {
            break;
        }
    }
    Some((&chosen.1, &chosen.2))
}

fn resolve_session_model_timeline(
    home: &std::path::Path,
    agent: &str,
    session_id: &str,
) -> Vec<(u64, String, String)> {
    let candidate: Option<PathBuf> = match agent {
        "agy" | "antigravity" => {
            let p = home
                .join(".gemini")
                .join("antigravity-cli")
                .join("brain")
                .join(session_id)
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            p.exists().then_some(p)
        }
        "copilot" => {
            let p = home
                .join(".copilot")
                .join("session-state")
                .join(session_id)
                .join("events.jsonl");
            p.exists().then_some(p)
        }
        "codex" => find_file_containing(&home.join(".codex").join("sessions"), session_id),
        "claude" => find_file_containing(&home.join(".claude").join("projects"), session_id),
        "pi" => find_file_containing(&home.join(".pi").join("agent").join("sessions"), session_id),
        _ => None,
    };

    match candidate {
        Some(path) => extract_model_timeline(&path),
        None => Vec::new(),
    }
}

fn find_file_containing(dir: &std::path::Path, needle: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file_containing(&p, needle) {
                return Some(found);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(needle))
        {
            return Some(p);
        }
    }
    None
}

fn extract_model_timeline(path: &std::path::Path) -> Vec<(u64, String, String)> {
    use std::io::{BufRead, BufReader};
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);

    let mut timeline: Vec<(u64, String, String)> = Vec::new();
    let mut current_ts: u64 = 0;
    let mut current_provider: Option<String> = None;

    for line in reader.lines().map_while(Result::ok) {
        if let Some(ts_str) = find_json_str_value(&line, "\"timestamp\"")
            .or_else(|| find_json_str_value(&line, "\"created_at\""))
            && let Some(secs) = crate::audit::parse_iso8601_utc(&ts_str)
        {
            current_ts = secs;
        }
        if let Some(p) = find_json_str_value(&line, "\"model_provider_id\"")
            && !p.is_empty()
        {
            current_provider = Some(p.to_lowercase());
        }
        if let Some(m) = find_json_str_value(&line, "\"model\"")
            && !m.is_empty()
            && m != "unknown"
        {
            let (prov, model) = if let Some((p, rest)) = m.split_once('/') {
                (
                    current_provider.clone().unwrap_or_else(|| p.to_lowercase()),
                    rest.to_string(),
                )
            } else {
                let prov = current_provider
                    .clone()
                    .unwrap_or_else(|| infer_provider(&m));
                (prov, m)
            };
            let changed = timeline
                .last()
                .is_none_or(|(_, lp, lm)| *lp != prov || *lm != model);
            if changed {
                timeline.push((current_ts, prov, model));
            }
        }
    }

    timeline
}

fn find_json_str_value(line: &str, key_quoted: &str) -> Option<String> {
    let idx = line.find(key_quoted)?;
    let bytes = line.as_bytes();
    let mut i = idx + key_quoted.len();
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
    let (val, _) = crate::json::JsonParser::parse_string_at(line, i)?;
    Some(val)
}

fn print_bucket_table(theme: &Theme, title: &str, col_name: &str, map: &BTreeMap<String, Bucket>) {
    println!("\n{}", theme.bold(&format!("{title}:")));
    let mut rows_sorted: Vec<(&String, &Bucket)> = map.iter().collect();
    rows_sorted.sort_by_key(|a| std::cmp::Reverse(a.1.raw_tokens));

    let headers: &[(&str, bool)] = &[
        (col_name, false),
        ("calls", true),
        ("raw_tok", true),
        ("out_tok", true),
        ("misses", true),
        ("drill", true),
        ("net%", false),
    ];

    let mut table_rows: Vec<Vec<Cell>> = Vec::with_capacity(rows_sorted.len());
    for (k, b) in rows_sorted {
        let net = (b.raw_tokens as isize - b.out_tokens as isize) - b.wasted_tokens as isize;
        let p = pct(net, b.raw_tokens);
        let p_plain = format!("{p:>5.1}%");
        let miss_plain = b.misses.to_string();
        let miss_cell = if b.misses > 0 {
            Cell::styled(theme.red(&miss_plain), miss_plain.len(), true)
        } else {
            Cell::right(miss_plain)
        };
        let net_rendered = format!("{} {}", theme.pct_color(p, &p_plain), theme.bar(p, 8));
        // Visible width: `p_plain` (6 chars) + 1 space + 8 bar chars = 15
        let net_cell = Cell::styled(net_rendered, p_plain.chars().count() + 9, false);

        table_rows.push(vec![
            Cell::left(k.clone()),
            Cell::right(b.reads.to_string()),
            Cell::right(style::fmt_compact(b.raw_tokens)),
            Cell::right(style::fmt_compact(b.out_tokens)),
            miss_cell,
            Cell::right(b.drilldowns.to_string()),
            net_cell,
        ]);
    }

    for line in style::render_table(theme, headers, &table_rows) {
        println!("{line}");
    }
}

fn range_coverage(range: Option<&str>, total_lines: usize) -> f64 {
    if total_lines == 0 {
        return 1.0;
    }
    let Some(r_str) = range else {
        return 1.0;
    };
    let Ok(r) = LineRange::parse(r_str) else {
        return 1.0;
    };
    let end = r.end.unwrap_or(total_lines).min(total_lines);
    let span = end.saturating_sub(r.start) + 1;
    (span as f64) / (total_lines as f64)
}

fn pct(saved: isize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (saved as f64 / total as f64) * 100.0
    }
}

fn short_path(p: &str) -> String {
    if let Ok(home) = env::var("HOME")
        && let Some(rest) = p.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    p.to_string()
}

pub(crate) fn fmt_age(ts: u64, now: u64) -> String {
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3_600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3_600)
    } else {
        format!("{}d ago", diff / 86_400)
    }
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn fmt_signed(n: isize) -> String {
    if n < 0 {
        format!("-{}", fmt_num(n.unsigned_abs()))
    } else {
        fmt_num(n as usize)
    }
}

fn serialize_event(e: &Event) -> String {
    let jstr = crate::json::json_string;
    let range_json = match &e.range {
        Some(r) => jstr(r),
        None => "null".to_string(),
    };
    let session_json = match &e.session {
        Some(s) => jstr(s),
        None => "null".to_string(),
    };
    format!(
        "{{\"ts\":{},\"cmd\":{},\"agent\":{},\"session\":{},\"provider\":{},\"model\":{},\"path\":{},\"lang\":{},\"req_mode\":{},\"eff_mode\":{},\"range\":{},\"total_lines\":{},\"raw_bytes\":{},\"out_bytes\":{},\"raw_tokens\":{},\"out_tokens\":{}}}",
        e.ts,
        jstr(&e.cmd),
        jstr(&e.agent),
        session_json,
        jstr(&e.provider),
        jstr(&e.model),
        jstr(&e.path),
        jstr(&e.lang),
        jstr(&e.req_mode),
        jstr(&e.eff_mode),
        range_json,
        e.total_lines,
        e.raw_bytes,
        e.out_bytes,
        e.raw_tokens,
        e.out_tokens
    )
}

fn parse_event(line: &str) -> Option<Event> {
    use crate::json::{JsonParser, JsonValue};
    let obj = JsonParser::parse(line.trim()).ok()?;
    Some(Event {
        ts: obj.get("ts").and_then(JsonValue::as_u64)?,
        cmd: obj.get("cmd").and_then(JsonValue::as_str)?.to_string(),
        agent: match obj
            .get("agent")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
        {
            "antigravity" => "agy".to_string(),
            other => other.to_string(),
        },
        session: obj
            .get("session")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        provider: obj
            .get("provider")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
            .to_string(),
        model: obj
            .get("model")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
            .to_string(),
        path: obj.get("path").and_then(JsonValue::as_str)?.to_string(),
        lang: obj.get("lang").and_then(JsonValue::as_str)?.to_string(),
        req_mode: obj.get("req_mode").and_then(JsonValue::as_str)?.to_string(),
        eff_mode: obj.get("eff_mode").and_then(JsonValue::as_str)?.to_string(),
        range: obj
            .get("range")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        total_lines: obj.get("total_lines").and_then(JsonValue::as_u64)? as usize,
        raw_bytes: obj.get("raw_bytes").and_then(JsonValue::as_u64)? as usize,
        out_bytes: obj.get("out_bytes").and_then(JsonValue::as_u64)? as usize,
        raw_tokens: obj.get("raw_tokens").and_then(JsonValue::as_u64)? as usize,
        out_tokens: obj.get("out_tokens").and_then(JsonValue::as_u64)? as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_language_token_estimation_scales_as_expected() {
        assert_eq!(estimate_tokens_for_lang(0, "json"), 0);
        assert_eq!(estimate_tokens_for_lang(0, "markdown"), 0);

        let bytes = 3_800;
        let json_tok = estimate_tokens_for_lang(bytes, "json");
        let rust_tok = estimate_tokens_for_lang(bytes, "rust");
        let md_tok = estimate_tokens_for_lang(bytes, "markdown");
        assert!(json_tok > rust_tok, "json={json_tok} rust={rust_tok}");
        assert!(rust_tok > md_tok, "rust={rust_tok} md={md_tok}");
        assert_eq!(rust_tok, 1_000);
    }

    #[test]
    fn recommendations_flag_high_miss_rate_language_and_rg_block() {
        let mut events = Vec::new();
        for i in 0..5 {
            events.push(Event {
                ts: 100 + i,
                cmd: "read".into(),
                agent: "codex".into(),
                session: None,
                provider: "openai".into(),
                model: "gpt-5".into(),
                path: format!("/tmp/f{i}.swift"),
                lang: "swift".into(),
                req_mode: "auto".into(),
                eff_mode: "outline".into(),
                range: None,
                total_lines: 400,
                raw_bytes: 8000,
                out_bytes: 800,
                raw_tokens: 2000,
                out_tokens: 200,
            });
            events.push(Event {
                ts: 200 + i,
                cmd: "rg".into(),
                agent: "codex".into(),
                session: None,
                provider: "openai".into(),
                model: "gpt-5".into(),
                path: format!("q{i}"),
                lang: "search".into(),
                req_mode: "auto".into(),
                eff_mode: "block".into(),
                range: None,
                total_lines: 20,
                raw_bytes: 1000,
                out_bytes: 300,
                raw_tokens: 250,
                out_tokens: 75,
            });
        }
        let misses = vec![TrimMiss {
            ts: 101,
            cmd: "read".into(),
            path: "/tmp/f0.swift".into(),
            agent: "codex".into(),
            model_key: "openai/gpt-5".into(),
            first_mode: "outline".into(),
            followup_mode: "raw".into(),
            followup_range: None,
            total_lines: 400,
            delta_secs: 10,
            wasted_tokens: 200,
        }];
        let hints = collect_recommendations(&events, &misses, &BTreeMap::new(), &BTreeMap::new());
        assert!(
            hints
                .iter()
                .any(|h| h.contains("Language 'swift' has a 20% miss rate")),
            "got: {hints:?}"
        );
        assert!(
            hints
                .iter()
                .any(|h| h.contains("`rg --block` is working cleanly")),
            "got: {hints:?}"
        );
    }
}
