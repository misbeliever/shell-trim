use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::cli::SearchOptions;
use crate::config::Config;
use crate::lang::Language;
use crate::outline;
use crate::telemetry;

const DEFAULT_MAX_FILES_LIST: usize = 80;
const AUTO_BLOCK_MAX_MATCHES: usize = 3;
const BLOCK_MAX_SPAN_LINES: usize = 60;
const BLOCK_FALLBACK_RADIUS: usize = 8;
const BLOCK_TOTAL_LINE_CAP: usize = 120;

pub fn run(opts: &SearchOptions) -> Result<ExitCode, String> {
    let is_files_mode = opts.rg_args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--files" | "-l" | "--files-with-matches" | "--files-without-match"
        )
    });
    let is_count_mode = opts
        .rg_args
        .iter()
        .any(|a| matches!(a.as_str(), "-c" | "--count" | "--count-matches"));

    let mut cmd = Command::new("rg");
    cmd.stdin(Stdio::inherit());
    cmd.arg("--color=never");
    if !is_files_mode && !is_count_mode && !opts.raw {
        cmd.args(["--no-heading", "--with-filename", "--line-number"]);
    }
    cmd.args(&opts.rg_args);

    let output = cmd.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            "ripgrep ('rg') not found in PATH".to_string()
        } else {
            format!("failed to run rg: {e}")
        }
    })?;

    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(&output.stderr);
    }

    let code = output.status.code().unwrap_or(1);
    if output.stdout.is_empty() {
        return Ok(ExitCode::from(code as u8));
    }

    let raw_bytes = output.stdout.len();
    let raw_str = String::from_utf8_lossy(&output.stdout);

    let has_explicit_context = opts.rg_args.iter().any(|a| {
        matches!(
            a.as_str(),
            "-C" | "-A" | "-B" | "--context" | "--after-context" | "--before-context"
        ) || a.starts_with("-C")
            || a.starts_with("-A")
            || a.starts_with("-B")
    });

    let (rendered, eff_mode) = if opts.raw || is_count_mode {
        (raw_str.into_owned(), "raw")
    } else if is_files_mode {
        let formatted = format_files_list(&raw_str, opts.all);
        if formatted.len() >= raw_bytes {
            (raw_str.into_owned(), "raw")
        } else {
            (formatted, if opts.all { "all" } else { "files" })
        }
    } else {
        // Auto-expand to enclosing function/context blocks when 1..=3 matches (or --block forced)
        let try_block = match opts.block {
            Some(true) => true,
            Some(false) => false,
            None => !has_explicit_context && !opts.all && opts.max_chars.is_none(),
        };
        if try_block {
            if let Some(expanded) = try_expand_blocks(&raw_str, opts.block == Some(true)) {
                (expanded, "block")
            } else {
                format_grouped_or_raw(&raw_str, raw_bytes, opts)
            }
        } else {
            format_grouped_or_raw(&raw_str, raw_bytes, opts)
        }
    };

    let out_bytes = rendered.len();
    io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if opts.log_stats {
        let query_key = extract_query_key(&opts.rg_args);
        let req_mode = if opts.raw {
            "raw"
        } else if opts.all {
            "all"
        } else {
            "auto"
        };
        let total_lines = raw_str_line_count(&output.stdout);
        // `rg:block` intentionally adds enclosing context to save a follow-up `read` turn;
        // log neutral baseline (`out_bytes`) like `read:slice` so it neither inflates nor penalizes net%.
        let logged_raw = if eff_mode == "block" {
            out_bytes
        } else {
            raw_bytes
        };
        telemetry::log_search_event(
            &query_key,
            req_mode,
            eff_mode,
            total_lines,
            logged_raw,
            out_bytes,
        );
    }

    Ok(ExitCode::from(code as u8))
}

fn raw_str_line_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count().max(1)
}

fn format_grouped_or_raw(
    raw_str: &str,
    raw_bytes: usize,
    opts: &SearchOptions,
) -> (String, &'static str) {
    let cfg = Config::load();
    let max_total = if opts.all {
        usize::MAX
    } else {
        opts.max_total.unwrap_or(cfg.rg_max_total)
    };
    let per_file = if opts.all {
        usize::MAX
    } else {
        opts.per_file.unwrap_or(cfg.rg_per_file)
    };
    let max_chars = opts.max_chars.unwrap_or(cfg.rg_max_chars);
    let formatted = format_matches(raw_str, max_total, per_file, max_chars);
    if formatted.len() >= raw_bytes {
        (raw_str.to_string(), "raw")
    } else {
        (formatted, if opts.all { "all" } else { "grouped" })
    }
}

