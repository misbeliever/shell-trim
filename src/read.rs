use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::cli::{LineRange, ReadMode, ReadOptions};
use crate::config::Config;
use crate::filter::{self, FilterStrength, FilteredItem};
use crate::lang::Language;
use crate::outline;
use crate::telemetry;

const BINARY_SNIFF_BYTES: usize = 8192;

pub fn run(opts: &ReadOptions) -> Result<(), String> {
    let multi = opts.paths.len() > 1;
    for (idx, path) in opts.paths.iter().enumerate() {
        if multi {
            let header = if idx == 0 {
                format!("==> {path} <==\n")
            } else {
                format!("\n==> {path} <==\n")
            };
            io::stdout()
                .write_all(header.as_bytes())
                .map_err(|e| format!("write failed: {e}"))?;
        }
        run_single(path, opts)?;
    }
    Ok(())
}

fn run_single(path: &str, opts: &ReadOptions) -> Result<(), String> {
    let raw_bytes_vec = read_input(path)?;

    // Binary file guard
    let sniff_len = raw_bytes_vec.len().min(BINARY_SNIFF_BYTES);
    if raw_bytes_vec[..sniff_len].contains(&0) {
        let msg = format!(
            "[trm: binary file ({}) — skipped]\n",
            format_size(raw_bytes_vec.len())
        );
        let _ = io::stdout().write_all(msg.as_bytes());
        return Ok(());
    }

    let content = String::from_utf8_lossy(&raw_bytes_vec);
    let all_lines: Vec<&str> = split_lines_preserving_cr(&content);
    let total_lines = all_lines.len();

    if total_lines == 0 {
        return Ok(());
    }

    let lang = Language::detect(path, opts.lang_override.as_deref());

    // Handle -s / --symbol jump (always outputs exact raw lines of matched symbol(s))
    if let Some(ref sym) = opts.symbol {
        return run_symbol_read(path, &all_lines, raw_bytes_vec.len(), lang, sym, opts);
    }

    // Resolve effective range from --range, --head, or --tail
    let effective_range: Option<LineRange> = if let Some(r) = opts.range {
        Some(r)
    } else if let Some(n) = opts.head {
        Some(LineRange {
            start: 1,
            end: Some(n.min(total_lines)),
        })
    } else if let Some(n) = opts.tail {
        let start = total_lines.saturating_sub(n) + 1;
        Some(LineRange {
            start,
            end: Some(total_lines),
        })
    } else {
        None
    };

    // Apply requested range (1-based inclusive)
    let (slice_start_1based, slice_lines): (usize, &[&str]) = match effective_range {
        Some(r) => {
            if r.start > total_lines {
                return Err(format!(
                    "range start {} exceeds file length ({} lines)",
                    r.start, total_lines
                ));
            }
            let start_idx = r.start - 1;
            let end_idx = r.end.unwrap_or(total_lines).min(total_lines);
            (r.start, &all_lines[start_idx..end_idx])
        }
        None => (1, &all_lines[..]),
    };

    // Baseline raw byte count for fair telemetry (the slice if ranged, else whole file)
    let baseline_bytes = if effective_range.is_some() {
        slice_lines.iter().map(|l| l.len() + 1).sum()
    } else {
        raw_bytes_vec.len()
    };

    // Decide effective mode (honoring optional ~/.config/trm/config.toml thresholds)
    let cfg = Config::load();
    let mut eff_mode = match opts.mode {
        ReadMode::Auto => {
            if effective_range.is_some() || total_lines <= cfg.small_lines {
                ReadMode::Raw
            } else if total_lines > cfg.outline_lines && lang.supports_outline() {
                ReadMode::Outline
            } else {
                ReadMode::Minimal
            }
        }
        explicit => explicit,
    };

    let mut output_lines: Vec<String> = match eff_mode {
        ReadMode::Raw => format_raw(slice_lines, slice_start_1based, opts.line_numbers),
        ReadMode::Minimal => {
            let clean_slice = strip_cr_lines(slice_lines);
            let items = filter::filter_lines(
                &clean_slice,
                slice_start_1based,
                lang,
                FilterStrength::Minimal,
            );
            let formatted = format_filtered(&items, opts.line_numbers);
            let est_bytes: usize = formatted.iter().map(|l| l.len() + 1).sum();
            if !opts.line_numbers && est_bytes >= baseline_bytes {
                eff_mode = ReadMode::Raw;
                format_raw(slice_lines, slice_start_1based, false)
            } else {
                formatted
            }
        }
        ReadMode::Aggressive => {
            let clean_slice = strip_cr_lines(slice_lines);
            let items = filter::filter_lines(
                &clean_slice,
                slice_start_1based,
                lang,
                FilterStrength::Aggressive,
            );
            let formatted = format_filtered(&items, opts.line_numbers);
            let est_bytes: usize = formatted.iter().map(|l| l.len() + 1).sum();
            if !opts.line_numbers && est_bytes >= baseline_bytes {
                eff_mode = ReadMode::Raw;
                format_raw(slice_lines, slice_start_1based, false)
            } else {
                formatted
            }
        }
        ReadMode::Outline => {
            let clean_slice = strip_cr_lines(slice_lines);
            let outlined = outline::extract_outline(&clean_slice, lang);
            if outlined.len() < 3 && opts.mode == ReadMode::Auto {
                eff_mode = ReadMode::Minimal;
                let items = filter::filter_lines(
                    &clean_slice,
                    slice_start_1based,
                    lang,
                    FilterStrength::Minimal,
                );
                format_filtered(&items, opts.line_numbers)
            } else if outlined.is_empty() {
                vec![format!(
                    "[trm: no outline symbols found in {path} ({total_lines} lines) — use --minimal or --raw]"
                )]
            } else {
                let mut v = outlined;
                v.push(format!(
                    "[trm: outline of {total_lines} lines ({}) — 'trm read {path} -r START-END' or '-s <symbol>' for raw code]",
                    lang.name()
                ));
                v
            }
        }
        ReadMode::Auto => unreachable!(),
    };

    apply_limits(&mut output_lines, opts);
    emit_and_log(
        path,
        lang,
        opts.mode.as_str(),
        eff_mode.as_str(),
        effective_range.map(|r| r.format()),
        total_lines,
        baseline_bytes,
        &output_lines,
        opts.log_stats,
    )
}

