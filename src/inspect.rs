use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};

use crate::cli::InspectOptions;
use crate::json::{JsonParser, JsonValue};
use crate::style::{Theme, fmt_compact, fmt_compact_signed};

pub fn run(opts: &InspectOptions) -> Result<(), String> {
    let raw = if opts.path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        buf
    } else {
        fs::read_to_string(&opts.path).map_err(|e| format!("cannot read '{}': {e}", opts.path))?
    };

    let theme = Theme::detect();
    let rendered = render_report(&raw, &theme)?;
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))
}

fn render_report(raw: &str, theme: &Theme) -> Result<String, String> {
    let root = JsonParser::parse(raw).map_err(|e| format!("invalid JSON report: {e}"))?;
    if root.as_object().is_none()
        || (root.get("stats").is_none() && root.get("trm_version").is_none())
    {
        return Err("JSON does not look like a 'trm export' report".into());
    }

    let version = root
        .get("trm_version")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let os = root.get("os").and_then(|v| v.as_str()).unwrap_or("?");
    let arch = root.get("arch").and_then(|v| v.as_str()).unwrap_or("?");
    let redacted = root
        .get("redacted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let since_label = match root.get("since_secs").and_then(|v| v.as_u64()) {
        Some(s) => fmt_window(s),
        None => "all".to_string(),
    };

    let mut out = String::with_capacity(2048);
    out.push_str(&theme.header_rule(
        &format!("trm inspect — report v{version} ({os}/{arch})"),
        &format!(
            "window: {since_label}, {}",
            if redacted { "redacted" } else { "unredacted" }
        ),
    ));
    out.push('\n');

    if let Some(agents) = root.get("installed_agents").and_then(|v| v.as_object())
        && !agents.is_empty()
    {
        let installed: Vec<String> = agents
            .iter()
            .filter(|(_, st)| st.as_str().is_some_and(|s| s == "installed"))
            .map(|(k, _)| k.clone())
            .collect();
        let summary = if installed.is_empty() {
            "none installed".to_string()
        } else {
            installed.join(", ")
        };
        out.push_str(&format!("  Agents: {summary}\n"));
    }

    if let Some(stats) = root.get("stats") {
        let events = stats.get("events").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let raw_tok = stats
            .get("raw_tokens_est")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let out_tok = stats
            .get("out_tokens_est")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let gross_saved = stats
            .get("gross_saved_tokens_est")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let gross_pct = stats
            .get("gross_saved_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let net_saved = stats
            .get("net_saved_tokens_est")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as isize;
        let net_pct = stats
            .get("net_saved_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let misses = stats
            .get("trim_misses")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let wasted = stats
            .get("wasted_tokens_est")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let drilldowns = stats
            .get("outline_drilldowns")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let lp_count = stats
            .get("large_passthroughs_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        out.push_str(&format!("\n{}\n", theme.bold("Summary")));
        out.push_str(&format!(
            "  Events: {events}   Tokens: {} -> {}\n",
            fmt_compact(raw_tok),
            fmt_compact(out_tok)
        ));
        out.push_str(&format!(
            "  Gross saved: {} ({})   Net saved: {} ({})\n",
            fmt_compact(gross_saved),
            theme.pct_color(gross_pct, &format!("{gross_pct:.1}%")),
            fmt_compact_signed(net_saved),
            theme.pct_color(net_pct, &format!("{net_pct:.1}%")),
        ));
        out.push_str(&format!(
            "  Misses: {misses} (~{} wasted)   Drill-downs: {drilldowns}   Large passthroughs: {lp_count}\n",
            fmt_compact(wasted)
        ));
    }

    if let Some(audit) = root.get("audit") {
        let files = audit
            .get("files_scanned")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let adopted = audit.get("adopted").and_then(|v| v.as_u64()).unwrap_or(0);
        let bypasses = audit.get("bypasses").and_then(|v| v.as_u64()).unwrap_or(0);
        let adoption_pct = audit
            .get("adoption_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        out.push_str(&format!(
            "  Audit: {} adoption ({adopted} adopted / {bypasses} bypasses across {files} session logs)\n",
            theme.pct_color(adoption_pct, &format!("{adoption_pct:.1}%")),
        ));
    }

    if let Some(stats) = root.get("stats") {
        render_bucket_section(&mut out, "By mode", stats.get("by_mode"), theme);
        render_bucket_section(&mut out, "By agent", stats.get("by_agent"), theme);
        render_bucket_section(&mut out, "By lang", stats.get("by_lang"), theme);
    }

    if let Some(misses) = root.get("misses").and_then(|v| v.as_array())
        && !misses.is_empty()
    {
        let mut sorted: Vec<&JsonValue> = misses.iter().collect();
        sorted.sort_by_key(|m| {
            std::cmp::Reverse(
                m.get("wasted_tokens_est")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            )
        });
        out.push_str(&format!(
            "\n{}\n",
            theme.bold(&format!("Top misses ({} total)", misses.len()))
        ));
        for m in sorted.into_iter().take(10) {
            let wasted = m
                .get("wasted_tokens_est")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let agent = m.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let first = m.get("first_mode").and_then(|v| v.as_str()).unwrap_or("?");
            let follow = m
                .get("followup_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let range = m.get("followup_range").and_then(|v| v.as_str());
            let lines = m.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(0);
            let delta = m.get("delta_secs").and_then(|v| v.as_u64()).unwrap_or(0);
            let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let range_str = range.map(|r| format!(" ({r})")).unwrap_or_default();
            out.push_str(&format!(
                "  -{:<6} [{agent}] {first} -> {follow}{range_str}, {lines}L in {delta}s  {path}\n",
                fmt_compact(wasted),
            ));
        }
    }

    if let Some(bypasses) = root.get("bypasses").and_then(|v| v.as_array())
        && !bypasses.is_empty()
    {
        let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        for b in bypasses {
            let kind = b.get("kind").and_then(|v| v.as_str()).unwrap_or("other");
            *by_kind.entry(kind).or_insert(0) += 1;
        }
        let mut kind_counts: Vec<(&str, usize)> = by_kind.into_iter().collect();
        kind_counts.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        let summary = kind_counts
            .iter()
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join(", ");

        out.push_str(&format!(
            "\n{} ({summary})\n",
            theme.bold(&format!("Bypasses ({} total)", bypasses.len()))
        ));
        for b in bypasses.iter().take(10) {
            let agent = b.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = b.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let cmd = b.get("cmd").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!("  [{agent}/{kind}] {}\n", truncate_chars(cmd, 90)));
        }
    }

    if let Some(lps) = root.get("large_passthroughs").and_then(|v| v.as_array())
        && !lps.is_empty()
    {
        let mut sorted: Vec<&JsonValue> = lps.iter().collect();
        sorted.sort_by_key(|lp| {
            std::cmp::Reverse(
                lp.get("raw_tokens_est")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            )
        });
        out.push_str(&format!(
            "\n{}\n",
            theme.bold(&format!("Top large passthroughs ({} total)", lps.len()))
        ));
        for lp in sorted.into_iter().take(10) {
            let tok = lp
                .get("raw_tokens_est")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let lang = lp.get("lang").and_then(|v| v.as_str()).unwrap_or("?");
            let lines = lp.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(0);
            let path = lp.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!(
                "  {:<6} [{lang}, {lines}L] {path}\n",
                fmt_compact(tok)
            ));
        }
    }

    Ok(out)
}

fn render_bucket_section(out: &mut String, title: &str, val: Option<&JsonValue>, theme: &Theme) {
    let Some(entries) = val.and_then(|v| v.as_object()) else {
        return;
    };
    if entries.is_empty() {
        return;
    }

    let mut rows: Vec<(&str, usize, usize, usize, f64)> = entries
        .iter()
        .map(|(k, b)| {
            let ev = b.get("events").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let raw = b
                .get("raw_tokens_est")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let saved = b
                .get("saved_tokens_est")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let pct = b.get("saved_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (k.as_str(), ev, raw, saved, pct)
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));

    out.push_str(&format!("\n{}\n", theme.bold(title)));
    for (name, ev, raw, saved, pct) in rows.into_iter().take(8) {
        out.push_str(&format!(
            "  {:<14} {:>4} ev   {:>6} raw   {:>6} saved ({})\n",
            truncate_chars(name, 14),
            ev,
            fmt_compact(raw),
            fmt_compact(saved),
            theme.pct_color(pct, &format!("{pct:.1}%")),
        ));
    }
}

fn fmt_window(secs: u64) -> String {
    match secs {
        3_600 => "1h".into(),
        86_400 => "24h".into(),
        604_800 => "7d".into(),
        2_592_000 => "30d".into(),
        s if s.is_multiple_of(86_400) => format!("{}d", s / 86_400),
        s if s.is_multiple_of(3_600) => format!("{}h", s / 3_600),
        s => format!("{s}s"),
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let byte_end = s.char_indices().nth(keep).map_or(s.len(), |(i, _)| i);
    format!("{}…", &s[..byte_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_export_report_summary() {
        let report = r#"{
            "trm_version": "0.1.0",
            "os": "macos",
            "arch": "aarch64",
            "redacted": true,
            "since_secs": 604800,
            "installed_agents": {"codex": "installed", "claude": "missing"},
            "stats": {
                "events": 12,
                "raw_tokens_est": 4000,
                "out_tokens_est": 1000,
                "gross_saved_tokens_est": 3000,
                "gross_saved_pct": 75.0,
                "net_saved_tokens_est": 2700,
                "net_saved_pct": 67.5,
                "trim_misses": 1,
                "wasted_tokens_est": 300,
                "outline_drilldowns": 2,
                "large_passthroughs_count": 1,
                "by_mode": {"outline": {"events": 5, "raw_tokens_est": 3000, "out_tokens_est": 500, "saved_tokens_est": 2500, "saved_pct": 83.3}},
                "by_agent": {"antigravity": {"events": 12, "raw_tokens_est": 4000, "out_tokens_est": 1000, "saved_tokens_est": 3000, "saved_pct": 75.0}},
                "by_lang": {"rust": {"events": 12, "raw_tokens_est": 4000, "out_tokens_est": 1000, "saved_tokens_est": 3000, "saved_pct": 75.0}}
            },
            "audit": {"files_scanned": 3, "adopted": 18, "bypasses": 2, "adoption_pct": 90.0},
            "misses": [{"ts": 1, "agent": "codex", "first_mode": "minimal", "followup_mode": "raw", "followup_range": null, "total_lines": 120, "delta_secs": 15, "wasted_tokens_est": 300, "path": "~/app/foo.rs"}],
            "bypasses": [{"ts": 2, "agent": "codex", "kind": "cat", "cmd": "cat README.md"}],
            "large_passthroughs": [{"ts": 3, "agent": "antigravity", "lang": "markdown", "total_lines": 140, "raw_tokens_est": 950, "path": "~/app/NOTES.md"}]
        }"#;
        let out = render_report(report, &Theme { color: false }).unwrap();
        assert!(out.contains("trm inspect — report v0.1.0 (macos/aarch64)"));
        assert!(out.contains("window: 7d, redacted"));
        assert!(out.contains("Agents: codex"));
        assert!(out.contains("Events: 12"));
        assert!(out.contains("Gross saved: 3.0K (75.0%)"));
        assert!(out.contains("90.0% adoption (18 adopted / 2 bypasses"));
        assert!(out.contains("Top misses (1 total)"));
        assert!(out.contains("~/app/foo.rs"));
        assert!(out.contains("Bypasses (1 total) (cat:1)"));
        assert!(out.contains("Top large passthroughs (1 total)"));
    }
}
