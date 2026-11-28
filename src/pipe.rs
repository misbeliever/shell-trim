use std::collections::HashSet;
use std::io::{self, Read, Write};

use crate::cli::PipeOptions;
use crate::json::{JsonParser, JsonValue};
use crate::telemetry;

pub fn run(opts: &PipeOptions) -> Result<(), String> {
    let mut raw_bytes_vec = Vec::new();
    io::stdin()
        .read_to_end(&mut raw_bytes_vec)
        .map_err(|e| format!("failed to read stdin: {e}"))?;

    if raw_bytes_vec.is_empty() {
        return Ok(());
    }

    let raw_bytes = raw_bytes_vec.len();
    let sniff = raw_bytes.min(8192);
    let is_binary = raw_bytes_vec[..sniff].contains(&0);

    if opts.raw {
        io::stdout()
            .write_all(&raw_bytes_vec)
            .map_err(|e| format!("write failed: {e}"))?;
        if opts.log_stats {
            let total_lines = raw_bytes_vec.iter().filter(|&&b| b == b'\n').count().max(1);
            let kind = if is_binary { "binary" } else { "text" };
            telemetry::log_cmd_event(
                "pipe",
                &format!("pipe:{kind}"),
                kind,
                "raw",
                "raw",
                total_lines,
                raw_bytes,
                raw_bytes,
            );
        }
        return Ok(());
    }

    if is_binary {
        let msg = format!("[trm: binary stdin ({raw_bytes} B) — skipped]\n");
        io::stdout()
            .write_all(msg.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
        return Ok(());
    }

    let raw_str = String::from_utf8_lossy(&raw_bytes_vec).into_owned();
    let total_lines = raw_str.lines().count().max(1);

    let (rendered, kind, eff_mode) = trim_piped(&raw_str, opts);
    let (out_bytes, final_mode) = if rendered.len() >= raw_bytes {
        io::stdout()
            .write_all(&raw_bytes_vec)
            .map_err(|e| format!("write failed: {e}"))?;
        (raw_bytes, "raw")
    } else {
        io::stdout()
            .write_all(rendered.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
        (rendered.len(), eff_mode)
    };

    if opts.log_stats {
        let req_mode = if opts.raw {
            "raw"
        } else if opts.all {
            "all"
        } else {
            "auto"
        };
        telemetry::log_cmd_event(
            "pipe",
            &format!("pipe:{kind}"),
            kind,
            req_mode,
            final_mode,
            total_lines,
            raw_bytes,
            out_bytes,
        );
    }

    Ok(())
}

pub(crate) fn trim_piped(raw: &str, opts: &PipeOptions) -> (String, &'static str, &'static str) {
    let trimmed_start = raw.trim();
    if (trimmed_start.starts_with('{') || trimmed_start.starts_with('['))
        && let Ok(val) = JsonParser::parse(trimmed_start)
    {
        let line_cnt = raw.lines().count();
        if raw.len() > 2048 || line_cnt > 40 {
            let summary = summarize_json_root(&val, raw.len());
            return (summary, "json", "summary");
        }
        return (raw.to_string(), "json", "raw");
    }

    let lines: Vec<&str> = raw.lines().collect();
    if let Some(table) = try_trim_table(&lines, opts.all) {
        return (table, "sql", "trimmed");
    }

    let log_trimmed = trim_logs(&lines, opts);
    if log_trimmed.len() < raw.len() {
        (log_trimmed, "text", "trimmed")
    } else {
        (raw.to_string(), "text", "raw")
    }
}

fn summarize_json_root(val: &JsonValue, raw_bytes: usize) -> String {
    let mut out = String::new();
    match val {
        JsonValue::Array(items) => {
            out.push_str(&format!("array[{} items]:\n", items.len()));
            for (idx, item) in items.iter().take(2).enumerate() {
                out.push_str(&format!("  [{idx}] {}\n", format_json_brief(item, 0)));
            }
            if items.len() > 2 {
                out.push_str(&format!("  ... (+{} more items)\n", items.len() - 2));
            }
        }
        JsonValue::Object(entries) => {
            out.push_str(&format!("object ({} keys):\n", entries.len()));
            for (k, v) in entries.iter().take(24) {
                out.push_str(&format!("  \"{k}\": {}\n", format_json_brief(v, 1)));
            }
            if entries.len() > 24 {
                out.push_str(&format!("  ... (+{} more keys)\n", entries.len() - 24));
            }
        }
        _ => out.push_str(&format!("{}\n", format_json_brief(val, 0))),
    }
    out.push_str(&format!(
        "[trm: json summary ({raw_bytes} B) — use --raw for full]\n"
    ));
    out
}

fn format_json_brief(val: &JsonValue, depth: usize) -> String {
    match val {
        JsonValue::Null => "null".into(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Num(_) => val.to_compact_string(),
        JsonValue::Str(s) => {
            if s.chars().count() > 60 {
                let end = s.char_indices().nth(57).map_or(s.len(), |(i, _)| i);
                format!("\"{}…\"", &s[..end])
            } else {
                format!("\"{s}\"")
            }
        }
        JsonValue::Array(items) => {
            if depth >= 2 || items.is_empty() {
                format!("[{} items]", items.len())
            } else {
                let first = format_json_brief(&items[0], depth + 1);
                format!("[{} items, sample: {first}]", items.len())
            }
        }
        JsonValue::Object(entries) => {
            if depth >= 2 || entries.is_empty() {
                format!("{{{} keys}}", entries.len())
            } else {
                let preview: Vec<String> = entries
                    .iter()
                    .take(5)
                    .map(|(k, v)| format!("{k}: {}", format_json_brief(v, depth + 1)))
                    .collect();
                let more = if entries.len() > 5 {
                    format!(", +{} more", entries.len() - 5)
                } else {
                    String::new()
                };
                format!("{{{}{more}}}", preview.join(", "))
            }
        }
    }
}

fn try_trim_table(lines: &[&str], show_all: bool) -> Option<String> {
    if show_all || lines.len() < 25 {
        return None;
    }

    // MySQL/psql box table: `+---+` borders and `| ... |` rows
    let border_count = lines
        .iter()
        .filter(|l| l.starts_with('+') && l.contains("---"))
        .count();
    let pipe_rows: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('|') && l.ends_with('|'))
        .map(|(i, _)| i)
        .collect();
    if border_count >= 2 && pipe_rows.len() >= 18 {
        let mut out: Vec<String> = Vec::new();
        let header_end = pipe_rows[0] + 2; // top border + header + separator border
        for l in lines.iter().take(header_end.min(lines.len())) {
            out.push((*l).to_string());
        }
        let body_start = header_end;
        let data_rows: Vec<&str> = lines[body_start..]
            .iter()
            .copied()
            .filter(|l| l.starts_with('|'))
            .collect();
        if data_rows.len() > 15 {
            for r in data_rows.iter().take(10) {
                out.push((*r).to_string());
            }
            out.push(format!(
                "| ... (+{} rows omitted) ... |",
                data_rows.len() - 13
            ));
            for r in &data_rows[data_rows.len() - 3..] {
                out.push((*r).to_string());
            }
            if let Some(last_border) = lines.iter().rev().find(|l| l.starts_with('+')) {
                out.push((*last_border).to_string());
            }
            out.push(format!(
                "[trm: table ({} rows -> 13 shown) — use --raw or --all for full]",
                data_rows.len()
            ));
            let mut s = out.join("\n");
            s.push('\n');
            return Some(s);
        }
    }

    // TSV table (>= 25 lines, >= 2 tabs in header and sample rows)
    let header_tabs = lines[0].matches('\t').count();
    if header_tabs >= 2
        && lines[1..6]
            .iter()
            .all(|l| l.matches('\t').count() == header_tabs)
    {
        let total_rows = lines.len() - 1;
        let mut out = Vec::with_capacity(16);
        out.push(lines[0].to_string());
        for l in &lines[1..11] {
            out.push((*l).to_string());
        }
        out.push(format!("... (+{} rows omitted) ...", total_rows - 13));
        for l in &lines[lines.len() - 3..] {
            out.push((*l).to_string());
        }
        out.push(format!(
            "[trm: tsv ({total_rows} rows -> 13 shown) — use --raw or --all for full]"
        ));
        let mut s = out.join("\n");
        s.push('\n');
        return Some(s);
    }

    None
}

fn trim_logs(lines: &[&str], opts: &PipeOptions) -> String {
    // Pass 1: collapse consecutive duplicate lines after volatile timestamp/hex normalization
    let mut collapsed: Vec<(String, usize, bool)> = Vec::new();
    let mut prev_norm: Option<String> = None;

    for raw_line in lines {
        let norm = normalize_log_line(raw_line);
        let important = is_important_log_line(raw_line);
        if !important
            && prev_norm.as_deref() == Some(norm.as_str())
            && let Some(last) = collapsed.last_mut()
        {
            last.1 += 1;
            continue;
        }
        prev_norm = Some(norm);
        collapsed.push(((*raw_line).to_string(), 1, important));
    }

    let formatted: Vec<(String, bool)> = collapsed
        .into_iter()
        .map(|(line, cnt, imp)| {
            if cnt > 1 {
                (format!("{line} (x{cnt})"), imp)
            } else {
                (line, imp)
            }
        })
        .collect();

    let cap = opts.max_lines.unwrap_or(120);
    if opts.all || formatted.len() <= cap {
        let mut s = formatted
            .into_iter()
            .map(|(l, _)| l)
            .collect::<Vec<_>>()
            .join("\n");
        if formatted_changed(lines.len(), &s) {
            s.push_str(&format!(
                "\n[trm: collapsed duplicate lines ({} lines total) — use --raw for full]",
                lines.len()
            ));
        }
        s.push('\n');
        return s;
    }

    let head = cap / 2;
    let tail = cap - head;
    let mid_end = formatted.len() - tail;
    let mut out = Vec::with_capacity(cap + 8);
    let mut seen_imp = HashSet::new();

    for (l, _) in &formatted[..head] {
        out.push(l.clone());
    }
    let mut middle_important = Vec::new();
    for (l, imp) in &formatted[head..mid_end] {
        if *imp && seen_imp.insert(l.clone()) {
            middle_important.push(l.clone());
        }
    }
    let omitted = mid_end - head;
    out.push(format!(
        "... ({omitted} lines omitted; preserving {} error/warn lines) ...",
        middle_important.len()
    ));
    out.extend(middle_important.into_iter().take(25));
    for (l, _) in &formatted[mid_end..] {
        out.push(l.clone());
    }
    out.push(format!(
        "[trm: piped log ({} lines -> {} lines) — use --raw or --all for full]",
        lines.len(),
        out.len()
    ));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn formatted_changed(raw_lines: usize, rendered: &str) -> bool {
    rendered.lines().count() < raw_lines
}

fn is_important_log_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("error")
        || lower.contains("warn")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.contains("traceback")
        || lower.contains("failed")
}

// ponytail: simple timestamp/hex/digit normalizer; refine if log formats need custom rules
fn normalize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_large_json_array() {
        let items: Vec<String> = (0..80)
            .map(|i| {
                format!(r#"{{"id":{i},"name":"item_{i}","active":true,"payload":"abcdef123456"}}"#)
            })
            .collect();
        let json = format!("[{}]", items.join(","));
        let (out, kind, mode) = trim_piped(&json, &PipeOptions::default());
        assert_eq!(kind, "json");
        assert_eq!(mode, "summary");
        assert!(out.contains("array[80 items]"));
        assert!(out.contains("[trm: json summary"));
        assert!(out.len() < json.len());
    }

    #[test]
    fn collapses_repeated_logs_and_preserves_errors() {
        let mut lines = Vec::new();
        for i in 0..160 {
            if i == 80 {
                lines.push("2026-11-27T10:00:00Z ERROR database connection refused".to_string());
            } else {
                lines.push(format!("2026-11-27T10:00:{:02}Z INFO heartbeat ok", i % 60));
            }
        }
        let raw = lines.join("\n");
        let (out, kind, mode) = trim_piped(&raw, &PipeOptions::default());
        assert_eq!(kind, "text");
        assert_eq!(mode, "trimmed");
        assert!(out.contains("(x80)"));
        assert!(out.contains("ERROR database connection refused"));
        assert!(out.len() < raw.len());
    }

    #[test]
    fn trims_sql_box_table() {
        let mut lines = vec![
            "+----+-------+".to_string(),
            "| id | name  |".to_string(),
            "+----+-------+".to_string(),
        ];
        for i in 0..30 {
            lines.push(format!("| {i:<2} | row{i:<2} |"));
        }
        lines.push("+----+-------+".to_string());
        let raw = lines.join("\n");
        let (out, kind, _) = trim_piped(&raw, &PipeOptions::default());
        assert_eq!(kind, "sql");
        assert!(out.contains("(+17 rows omitted)"));
        assert!(out.contains("| 29 | row29 |"));
    }

    #[test]
    fn pipe_fails_open_on_malformed_json_and_unstructured_input() {
        let broken_json = "{\"items\": [1, 2, 3, ".repeat(100);
        let (out, _, mode) = trim_piped(&broken_json, &PipeOptions::default());
        assert_eq!(out, broken_json);
        assert_eq!(mode, "raw");

        let plain = "line one\nline two\nline three\n";
        let (out2, _, mode2) = trim_piped(plain, &PipeOptions::default());
        assert_eq!(out2, plain);
        assert_eq!(mode2, "raw");
    }
}