fn run_symbol_read(
    path: &str,
    all_lines: &[&str],
    full_file_bytes: usize,
    lang: Language,
    sym: &str,
    opts: &ReadOptions,
) -> Result<(), String> {
    let total_lines = all_lines.len();
    let clean_lines = strip_cr_lines(all_lines);
    let queries: Vec<&str> = sym
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for q in &queries {
        let found = outline::find_symbol_ranges(&clean_lines, lang, q);
        if found.is_empty() {
            missing.push(q);
        } else {
            for r in found {
                if !ranges.contains(&r) {
                    ranges.push(r);
                }
            }
        }
    }

    if ranges.is_empty() {
        let mut msg = format!("symbol '{sym}' not found in '{path}'");
        let suggestions =
            outline::suggest_symbols(&clean_lines, lang, queries.first().copied().unwrap_or(sym), 3);
        if !suggestions.is_empty() {
            msg.push_str("\nClosest symbols:\n");
            for s in suggestions {
                msg.push_str("  ");
                msg.push_str(&s);
                msg.push('\n');
            }
            msg.push_str(&format!("(use 'trm read {path} -r START-END' or '-s <name>')"));
        } else {
            msg.push_str(&format!(" (try 'trm read {path} --outline')"));
        }
        return Err(msg);
    }

    let mut output_lines: Vec<String> = Vec::new();
    for (idx, &(start, end)) in ranges.iter().enumerate() {
        let s_idx = start.saturating_sub(1).min(total_lines);
        let e_idx = end.min(total_lines);
        if idx > 0 {
            output_lines.push(String::new());
        }
        output_lines.push(format!("[L{start}-L{end}]"));
        let slice = &all_lines[s_idx..e_idx];
        output_lines.extend(format_raw(slice, start, opts.line_numbers));
    }
    for m in missing {
        let sugg = outline::suggest_symbols(&clean_lines, lang, m, 2);
        if sugg.is_empty() {
            output_lines.push(format!("[trm: symbol '{m}' not found]"));
        } else {
            output_lines.push(format!(
                "[trm: symbol '{m}' not found — closest: {}]",
                sugg.join(" | ")
            ));
        }
    }

    apply_limits(&mut output_lines, opts);
    emit_and_log(
        path,
        lang,
        "symbol",
        "symbol",
        Some(format!("sym:{sym}")),
        total_lines,
        full_file_bytes,
        &output_lines,
        opts.log_stats,
    )
}

