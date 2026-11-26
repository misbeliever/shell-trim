use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::audit::{self, AuditSummary};
use crate::cli::UiOptions;
use crate::style::{self, Cell, Theme};
use crate::telemetry::{self, StatsSnapshot};

const WINDOWS: &[(&str, Option<u64>)] = &[
    ("1h", Some(3_600)),
    ("24h", Some(86_400)),
    ("7d", Some(7 * 86_400)),
    ("all", None),
];

struct RawTermGuard {
    saved_stty: Option<String>,
}

impl RawTermGuard {
    fn enter() -> Self {
        let saved_stty = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::inherit())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        let _ = Command::new("stty")
            .args(["-echo", "-icanon", "min", "0", "time", "2"])
            .status();

        let mut out = io::stdout();
        let _ = write!(out, "\x1b[?1049h\x1b[?25l");
        let _ = out.flush();

        Self { saved_stty }
    }
}

impl Drop for RawTermGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b[?25h\x1b[?1049l");
        let _ = out.flush();
        if let Some(ref saved) = self.saved_stty {
            let _ = Command::new("stty").arg(saved).status();
        } else {
            let _ = Command::new("stty").arg("sane").status();
        }
    }
}

pub fn run(opts: &UiOptions) -> Result<(), String> {
    let mut win_idx = match opts.since_label.as_deref() {
        Some("1h") => 0,
        Some("7d") => 2,
        Some("all") => 3,
        _ => 1, // default 24h
    };
    let mut tab: usize = 0; // 0 Overview, 1 Agents/Models, 2 Misses/Bypasses, 3 Live Feed
    let mut scroll: usize = 0;

    if opts.once {
        let theme = Theme::detect();
        let (label, secs) = WINDOWS[win_idx];
        let stats = telemetry::load_snapshot(secs);
        let aud = audit::load_summary(secs);
        let (cols, rows) = term_size();
        let frame = render_frame(&theme, tab, label, scroll, cols, rows, &stats, &aud);
        print!("{frame}");
        return Ok(());
    }

    let _guard = RawTermGuard::enter();
    let theme = Theme { color: true };

    let mut stats = telemetry::load_snapshot(WINDOWS[win_idx].1);
    let mut aud = audit::load_summary(WINDOWS[win_idx].1);
    let mut last_refresh = Instant::now();
    let mut last_size = term_size();
    let mut needs_redraw = true;
    let mut last_frame = String::new();

    let mut stdin = io::stdin();
    let mut buf = [0u8; 8];

    loop {
        if last_refresh.elapsed() >= Duration::from_secs(2) {
            stats = telemetry::load_snapshot(WINDOWS[win_idx].1);
            aud = audit::load_summary(WINDOWS[win_idx].1);
            last_refresh = Instant::now();
            needs_redraw = true;
        }

        let cur_size = term_size();
        if cur_size != last_size {
            last_size = cur_size;
            needs_redraw = true;
        }

        if needs_redraw {
            let (cols, rows) = cur_size;
            let frame = render_frame(
                &theme,
                tab,
                WINDOWS[win_idx].0,
                scroll,
                cols,
                rows,
                &stats,
                &aud,
            );
            if frame != last_frame {
                let mut painted = String::with_capacity(frame.len() + 256);
                painted.push_str("\x1b[?2026h\x1b[H");
                for (idx, line) in frame.lines().enumerate() {
                    if idx > 0 {
                        painted.push('\n');
                    }
                    painted.push_str(line);
                    painted.push_str("\x1b[K");
                }
                painted.push_str("\x1b[J\x1b[?2026l");
                let mut out = io::stdout();
                let _ = out.write_all(painted.as_bytes());
                let _ = out.flush();
                last_frame = frame;
            }
            needs_redraw = false;
        }

        match stdin.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                let slice = &buf[..n];
                if slice == b"q" || slice == b"Q" || slice == [3] || slice == [27] {
                    break;
                }
                match slice {
                    b"1" => {
                        tab = 0;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"2" => {
                        tab = 1;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"3" => {
                        tab = 2;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"4" => {
                        tab = 3;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"\t" | b"l" | b"\x1b[C" => {
                        tab = (tab + 1) % 4;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"h" | b"\x1b[D" => {
                        tab = (tab + 3) % 4;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"j" | b"\x1b[B" => {
                        scroll = scroll.saturating_add(1);
                        needs_redraw = true;
                    }
                    b"k" | b"\x1b[A" => {
                        scroll = scroll.saturating_sub(1);
                        needs_redraw = true;
                    }
                    b"w" | b"W" => {
                        win_idx = (win_idx + 1) % WINDOWS.len();
                        scroll = 0;
                        stats = telemetry::load_snapshot(WINDOWS[win_idx].1);
                        aud = audit::load_summary(WINDOWS[win_idx].1);
                        last_refresh = Instant::now();
                        needs_redraw = true;
                    }
                    b"r" | b"R" => {
                        stats = telemetry::load_snapshot(WINDOWS[win_idx].1);
                        aud = audit::load_summary(WINDOWS[win_idx].1);
                        last_refresh = Instant::now();
                        needs_redraw = true;
                    }
                    _ => {}
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}

fn term_size() -> (usize, usize) {
    // `stty size` queries stdin (fd 0); `.output()` defaults stdin to null,
    // so we must inherit stdin (or fall back to /dev/tty) to read the real TTY size.
    if let Ok(out) = Command::new("stty")
        .arg("size")
        .stdin(Stdio::inherit())
        .output()
        && out.status.success()
        && let Some(sz) = parse_stty_size(&out.stdout)
    {
        return sz;
    }
    if let Ok(tty) = std::fs::File::open("/dev/tty")
        && let Ok(out) = Command::new("stty")
            .arg("size")
            .stdin(Stdio::from(tty))
            .output()
        && out.status.success()
        && let Some(sz) = parse_stty_size(&out.stdout)
    {
        return sz;
    }
    (100, 30)
}

fn parse_stty_size(stdout: &[u8]) -> Option<(usize, usize)> {
    let s = String::from_utf8_lossy(stdout);
    let mut it = s.split_whitespace();
    let r = it.next()?.parse::<usize>().ok()?;
    let c = it.next()?.parse::<usize>().ok()?;
    if r == 0 || c == 0 {
        return None;
    }
    Some((c.max(60), r.max(18)))
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    t: &Theme,
    tab: usize,
    win_label: &str,
    scroll: usize,
    cols: usize,
    rows: usize,
    stats: &StatsSnapshot,
    aud: &AuditSummary,
) -> String {
    let w = cols.max(60);
    let mut lines: Vec<String> = Vec::new();

    // Top bar
    let tabs = ["1 Overview", "2 Agents", "3 Misses", "4 Live"];
    let tab_bar: Vec<String> = tabs
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == tab {
                t.brand(&format!("[{name}]"))
            } else {
                t.dim(&format!(" {name} "))
            }
        })
        .collect();
    lines.push(format!(
        "{}  {}   {}",
        t.brand("trm |"),
        tab_bar.join(" "),
        t.dim(&format!("window: {win_label} · {} logs", aud.files_scanned))
    ));
    lines.push(t.dim(&"─".repeat(w)));

    // KPI summary row (always visible)
    let net_str = format!(
        "{} tok ({:.1}%)",
        style::fmt_compact_signed(stats.net_saved),
        stats.net_pct
    );
    let adopt_str = format!(
        "{}/{} ({:.1}%)",
        aud.adopted,
        aud.adopted + aud.bypasses,
        aud.adoption_pct
    );
    lines.push(format!(
        "  {} {}  {}    {} {}  {}",
        t.dim("Net Saved:"),
        t.pct_color(stats.net_pct, &net_str),
        t.bar(stats.net_pct, 10),
        t.dim("Adoption:"),
        t.pct_color(aud.adoption_pct, &adopt_str),
        t.bar(aud.adoption_pct, 10)
    ));
    let waste_str = format!("{} tok", style::fmt_compact(stats.wasted_tokens));
    lines.push(format!(
        "  {} {}   {} {}   {} {}   {} {}   {} {}   {} {}",
        t.dim("Commands:"),
        stats.events_count,
        t.dim("Drill:"),
        stats.drilldowns,
        t.dim("1-call rg:"),
        t.green(&stats.turns_saved.to_string()),
        t.dim("Misses:"),
        if stats.misses_count > 0 {
            t.red(&stats.misses_count.to_string())
        } else {
            t.green("0")
        },
        t.dim("Waste:"),
        waste_str,
        t.dim("7d:"),
        t.green(&sparkline(&stats.daily_saved))
    ));
    lines.push(t.dim(&"─".repeat(w)));

    let body_budget = rows.saturating_sub(8);
    let bar_w = if w >= 150 {
        22
    } else if w >= 110 {
        16
    } else {
        10
    };
    let name_w = (w / 3).clamp(32, 72);
    let mut body: Vec<String> = Vec::new();

    match tab {
        0 => {
            body.push(t.bold("By Command:"));
            if stats.by_cmd.is_empty() {
                body.push(t.dim("  No trm telemetry recorded in this window yet."));
            } else {
                let mut rows_v: Vec<_> = stats.by_cmd.iter().collect();
                rows_v.sort_by_key(|a| std::cmp::Reverse(a.1.raw_tokens));
                let headers: &[(&str, bool)] = &[
                    ("cmd", false),
                    ("calls", true),
                    ("raw_tok", true),
                    ("out_tok", true),
                    ("miss", true),
                    ("net%", false),
                ];
                let table_rows: Vec<Vec<Cell>> = rows_v
                    .into_iter()
                    .map(|(cmd, b)| {
                        let net = (b.raw_tokens as isize - b.out_tokens as isize) - b.wasted_tokens as isize;
                        let p = if b.raw_tokens == 0 {
                            0.0
                        } else {
                            (net as f64 / b.raw_tokens as f64) * 100.0
                        };
                        let p_plain = format!("{p:>5.1}%");
                        let net_cell = Cell::styled(
                            format!("{} {}", t.pct_color(p, &p_plain), t.bar(p, bar_w)),
                            p_plain.chars().count() + 1 + bar_w,
                            false,
                        );
                        vec![
                            Cell::left(cmd.clone()),
                            Cell::right(b.reads.to_string()),
                            Cell::right(style::fmt_compact(b.raw_tokens)),
                            Cell::right(style::fmt_compact(b.out_tokens)),
                            Cell::right(b.misses.to_string()),
                            net_cell,
                        ]
                    })
                    .collect();
                body.extend(style::render_table(t, headers, &table_rows));
            }
            body.push(String::new());
            body.push(t.bold("Audit Adoption by Agent/Model:"));
            if aud.by_model.is_empty() {
                body.push(t.dim("  No agent session logs found in this window."));
            } else {
                let headers: &[(&str, bool)] = &[
                    ("agent/model", false),
                    ("trm/tot", true),
                    ("adopt%", false),
                ];
                let table_rows: Vec<Vec<Cell>> = aud
                    .by_model
                    .iter()
                    .map(|(name, adopted, total, p)| {
                        let p_plain = format!("{p:>5.1}%");
                        let bar_cell = Cell::styled(
                            format!("{} {}", t.pct_color(*p, &p_plain), t.bar(*p, bar_w)),
                            p_plain.chars().count() + 1 + bar_w,
                            false,
                        );
                        vec![
                            Cell::left(truncate(name, name_w)),
                            Cell::right(format!("{adopted}/{total}")),
                            bar_cell,
                        ]
                    })
                    .collect();
                body.extend(style::render_table(t, headers, &table_rows));
            }
        }
        1 => {
            body.push(t.bold("Token Savings by Agent:"));
            let headers_a: &[(&str, bool)] = &[
                ("agent", false),
                ("calls", true),
                ("raw_tok", true),
                ("out_tok", true),
                ("miss", true),
                ("net%", false),
            ];
            let rows_a: Vec<Vec<Cell>> = stats
                .by_agent
                .iter()
                .map(|(agent, b)| {
                    let net = (b.raw_tokens as isize - b.out_tokens as isize) - b.wasted_tokens as isize;
                    let p = if b.raw_tokens == 0 {
                        0.0
                    } else {
                        (net as f64 / b.raw_tokens as f64) * 100.0
                    };
                    let p_plain = format!("{p:>5.1}%");
                    let bar_cell = Cell::styled(
                        format!("{} {}", t.pct_color(p, &p_plain), t.bar(p, bar_w)),
                        p_plain.chars().count() + 1 + bar_w,
                        false,
                    );
                    vec![
                        Cell::left(agent.clone()),
                        Cell::right(b.reads.to_string()),
                        Cell::right(style::fmt_compact(b.raw_tokens)),
                        Cell::right(style::fmt_compact(b.out_tokens)),
                        Cell::right(b.misses.to_string()),
                        bar_cell,
                    ]
                })
                .collect();
            body.extend(style::render_table(t, headers_a, &rows_a));

            body.push(String::new());
            body.push(t.bold("Token Savings by Provider/Model:"));
            let headers_m: &[(&str, bool)] = &[
                ("provider/model", false),
                ("calls", true),
                ("saved", true),
                ("net%", false),
            ];
            let rows_m: Vec<Vec<Cell>> = stats
                .by_model
                .iter()
                .map(|(model, b)| {
                    let net = (b.raw_tokens as isize - b.out_tokens as isize) - b.wasted_tokens as isize;
                    let p = if b.raw_tokens == 0 {
                        0.0
                    } else {
                        (net as f64 / b.raw_tokens as f64) * 100.0
                    };
                    let p_plain = format!("{p:>5.1}%");
                    let bar_cell = Cell::styled(
                        format!("{} {}", t.pct_color(p, &p_plain), t.bar(p, bar_w)),
                        p_plain.chars().count() + 1 + bar_w,
                        false,
                    );
                    vec![
                        Cell::left(truncate(model, name_w)),
                        Cell::right(b.reads.to_string()),
                        Cell::right(style::fmt_compact_signed(net)),
                        bar_cell,
                    ]
                })
                .collect();
            body.extend(style::render_table(t, headers_m, &rows_m));
        }
        2 => {
            body.push(t.bold("Recent Trim Misses (re-read raw):"));
            if stats.recent_misses.is_empty() {
                body.push(t.green("  None detected in this window."));
            } else {
                for m in &stats.recent_misses {
                    body.push(format!("  {} {}", t.red("✗"), truncate(m, w.saturating_sub(6))));
                }
            }
            body.push(String::new());
            body.push(t.bold("Recent Bypassed Legacy Commands (from session logs):"));
            if aud.recent_bypasses.is_empty() {
                body.push(t.green("  Zero bypasses in this window!"));
            } else {
                for b in &aud.recent_bypasses {
                    body.push(format!("  {} {}", t.amber("!"), truncate(b, w.saturating_sub(6))));
                }
            }
            body.push(String::new());
            body.push(t.bold(&format!(
                "Large Untrimmed Passthroughs (>={} tok, auto):",
                telemetry::LARGE_PASSTHROUGH_MIN_TOKENS
            )));
            if stats.large_passthroughs.is_empty() {
                body.push(t.green("  None — all large auto outputs were trimmed."));
            } else {
                let now = telemetry::now_secs();
                let max_line = w.saturating_sub(6);
                for lp in &stats.large_passthroughs {
                    let age = telemetry::fmt_age(lp.ts, now);
                    let prefix = format!(
                        "{:>7} {:>6} tok  [{}:{}]  ",
                        age,
                        style::fmt_compact(lp.raw_tokens),
                        lp.cmd,
                        lp.lang
                    );
                    let path_budget = max_line.saturating_sub(prefix.chars().count()).max(16);
                    let target = compact_target(&lp.path, path_budget);
                    let line = format!("{prefix}{target}");
                    body.push(format!("  {} {}", t.amber("·"), truncate(&line, max_line)));
                }
            }
        }
        _ => {
            body.push(t.bold("Live Event Feed (auto-refreshes every 2s):"));
            if stats.recent_events.is_empty() {
                body.push(t.dim("  Waiting for trm commands..."));
            } else {
                let now = telemetry::now_secs();
                let (agent_w, mode_w) = if w < 95 {
                    (20usize, 12usize)
                } else if w < 130 {
                    (30usize, 14usize)
                } else {
                    (36usize, 14usize)
                };
                // Fixed prefix: "  " (2) + age (7) + " " (1) + agent_w + " " (1) + mode_w + " " (1)
                // + "XXXXXX->XXXXXX" (14) + " " (1) + "100.0%" (6) + "  " (2) = 35 + agent_w + mode_w
                let fixed_w = 35 + agent_w + mode_w;
                let target_w = w.saturating_sub(fixed_w).max(16);
                for (ts, agent, mode, path, rtok, otok) in &stats.recent_events {
                    let saved = *rtok as isize - *otok as isize;
                    let p = if *rtok == 0 {
                        0.0
                    } else {
                        (saved as f64 / *rtok as f64) * 100.0
                    };
                    let age = telemetry::fmt_age(*ts, now);
                    let target = compact_target(path, target_w);
                    body.push(format!(
                        "  {} {:<agent_w$} {:<mode_w$} {:>6}->{:<6} {}  {}",
                        t.dim(&format!("{age:>7}")),
                        truncate(agent, agent_w),
                        truncate(mode, mode_w),
                        style::fmt_compact(*rtok),
                        style::fmt_compact(*otok),
                        t.pct_color(p, &format!("{p:>5.1}%")),
                        truncate(&target, target_w)
                    ));
                }
            }
        }
    }

    let max_scroll = body.len().saturating_sub(body_budget);
    let offset = scroll.min(max_scroll);
    for line in body.into_iter().skip(offset).take(body_budget) {
        lines.push(line);
    }

    while lines.len() < rows.saturating_sub(2) {
        lines.push(String::new());
    }
    lines.push(t.dim(&"─".repeat(w)));
    lines.push(t.dim("  [1-4 / Tab] switch view   [w] window (1h/24h/7d/all)   [j/k] scroll   [r] refresh   [q] quit"));

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn sparkline(values: &[usize]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return "▁".repeat(values.len().max(7));
    }
    values
        .iter()
        .map(|&v| {
            let idx = ((v as f64 / max as f64) * 7.0).round() as usize;
            BARS[idx.min(7)]
        })
        .collect()
}

fn compact_target(s: &str, max_w: usize) -> String {
    if s.chars().count() <= max_w {
        return s.to_string();
    }
    if !s.contains('/')
        || s.starts_with("rg:")
        || s.starts_with("git:")
        || s.starts_with("cargo:")
        || s.starts_with("swift")
    {
        return truncate(s, max_w);
    }
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 1 {
        return truncate(s, max_w);
    }
    // Keep as many trailing path segments as fit inside `max_w`
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 1usize; // leading "…"
    for seg in parts.iter().rev() {
        let seg_len = 1 + seg.chars().count(); // "/seg"
        if !kept.is_empty() && used + seg_len > max_w {
            break;
        }
        kept.push(seg);
        used += seg_len;
    }
    kept.reverse();
    let candidate = format!("…/{}", kept.join("/"));
    truncate(&candidate, max_w)
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let prefix: String = s.chars().take(take).collect();
    format!("{prefix}…")
}
