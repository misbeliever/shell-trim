use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};

use crate::audit;
use crate::cli::ExportOptions;
use crate::config::Config;
use crate::doctor;
use crate::telemetry::{self, Bucket};

pub fn run(opts: &ExportOptions) -> Result<(), String> {
    let cfg = Config::load();
    let stats = telemetry::load_snapshot(opts.since_secs);
    let audit_sum = audit::load_summary(opts.since_secs);
    let agents = doctor::installed_agents();
    let home = env::var("TRM_HOME")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_default();

    let clean = |s: &str| -> String {
        if opts.redact {
            redact_text(s, &home)
        } else {
            s.to_string()
        }
    };

    let mut out = String::with_capacity(8192);
    out.push_str("{\n");
    out.push_str(&format!("  \"trm_version\": {},\n", jstr(env!("CARGO_PKG_VERSION"))));
    out.push_str(&format!("  \"os\": {},\n", jstr(env::consts::OS)));
    out.push_str(&format!("  \"arch\": {},\n", jstr(env::consts::ARCH)));
    out.push_str(&format!("  \"redacted\": {},\n", opts.redact));
    match opts.since_secs {
        Some(s) => out.push_str(&format!("  \"since_secs\": {s},\n")),
        None => out.push_str("  \"since_secs\": null,\n"),
    }

    // Config
    out.push_str("  \"config\": {\n");
    out.push_str(&format!("    \"small_lines\": {},\n", cfg.small_lines));
    out.push_str(&format!("    \"outline_lines\": {},\n", cfg.outline_lines));
    out.push_str(&format!("    \"reread_window_secs\": {},\n", cfg.reread_window_secs));
    out.push_str(&format!("    \"rg_max_total\": {},\n", cfg.rg_max_total));
    out.push_str(&format!("    \"rg_per_file\": {},\n", cfg.rg_per_file));
    out.push_str(&format!("    \"rg_max_chars\": {}\n", cfg.rg_max_chars));
    out.push_str("  },\n");

    // Installed agents
    out.push_str("  \"installed_agents\": {\n");
    for (idx, (agent, status)) in agents.iter().enumerate() {
        let comma = if idx + 1 < agents.len() { "," } else { "" };
        out.push_str(&format!("    {}: {}{comma}\n", jstr(agent), jstr(status)));
    }
    out.push_str("  },\n");

    // Stats summary
    out.push_str("  \"stats\": {\n");
    out.push_str(&format!("    \"events\": {},\n", stats.events_count));
    out.push_str(&format!("    \"raw_tokens_est\": {},\n", stats.raw_tokens));
    out.push_str(&format!("    \"out_tokens_est\": {},\n", stats.out_tokens));
    out.push_str(&format!("    \"gross_saved_tokens_est\": {},\n", stats.gross_saved));
    out.push_str(&format!("    \"gross_saved_pct\": {:.1},\n", stats.gross_pct));
    out.push_str(&format!("    \"net_saved_tokens_est\": {},\n", stats.net_saved));
    out.push_str(&format!("    \"net_saved_pct\": {:.1},\n", stats.net_pct));
    out.push_str(&format!("    \"trim_misses\": {},\n", stats.misses_count));
    out.push_str(&format!("    \"wasted_tokens_est\": {},\n", stats.wasted_tokens));
    out.push_str(&format!("    \"outline_drilldowns\": {},\n", stats.drilldowns));
    out.push_str(&format!(
        "    \"large_passthroughs_count\": {},\n",
        stats.large_passthroughs.len()
    ));
    out.push_str("    \"by_mode\": ");
    push_buckets_json(&mut out, &stats.by_mode, 4);
    out.push_str(",\n");
    out.push_str("    \"by_agent\": ");
    push_buckets_json(&mut out, &stats.by_agent, 4);
    out.push_str(",\n");
    out.push_str("    \"by_model\": ");
    push_buckets_json(&mut out, &stats.by_model, 4);
    out.push_str(",\n");
    out.push_str("    \"by_lang\": ");
    push_buckets_json(&mut out, &stats.by_lang, 4);
    out.push('\n');
    out.push_str("  },\n");

    // Audit summary
    out.push_str("  \"audit\": {\n");
    out.push_str(&format!("    \"files_scanned\": {},\n", audit_sum.files_scanned));
    out.push_str(&format!("    \"adopted\": {},\n", audit_sum.adopted));
    out.push_str(&format!("    \"bypasses\": {},\n", audit_sum.bypasses));
    out.push_str(&format!("    \"adoption_pct\": {:.1},\n", audit_sum.adoption_pct));
    out.push_str("    \"by_agent\": ");
    push_audit_rows_json(&mut out, &audit_sum.by_agent, 4);
    out.push_str(",\n");
    out.push_str("    \"by_model\": ");
    push_audit_rows_json(&mut out, &audit_sum.by_model, 4);
    out.push('\n');
    out.push_str("  },\n");

    // Full untruncated trim misses
    out.push_str("  \"misses\": [\n");
    for (idx, m) in stats.misses.iter().enumerate() {
        let comma = if idx + 1 < stats.misses.len() { "," } else { "" };
        let range_val = match m.followup_range.as_deref() {
            Some(r) => jstr(r),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "    {{\"ts\": {}, \"agent\": {}, \"model\": {}, \"cmd\": {}, \"path\": {}, \"first_mode\": {}, \"followup_mode\": {}, \"followup_range\": {}, \"total_lines\": {}, \"delta_secs\": {}, \"wasted_tokens_est\": {}}}{comma}\n",
            m.ts,
            jstr(&m.agent),
            jstr(&m.model_key),
            jstr(&m.cmd),
            jstr(&clean(&m.path)),
            jstr(&m.first_mode),
            jstr(&m.followup_mode),
            range_val,
            m.total_lines,
            m.delta_secs,
            m.wasted_tokens,
        ));
    }
    out.push_str("  ],\n");

    // Full untruncated bypassed legacy commands
    out.push_str("  \"bypasses\": [\n");
    for (idx, b) in audit_sum.bypass_samples.iter().enumerate() {
        let comma = if idx + 1 < audit_sum.bypass_samples.len() {
            ","
        } else {
            ""
        };
        let ts_val = match b.ts {
            Some(t) => t.to_string(),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "    {{\"ts\": {}, \"agent\": {}, \"model\": {}, \"kind\": {}, \"cmd\": {}}}{comma}\n",
            ts_val,
            jstr(&b.agent),
            jstr(&b.model),
            jstr(b.kind),
            jstr(&clean(&b.cmd)),
        ));
    }
    out.push_str("  ],\n");

    // Large auto passthroughs (>= 300 tok, non-slice) — potential missed trimming opportunities
    out.push_str("  \"large_passthroughs\": [\n");
    for (idx, lp) in stats.large_passthroughs.iter().enumerate() {
        let comma = if idx + 1 < stats.large_passthroughs.len() {
            ","
        } else {
            ""
        };
        out.push_str(&format!(
            "    {{\"ts\": {}, \"agent\": {}, \"model\": {}, \"cmd\": {}, \"lang\": {}, \"path\": {}, \"total_lines\": {}, \"raw_tokens_est\": {}}}{comma}\n",
            lp.ts,
            jstr(&lp.agent),
            jstr(&lp.model_key),
            jstr(&lp.cmd),
            jstr(&lp.lang),
            jstr(&clean(&lp.path)),
            lp.total_lines,
            lp.raw_tokens,
        ));
    }
    out.push_str("  ]\n");
    out.push_str("}\n");

    if let Some(ref path) = opts.out_path {
        fs::write(path, out.as_bytes()).map_err(|e| format!("cannot write '{path}': {e}"))?;
        eprintln!(
            "Exported {} misses, {} bypasses, and {} large passthroughs to {path}",
            stats.misses.len(),
            audit_sum.bypass_samples.len(),
            stats.large_passthroughs.len()
        );
    } else {
        io::stdout()
            .write_all(out.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
    }

    Ok(())
}