/// When a search is laser-focused (1..=3 hits, or `--block` forced), expand each hit to its
/// enclosing function/method block (or ±8 raw lines) so the agent can edit in 1 tool call.
fn try_expand_blocks(raw: &str, forced: bool) -> Option<String> {
    let mut hits: Vec<(&str, usize)> = Vec::new();
    for line in raw.lines() {
        if let Some((path, num_s, ':', _)) = parse_rg_line(line)
            && let Ok(n) = num_s.parse::<usize>()
        {
            hits.push((path, n));
        }
    }
    if hits.is_empty() {
        return None;
    }
    if !forced && hits.len() > AUTO_BLOCK_MAX_MATCHES {
        return None;
    }

    let mut out = String::new();
    let mut emitted_blocks: Vec<(String, usize, usize)> = Vec::new();
    let mut total_lines_emitted = 0usize;

    for (path, line_num) in hits.iter().take(AUTO_BLOCK_MAX_MATCHES) {
        let Ok(content) = fs::read_to_string(path) else {
            return None;
        };
        let raw_lines = crate::read::split_lines_preserving_cr(&content);
        if raw_lines.is_empty() || *line_num == 0 || *line_num > raw_lines.len() {
            continue;
        }
        let clean_lines: Vec<&str> = raw_lines
            .iter()
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        let lang = Language::detect(path, None);
        let (start, end) =
            outline::enclosing_block(&clean_lines, lang, *line_num, BLOCK_MAX_SPAN_LINES)
                .unwrap_or_else(|| {
                    let s = line_num.saturating_sub(BLOCK_FALLBACK_RADIUS).max(1);
                    let e = (*line_num + BLOCK_FALLBACK_RADIUS).min(raw_lines.len());
                    (s, e)
                });

        // Deduplicate if another hit already covered this exact block
        if emitted_blocks
            .iter()
            .any(|(p, s, e)| p == path && *s <= *line_num && *line_num <= *e)
        {
            continue;
        }

        let span = end.saturating_sub(start) + 1;
        if total_lines_emitted + span > BLOCK_TOTAL_LINE_CAP {
            return None;
        }

        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{path} [L{start}-L{end}] (match L{line_num}):\n"));
        for l in &raw_lines[start - 1..end] {
            out.push_str(l);
            out.push('\n');
        }
        total_lines_emitted += span;
        emitted_blocks.push(((*path).to_string(), start, end));
    }

    if emitted_blocks.is_empty() {
        None
    } else {
        if hits.len() > AUTO_BLOCK_MAX_MATCHES {
            out.push_str(&format!(
                "[trm: +{} more matches — pass --no-block for grouped list]\n",
                hits.len() - AUTO_BLOCK_MAX_MATCHES
            ));
        }
        Some(out)
    }
}

fn extract_query_key(args: &[String]) -> String {
    // First non-flag argument is usually the search pattern
    let mut skip_next = false;
    for a in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            a.as_str(),
            "-g" | "--glob"
                | "-t"
                | "--type"
                | "-T"
                | "--type-not"
                | "-C"
                | "-A"
                | "-B"
                | "-m"
                | "--max-count"
                | "-e"
                | "--regexp"
                | "-f"
                | "--file"
        ) {
            if a == "-e" || a == "--regexp" {
                continue;
            }
            skip_next = true;
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        return format!("rg:{a}");
    }
    "rg:<files>".to_string()
}

struct MatchLine<'a> {
    line_num: &'a str,
    sep: char, // ':' for match, '-' for context
    text: &'a str,
}

fn parse_rg_line(line: &str) -> Option<(&str, &str, char, &str)> {
    // Standard match: `path:123:text`
    // Context line:   `path-123-text`
    // Prefer ':' first since filenames rarely contain `:<digits>:`
    if let Some((path, rest)) = line.split_once(':')
        && let Some((num, text)) = rest.split_once(':')
        && !num.is_empty()
        && num.bytes().all(|b| b.is_ascii_digit())
    {
        return Some((path, num, ':', text));
    }
    // Context line fallback (`-C`/`-A`/`-B`)
    let bytes = line.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'-' && idx > 0 {
            let rest = &line[idx + 1..];
            if let Some((num, text)) = rest.split_once('-')
                && !num.is_empty()
                && num.bytes().all(|b| b.is_ascii_digit())
            {
                return Some((&line[..idx], num, '-', text));
            }
        }
    }
    None
}