fn apply_limits(output_lines: &mut Vec<String>, opts: &ReadOptions) {
    if let Some(limit) = opts.max_chars {
        for line in output_lines.iter_mut() {
            let content_len = line.strip_suffix('\r').map_or(line.len(), str::len);
            if content_len > limit {
                let mut end = limit;
                while end > 0 && !line.is_char_boundary(end) {
                    end -= 1;
                }
                let omitted = content_len.saturating_sub(end);
                *line = format!("{}… [+{omitted} chars]", &line[..end]);
            }
        }
    }

    if let Some(cap) = opts.max_lines
        && output_lines.len() > cap
    {
        let omitted = output_lines.len() - cap;
        output_lines.truncate(cap);
        output_lines.push(format!(
            "[trm: truncated {omitted} more lines (--max-lines {cap}) — use -r START-END]"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_and_log(
    path: &str,
    lang: Language,
    req_mode: &str,
    eff_mode: &str,
    range: Option<String>,
    total_lines: usize,
    baseline_bytes: usize,
    output_lines: &[String],
    log_stats: bool,
) -> Result<(), String> {
    let mut rendered = String::with_capacity(baseline_bytes.min(65_536));
    for line in output_lines {
        rendered.push_str(line);
        rendered.push('\n');
    }

    let out_bytes = rendered.len();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    handle
        .write_all(rendered.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    if log_stats {
        let canonical_path = if path == "-" {
            "<stdin>".to_string()
        } else {
            fs::canonicalize(Path::new(path))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string())
        };
        telemetry::log_read_event(
            &canonical_path,
            lang.name(),
            req_mode,
            eff_mode,
            range,
            total_lines,
            baseline_bytes,
            out_bytes,
        );
    }

    Ok(())
}

pub(crate) fn split_lines_preserving_cr(content: &str) -> Vec<&str> {
    content
        .split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .collect()
}

fn strip_cr_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    lines
        .iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

fn read_input(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed reading stdin: {e}"))?;
        Ok(buf)
    } else {
        fs::read(path).map_err(|e| format!("cannot read '{path}': {e}"))
    }
}

fn format_raw(lines: &[&str], start_line: usize, line_numbers: bool) -> Vec<String> {
    if !line_numbers {
        return lines.iter().map(|l| (*l).to_string()).collect();
    }
    let max_num = start_line + lines.len().saturating_sub(1);
    let width = digit_width(max_num).max(2);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>width$}| {l}", start_line + i))
        .collect()
}

fn format_filtered(items: &[FilteredItem], line_numbers: bool) -> Vec<String> {
    let max_num = items
        .iter()
        .map(|it| match it {
            FilteredItem::Line { num, .. } => *num,
            FilteredItem::Omitted { end, .. } => *end,
        })
        .max()
        .unwrap_or(1);
    let width = digit_width(max_num).max(2);
    let pad = " ".repeat(width);

    items
        .iter()
        .map(|it| match it {
            FilteredItem::Line { num, text } => {
                if line_numbers {
                    if text.is_empty() {
                        format!("{num:>width$}|")
                    } else {
                        format!("{num:>width$}| {text}")
                    }
                } else {
                    text.clone()
                }
            }
            FilteredItem::Omitted { start, end } => {
                let count = end.saturating_sub(*start) + 1;
                if line_numbers {
                    format!("{pad}| … ({count} lines omitted: L{start}-L{end})")
                } else {
                    format!("… ({count} lines omitted: L{start}-L{end})")
                }
            }
        })
        .collect()
}

fn digit_width(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut x = n;
    let mut d = 0;
    while x > 0 {
        d += 1;
        x /= 10;
    }
    d
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