fn push_buckets_json(out: &mut String, map: &BTreeMap<String, Bucket>, indent: usize) {
    if map.is_empty() {
        out.push_str("{}");
        return;
    }
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    out.push_str("{\n");
    let len = map.len();
    for (idx, (k, b)) in map.iter().enumerate() {
        let comma = if idx + 1 < len { "," } else { "" };
        out.push_str(&format!(
            "{inner}{}: {{\"reads\": {}, \"raw_tokens_est\": {}, \"out_tokens_est\": {}, \"misses\": {}, \"wasted_tokens_est\": {}, \"drilldowns\": {}}}{comma}\n",
            jstr(k),
            b.reads,
            b.raw_tokens,
            b.out_tokens,
            b.misses,
            b.wasted_tokens,
            b.drilldowns
        ));
    }
    out.push_str(&format!("{pad}}}"));
}

fn push_audit_rows_json(out: &mut String, rows: &[(String, usize, usize, f64)], indent: usize) {
    if rows.is_empty() {
        out.push_str("[]");
        return;
    }
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    out.push_str("[\n");
    for (idx, (name, adopted, total, pct)) in rows.iter().enumerate() {
        let comma = if idx + 1 < rows.len() { "," } else { "" };
        out.push_str(&format!(
            "{inner}{{\"name\": {}, \"adopted\": {}, \"total\": {}, \"adoption_pct\": {:.1}}}{comma}\n",
            jstr(name),
            adopted,
            total,
            pct
        ));
    }
    out.push_str(&format!("{pad}]"));
}

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Redacts `$HOME` prefixes and common inline secret patterns (Bearer tokens, API keys,
/// `*_KEY=...` / `*_TOKEN=...` / `*_SECRET=...` / `PASSWORD=...`) for safe report sharing.
pub(crate) fn redact_text(input: &str, home: &str) -> String {
    let mut s = if !home.is_empty() && home != "/" && home != "." {
        input.replace(home, "~")
    } else {
        input.to_string()
    };

    // Mask `Bearer <token>`
    s = redact_after_marker(&s, "Bearer ");
    // Mask known token prefixes
    for prefix in [
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AIza",
    ] {
        s = redact_prefixed_token(&s, prefix, 12);
    }
    // Mask `KEY=...` / `TOKEN=...` / `SECRET=...` / `PASSWORD=...`
    s = redact_env_assignments(&s);
    s
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+' | b'/' | b'=')
}

fn redact_after_marker(s: &str, marker: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(marker) {
        out.push_str(&rest[..idx + marker.len()]);
        let after = &rest[idx + marker.len()..];
        let tok_len = after.bytes().take_while(|&b| is_token_char(b)).count();
        if tok_len >= 8 {
            out.push_str("[REDACTED]");
            rest = &after[tok_len..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn redact_prefixed_token(s: &str, prefix: &str, min_tail: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(prefix) {
        let prev_ok = idx == 0 || !rest.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after = &rest[idx + prefix.len()..];
        let tail_len = after.bytes().take_while(|&b| is_token_char(b)).count();
        if prev_ok && tail_len >= min_tail {
            out.push_str(&rest[..idx]);
            out.push_str("[REDACTED]");
            rest = &after[tail_len..];
        } else {
            out.push_str(&rest[..idx + prefix.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn redact_env_assignments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &s[start..i];
            if i < len && bytes[i] == b'=' && is_secret_env_name(ident) {
                out.push_str(ident);
                out.push('=');
                i += 1; // skip '='
                // Consume optional quote + value
                let quote = if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let q = bytes[i];
                    out.push(q as char);
                    i += 1;
                    Some(q)
                } else {
                    None
                };
                let val_start = i;
                while i < len {
                    if let Some(q) = quote {
                        if bytes[i] == q {
                            break;
                        }
                    } else if bytes[i].is_ascii_whitespace() || matches!(bytes[i], b';' | b'&' | b'|') {
                        break;
                    }
                    i += 1;
                }
                if i > val_start {
                    out.push_str("[REDACTED]");
                }
                if let Some(q) = quote
                    && i < len
                    && bytes[i] == q
                {
                    out.push(q as char);
                    i += 1;
                }
                continue;
            }
            out.push_str(ident);
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with("_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper == "PASSWORD"
        || upper.ends_with("_PASSWORD")
        || upper == "API_KEY"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_and_inline_secrets() {
        let input = "OPENAI_API_KEY=sk-1234567890abcdef curl -H 'Authorization: Bearer tok_abcdefgh1234' /Users/alice/app";
        let out = redact_text(input, "/Users/alice");
        assert!(out.contains("OPENAI_API_KEY=[REDACTED]"), "got: {out}");
        assert!(out.contains("Bearer [REDACTED]"), "got: {out}");
        assert!(out.contains("~/app"), "got: {out}");
        assert!(!out.contains("1234567890"), "leaked key: {out}");
        assert!(!out.contains("/Users/alice"), "leaked home: {out}");
    }
}