fn format_matches(raw: &str, max_total: usize, per_file: usize, max_chars: usize) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut groups: BTreeMap<&str, Vec<MatchLine<'_>>> = BTreeMap::new();
    let mut unparsed: Vec<&str> = Vec::new();

    for line in raw.lines() {
        if line == "--" || line.is_empty() {
            continue;
        }
        if let Some((path, num, sep, text)) = parse_rg_line(line) {
            if !groups.contains_key(path) {
                order.push(path);
            }
            groups.entry(path).or_default().push(MatchLine {
                line_num: num,
                sep,
                text,
            });
        } else {
            unparsed.push(line);
        }
    }

    if order.is_empty() {
        return raw.to_string();
    }

    let mut out = String::with_capacity(raw.len().min(16_384));
    let mut shown_total: usize = 0;
    let mut overflow_files: Vec<(&str, usize)> = Vec::new();
    let mut overflow_matches: usize = 0;

    let stdin_only = order.len() == 1 && order[0] == "<stdin>";

    for path in order {
        let items = &groups[path];
        if shown_total >= max_total {
            overflow_files.push((path, items.len()));
            overflow_matches += items.len();
            continue;
        }

        let remaining_budget = max_total.saturating_sub(shown_total);
        let limit = per_file.min(remaining_budget);

        if stdin_only {
            let take = items.len().min(limit);
            for m in &items[..take] {
                let clean = truncate_clean(m.text, max_chars);
                out.push_str(m.line_num);
                out.push(m.sep);
                out.push(' ');
                out.push_str(&clean);
                out.push('\n');
            }
            if items.len() > take {
                let omitted = items.len() - take;
                out.push_str(&format!("… (+{omitted} more matches — pass --all)\n"));
            }
            shown_total += take;
            continue;
        }

        if items.len() == 1 {
            let m = &items[0];
            let clean = truncate_clean(m.text, max_chars);
            out.push_str(path);
            out.push(':');
            out.push_str(m.line_num);
            out.push_str(": ");
            out.push_str(&clean);
            out.push('\n');
            shown_total += 1;
        } else {
            out.push_str(path);
            out.push_str(" (");
            out.push_str(&items.len().to_string());
            out.push_str(")\n");
            let take = items.len().min(limit);
            for m in &items[..take] {
                let clean = truncate_clean(m.text, max_chars);
                out.push_str("  ");
                out.push_str(m.line_num);
                out.push(m.sep);
                out.push(' ');
                out.push_str(&clean);
                out.push('\n');
            }
            shown_total += take;
            if items.len() > take {
                let rem = items.len() - take;
                out.push_str(&format!("  … (+{rem} more)\n"));
            }
        }
    }

    if !overflow_files.is_empty() {
        let preview: Vec<String> = overflow_files
            .iter()
            .take(5)
            .map(|(p, c)| {
                let name = Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(p);
                format!("{name} ({c})")
            })
            .collect();
        let more_suffix = if overflow_files.len() > 5 {
            format!(", +{} more", overflow_files.len() - 5)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "[trm: +{overflow_matches} more matches in {} files ({}{more_suffix}) — narrow path/pattern or pass --all]\n",
            overflow_files.len(),
            preview.join(", ")
        ));
    }

    for u in unparsed.into_iter().take(5) {
        out.push_str(u);
        out.push('\n');
    }

    out
}

fn format_files_list(raw: &str, show_all: bool) -> String {
    let files: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    if files.len() <= 20 {
        return raw.to_string();
    }

    // Group by parent directory to eliminate repeated directory prefixes
    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in &files {
        let p = Path::new(f);
        let dir = p
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| (*f).to_string());
        by_dir.entry(dir).or_default().push(name);
    }

    let max_files = if show_all {
        usize::MAX
    } else {
        DEFAULT_MAX_FILES_LIST
    };
    let mut out = String::new();
    let mut shown = 0usize;
    let mut hidden_files = 0usize;
    let mut hidden_dirs = 0usize;

    for (dir, names) in by_dir {
        if shown >= max_files {
            hidden_files += names.len();
            hidden_dirs += 1;
            continue;
        }
        let budget = max_files - shown;
        let take = names.len().min(budget);
        out.push_str(&dir);
        out.push_str("/ ");
        out.push_str(&names[..take].join(", "));
        if names.len() > take {
            let rem = names.len() - take;
            out.push_str(&format!(" (+{rem} more)"));
            hidden_files += rem;
        }
        out.push('\n');
        shown += take;
    }

    if hidden_files > 0 {
        out.push_str(&format!(
            "[trm: +{hidden_files} more files across {hidden_dirs} dirs — pass --all or narrow path]\n"
        ));
    }

    out
}

fn truncate_clean(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_chars {
        return trimmed.to_string();
    }
    let mut end = max_chars;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = trimmed.len().saturating_sub(end);
    format!("{}… [+{omitted} chars]", &trimmed[..end])
}
