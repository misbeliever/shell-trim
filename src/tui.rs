use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::audit::{self, AuditSummary};
use crate::cli::UiOptions;
use crate::context_audit::{self, ContextSnapshot};
use crate::style::{self, Cell, Theme};
use crate::telemetry::{self, StatsSnapshot};

const WINDOWS: &[(&str, Option<u64>)] = &[
    ("1h", Some(3_600)),
    ("24h", Some(86_400)),
    ("7d", Some(7 * 86_400)),
    ("all", None),
];

const NUM_TABS: usize = 5;

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
    let mut tab: usize = 0; // 0 Overview, 1 Agents, 2 Misses, 3 Live, 4 Context
    let mut scroll: usize = 0;
    let mut paused = false;

    if opts.once {
        let theme = Theme::detect();
        let (label, secs) = WINDOWS[win_idx];
        let stats = telemetry::load_snapshot(secs);
        let aud = audit::load_summary(secs);
        let ctx = context_audit::load_snapshot();
        let (cols, rows) = term_size();
        let frame = render_frame(
            &theme, tab, label, None, scroll, paused, cols, rows, &stats, &aud, &ctx,
        );
        print!("{frame}");
        return Ok(());
    }

    let _guard = RawTermGuard::enter();
    let theme = Theme { color: true };

    let mut stats = telemetry::load_snapshot(WINDOWS[win_idx].1);
    let mut aud = audit::load_summary(WINDOWS[win_idx].1);
    let mut ctx = context_audit::load_snapshot();
    let mut data_win_idx = win_idx;
    let mut last_refresh = Instant::now();
    let mut last_size_check = Instant::now();
    let mut last_size = term_size();
    let mut needs_redraw = true;
    let mut last_frame = String::new();

    // Offload telemetry / session-log / context disk scans to a background worker
    // so keypresses (tab switching, scrolling, window cycling, quit) stay <1ms instant.
    let (req_tx, req_rx) = std::sync::mpsc::channel::<(usize, bool)>();
    let (res_tx, res_rx) =
        std::sync::mpsc::channel::<(usize, StatsSnapshot, AuditSummary, Option<ContextSnapshot>)>();
    let _worker = std::thread::spawn(move || {
        while let Ok(mut req) = req_rx.recv() {
            // Coalesce rapid `w` / `r` presses so we only scan the latest requested window.
            let mut include_ctx = req.1;
            while let Ok(newer) = req_rx.try_recv() {
                include_ctx |= newer.1;
                req = newer;
            }
            let secs = WINDOWS[req.0].1;
            let new_stats = telemetry::load_snapshot(secs);
            let new_aud = audit::load_summary(secs);
            let new_ctx = if include_ctx {
                Some(context_audit::load_snapshot())
            } else {
                None
            };
            if res_tx.send((req.0, new_stats, new_aud, new_ctx)).is_err() {
                break;
            }
        }
    });
    let mut refresh_in_flight = false;

    let mut stdin = io::stdin();
    let mut buf = [0u8; 8];

    loop {
        while let Ok((resp_win, new_stats, new_aud, new_ctx)) = res_rx.try_recv() {
            refresh_in_flight = false;
            // Context audit has no time window — always apply it even if the user switched windows.
            if let Some(c) = new_ctx {
                ctx = c;
                needs_redraw = true;
            }
            if resp_win == win_idx {
                stats = new_stats;
                aud = new_aud;
                data_win_idx = resp_win;
                needs_redraw = true;
            }
        }

        if !paused && !refresh_in_flight && last_refresh.elapsed() >= Duration::from_secs(2) {
            if req_tx.send((win_idx, false)).is_ok() {
                refresh_in_flight = true;
            }
            last_refresh = Instant::now();
        }

        if last_size_check.elapsed() >= Duration::from_millis(300) {
            let cur_size = term_size();
            last_size_check = Instant::now();
            if cur_size != last_size {
                last_size = cur_size;
                needs_redraw = true;
            }
        }

        if needs_redraw {
            let (cols, rows) = last_size;
            let pending_win = (data_win_idx != win_idx).then_some(WINDOWS[win_idx].0);
            let frame = render_frame(
                &theme,
                tab,
                WINDOWS[data_win_idx].0,
                pending_win,
                scroll,
                paused,
                cols,
                rows,
                &stats,
                &aud,
                &ctx,
            );
            if frame != last_frame {
                let mut painted = String::with_capacity(frame.len() + 256);
                painted.push_str("\x1b[?2026h\x1b[H");
                for (idx, line) in frame.lines().enumerate() {
                    if idx > 0 {
                        painted.push('\n');
                    }
                    painted.push_str(line);
                    if style::visible_width(line) < cols {
                        painted.push_str("\x1b[K");
                    }
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
                    b"5" => {
                        tab = 4;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"\t" | b"l" | b"\x1b[C" => {
                        tab = (tab + 1) % NUM_TABS;
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"h" | b"\x1b[D" | b"\x1b[Z" => {
                        tab = (tab + NUM_TABS - 1) % NUM_TABS;
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
                    b"\x1b[6~" | [4] => {
                        scroll = scroll.saturating_add(10);
                        needs_redraw = true;
                    }
                    b"\x1b[5~" | [21] => {
                        scroll = scroll.saturating_sub(10);
                        needs_redraw = true;
                    }
                    b"g" | b"\x1b[H" => {
                        scroll = 0;
                        needs_redraw = true;
                    }
                    b"p" | b"P" => {
                        paused = !paused;
                        needs_redraw = true;
                    }
                    b"w" | b"W" => {
                        win_idx = (win_idx + 1) % WINDOWS.len();
                        scroll = 0;
                        if req_tx.send((win_idx, false)).is_ok() {
                            refresh_in_flight = true;
                        }
                        last_refresh = Instant::now();
                        needs_redraw = true;
                    }
                    b"r" | b"R" => {
                        if req_tx.send((win_idx, true)).is_ok() {
                            refresh_in_flight = true;
                        }
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
    Some((c.max(20), r.max(12)))
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    t: &Theme,
    tab: usize,
    win_label: &str,
    pending_win: Option<&str>,
    scroll: usize,
    paused: bool,
    cols: usize,
    rows: usize,
    stats: &StatsSnapshot,
    aud: &AuditSummary,
    ctx: &ContextSnapshot,
) -> String {
    // Always leave 1 trailing column free so no terminal's pending-wrap (`xenl`)
    // or `\x1b[K` can ever clip the rightmost character (even at <= 60 columns).
    let w = cols.saturating_sub(1).max(20);
    let mut lines: Vec<String> = Vec::new();

    // 1. Top navigation bar with highlighted active tab pill + window selector pills
    let tabs: [&str; 5] = if w < 74 {
        ["1 Over", "2 Agnt", "3 Miss", "4 Live", "5 Ctx"]
    } else {
        ["1 Overview", "2 Agents", "3 Misses", "4 Live", "5 Context"]
    };
    let tab_bar: Vec<String> = tabs
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == tab {
                t.pill(name)
            } else {
                t.dim(&format!(" {name} "))
            }
        })
        .collect();
    let win_pills: Vec<String> = if w < 78 {
        vec![t.brand(&format!("[{win_label}]"))]
    } else {
        WINDOWS
            .iter()
            .map(|(lbl, _)| {
                if *lbl == win_label {
                    t.brand(&format!("[{lbl}]"))
                } else {
                    t.dim(lbl)
                }
            })
            .collect()
    };
    let pending_badge = pending_win
        .map(|p| format!(" {}", t.dim(&format!("→{p}…"))))
        .unwrap_or_default();
    let pause_badge = if paused {
        format!(" {}", t.amber("[PAUSED]"))
    } else {
        String::new()
    };
    let left_hdr = format!("{}  {}", t.brand("trm"), tab_bar.join(" "));
    let right_hdr = if w >= 84 {
        format!(
            "{} {}{pending_badge}{pause_badge}",
            t.dim("w:"),
            win_pills.join(" ")
        )
    } else {
        format!("{}{pending_badge}{pause_badge}", win_pills.join(" "))
    };
    let left_vis = style::visible_width(&left_hdr);
    let right_vis = style::visible_width(&right_hdr);
    if left_vis + right_vis + 2 <= w {
        let gap = " ".repeat(w - left_vis - right_vis);
        lines.push(format!("{left_hdr}{gap}{right_hdr}"));
    } else if left_vis + right_vis < w {
        lines.push(format!("{left_hdr} {right_hdr}"));
    } else {
        lines.push(left_hdr);
    }
    lines.push(t.dim(&"─".repeat(w)));

    // 2. KPI Header Cards (4 side-by-side cards on wide terminals, compact 2-line fallback on narrow)
    let kpi_lines = render_kpi_header(t, w, stats, aud, ctx);
    lines.extend(kpi_lines);
    lines.push(t.dim(&"─".repeat(w)));

    let header_rows = lines.len();
    let body_budget = rows.saturating_sub(header_rows + 2).max(4);
    let bar_w = if w >= 150 {
        18
    } else if w >= 110 {
        12
    } else {
        10
    };
    let name_w = (w / 3).clamp(28, 64);
    let mut body: Vec<String> = Vec::new();

    match tab {
        0 => render_overview_tab(t, w, bar_w, name_w, stats, aud, &mut body),
        1 => render_agents_tab(t, bar_w, name_w, stats, &mut body),
        2 => render_misses_tab(t, w, stats, aud, &mut body),
        3 => render_live_tab(t, w, paused, stats, &mut body),
        _ => render_context_tab(t, ctx, &mut body),
    }

    let total_body = body.len();
    let max_scroll = total_body.saturating_sub(body_budget);
    let offset = scroll.min(max_scroll);
    for line in body.into_iter().skip(offset).take(body_budget) {
        lines.push(line);
    }

    while lines.len() < rows.saturating_sub(2) {
        lines.push(String::new());
    }
    lines.push(t.dim(&"─".repeat(w)));

    let footer_left = if tab == 3 {
        "  [1-5/←→] view  [w] time  [p] pause  [↑↓/PgUp/Dn] scroll  [r] refresh  [q] quit"
    } else {
        "  [1-5/←→] view  [w] time  [↑↓/PgUp/Dn] scroll  [r] refresh  [q] quit"
    };
    let footer_right = if total_body > body_budget {
        let start = offset + 1;
        let end = (offset + body_budget).min(total_body);
        format!("{start}-{end}/{total_body}  ")
    } else {
        String::new()
    };
    let fl_w = footer_left.chars().count();
    let fr_w = footer_right.chars().count();
    if !footer_right.is_empty() && fl_w + fr_w < w {
        let pad = " ".repeat(w - fl_w - fr_w);
        lines.push(format!(
            "{}{pad}{}",
            t.dim(footer_left),
            t.dim(&footer_right)
        ));
    } else {
        lines.push(t.dim(footer_left));
    }

    // Clip every line to terminal width so no line can soft-wrap and scroll the header off-screen.
    let mut clipped: Vec<String> = lines
        .into_iter()
        .map(|l| style::truncate_ansi(&l, w))
        .collect();
    clipped.truncate(rows);

    let mut out = clipped.join("\n");
    out.push('\n');
    out
}

fn render_kpi_header(
    t: &Theme,
    w: usize,
    stats: &StatsSnapshot,
    aud: &AuditSummary,
    ctx: &ContextSnapshot,
) -> Vec<String> {
    if w < 92 || !t.color {
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
        let turn0_str = ctx
            .by_agent
            .first()
            .map(|(a, _, _, tot)| format!("{a}: {} tok", style::fmt_compact(*tot)))
            .unwrap_or_else(|| "0 tok".into());
        return vec![
            format!(
                "  {} {}  {}    {} {}  {}",
                t.dim("Net Saved:"),
                t.pct_color(stats.net_pct, &net_str),
                t.bar(stats.net_pct, 10),
                t.dim("Adoption:"),
                t.pct_color(aud.adoption_pct, &adopt_str),
                t.bar(aud.adoption_pct, 10)
            ),
            format!(
                "  {} {}   {} {}   {} {}   {} {}   {} {}   {} {}",
                t.dim("Cmds:"),
                stats.events_count,
                t.dim("Misses:"),
                if stats.misses_count > 0 {
                    t.red(&stats.misses_count.to_string())
                } else {
                    t.green("0")
                },
                t.dim("Waste:"),
                format!("{} tok", style::fmt_compact(stats.wasted_tokens)),
                t.dim("1-call rg:"),
                t.green(&stats.turns_saved.to_string()),
                t.dim("Turn-0:"),
                t.cyan(&turn0_str),
                t.dim("7d:"),
                t.green(&sparkline(&stats.daily_saved))
            ),
        ];
    }

    let card_w = ((w.saturating_sub(3)) / 4).max(20);
    let inner = card_w.saturating_sub(2);

    let net_val = format!(
        "{} ({:.1}%)",
        style::fmt_compact_signed(stats.net_saved),
        stats.net_pct
    );
    let card1 = make_card(
        t,
        card_w,
        "NET SAVED",
        &t.pct_color(stats.net_pct, &truncate(&net_val, inner)),
        &format!(
            "{} {}",
            t.bar(stats.net_pct, 8),
            t.green(&sparkline(&stats.daily_saved))
        ),
    );

    let adopt_val = format!(
        "{:.1}% ({}/{})",
        aud.adoption_pct,
        aud.adopted,
        aud.adopted + aud.bypasses
    );
    let card2 = make_card(
        t,
        card_w,
        "ADOPTION",
        &t.pct_color(aud.adoption_pct, &truncate(&adopt_val, inner)),
        &format!(
            "{} {}",
            t.bar(aud.adoption_pct, 8),
            t.dim(&format!("{} logs", aud.files_scanned))
        ),
    );

    let miss_colored = if stats.misses_count > 0 {
        t.red(&format!("{} miss", stats.misses_count))
    } else {
        t.green("0 miss")
    };
    let card3 = make_card(
        t,
        card_w,
        "HEALTH",
        &format!(
            "{miss_colored} · {}",
            t.dim(&format!(
                "{} waste",
                style::fmt_compact(stats.wasted_tokens)
            ))
        ),
        &format!(
            "{} cmd · {} rg1",
            stats.events_count,
            t.green(&stats.turns_saved.to_string())
        ),
    );

    let (t0_top, t0_sub) = if let Some((agent, _, _, tot)) = ctx.by_agent.first() {
        (
            t.cyan(&format!("{agent}: {} tok", style::fmt_compact(*tot))),
            t.dim(&format!(
                "{} skills · {} mcp",
                ctx.enabled_skills,
                ctx.mcp_servers.len()
            )),
        )
    } else {
        (t.dim("none"), t.dim("0 skills · 0 mcp"))
    };
    let card4 = make_card(t, card_w, "TURN-0 TAX", &t0_top, &t0_sub);

    (0..card1.len())
        .map(|i| format!("{} {} {} {}", card1[i], card2[i], card3[i], card4[i]))
        .collect()
}

fn make_card(t: &Theme, width: usize, title: &str, line1: &str, line2: &str) -> Vec<String> {
    let inner = width.saturating_sub(2);
    let title_pad = inner.saturating_sub(title.chars().count() + 1);
    let top = format!("╭─{}{}╮", title, "─".repeat(title_pad));
    let bot = format!("╰{}╯", "─".repeat(inner));
    let v = t.dim("│");
    vec![
        t.dim(&top),
        format!("{v}{}{v}", pad_ansi(line1, inner)),
        format!("{v}{}{v}", pad_ansi(line2, inner)),
        t.dim(&bot),
    ]
}

fn pad_ansi(s: &str, inner: usize) -> String {
    let vis = style::visible_width(s);
    if vis + 1 >= inner {
        return format!(" {s}");
    }
    format!(" {s}{}", " ".repeat(inner - vis - 1))
}

fn join_columns(left: &[String], right: &[String], left_w: usize, gap: usize) -> Vec<String> {
    let rows = left.len().max(right.len());
    let gap_str = " ".repeat(gap);
    let mut out = Vec::with_capacity(rows);
    for i in 0..rows {
        let l = left.get(i).map(|s| s.as_str()).unwrap_or("");
        let r = right.get(i).map(|s| s.as_str()).unwrap_or("");
        let lv = style::visible_width(l);
        let pad = " ".repeat(left_w.saturating_sub(lv));
        if r.is_empty() {
            out.push(l.to_string());
        } else {
            out.push(format!("{l}{pad}{gap_str}{r}"));
        }
    }
    out
}

fn render_overview_tab(
    t: &Theme,
    w: usize,
    bar_w: usize,
    name_w: usize,
    stats: &StatsSnapshot,
    aud: &AuditSummary,
    body: &mut Vec<String>,
) {
    let mut left_col = Vec::new();
    left_col.push(t.bold("By Command:"));
    if stats.by_cmd.is_empty() {
        left_col.push(t.dim("  No trm telemetry recorded in this window yet."));
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
                let net =
                    (b.raw_tokens as isize - b.out_tokens as isize) - b.wasted_tokens as isize;
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
        left_col.extend(style::render_table(t, headers, &table_rows));
    }

    let mut right_col = Vec::new();
    right_col.push(t.bold("Audit Adoption by Agent/Model:"));
    if aud.by_model.is_empty() {
        right_col.push(t.dim("  No agent session logs found in this window."));
    } else {
        let col_name_w = if w >= 108 { 24 } else { name_w };
        let col_bar_w = if w >= 108 { 10 } else { bar_w };
        let headers: &[(&str, bool)] =
            &[("agent/model", false), ("trm/tot", true), ("adopt%", false)];
        let table_rows: Vec<Vec<Cell>> = aud
            .by_model
            .iter()
            .map(|(name, adopted, total, p)| {
                let p_plain = format!("{p:>5.1}%");
                let bar_cell = Cell::styled(
                    format!("{} {}", t.pct_color(*p, &p_plain), t.bar(*p, col_bar_w)),
                    p_plain.chars().count() + 1 + col_bar_w,
                    false,
                );
                vec![
                    Cell::left(truncate(name, col_name_w)),
                    Cell::right(format!("{adopted}/{total}")),
                    bar_cell,
                ]
            })
            .collect();
        right_col.extend(style::render_table(t, headers, &table_rows));
    }

    let left_w = left_col
        .iter()
        .map(|l| style::visible_width(l))
        .max()
        .unwrap_or(0);
    let right_w = right_col
        .iter()
        .map(|l| style::visible_width(l))
        .max()
        .unwrap_or(0);
    if w >= 108 && left_w + right_w + 3 <= w {
        body.extend(join_columns(&left_col, &right_col, left_w, 3));
    } else {
        body.extend(left_col);
        body.push(String::new());
        body.extend(right_col);
    }
}

fn render_agents_tab(
    t: &Theme,
    bar_w: usize,
    name_w: usize,
    stats: &StatsSnapshot,
    body: &mut Vec<String>,
) {
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

fn render_misses_tab(
    t: &Theme,
    w: usize,
    stats: &StatsSnapshot,
    aud: &AuditSummary,
    body: &mut Vec<String>,
) {
    body.push(t.bold("Recent Trim Misses (re-read raw):"));
    if stats.recent_misses.is_empty() {
        body.push(t.green("  None detected in this window."));
    } else {
        for m in &stats.recent_misses {
            body.push(format!(
                "  {} {}",
                t.red("✗"),
                truncate(m, w.saturating_sub(6))
            ));
        }
    }
    body.push(String::new());
    body.push(t.bold("Recent Bypassed Legacy Commands (from session logs):"));
    if aud.recent_bypasses.is_empty() {
        body.push(t.green("  Zero bypasses in this window!"));
    } else {
        for b in &aud.recent_bypasses {
            body.push(format!(
                "  {} {}",
                t.amber("!"),
                truncate(b, w.saturating_sub(6))
            ));
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

fn render_live_tab(
    t: &Theme,
    w: usize,
    paused: bool,
    stats: &StatsSnapshot,
    body: &mut Vec<String>,
) {
    let hdr = if paused {
        "Live Event Feed (PAUSED — press 'p' to resume):"
    } else {
        "Live Event Feed (auto-refreshes every 2s — press 'p' to pause):"
    };
    body.push(t.bold(hdr));
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

fn render_context_tab(t: &Theme, ctx: &ContextSnapshot, body: &mut Vec<String>) {
    body.push(t.bold("Turn-0 Tax by Agent (offline — press 'r' to rescan):"));
    if ctx.by_agent.is_empty() {
        body.push(t.dim("  No agent rules or skills detected."));
    } else {
        let headers: &[(&str, bool)] = &[
            ("agent", false),
            ("rules", true),
            ("skills", true),
            ("total", true),
        ];
        let rows: Vec<Vec<Cell>> = ctx
            .by_agent
            .iter()
            .map(|(a, r, s, tot)| {
                let tot_str = format!("{} tok", style::fmt_compact(*tot));
                vec![
                    Cell::left(a.clone()),
                    Cell::right(style::fmt_compact(*r)),
                    Cell::right(style::fmt_compact(*s)),
                    Cell::styled(t.bold(&tot_str), tot_str.chars().count(), true),
                ]
            })
            .collect();
        body.extend(style::render_table(t, headers, &rows));
    }

    body.push(String::new());
    body.push(t.bold(&format!(
        "Enabled Skills by Source ({} skills):",
        ctx.enabled_skills
    )));
    if !ctx.disabled_plugins.is_empty() {
        body.push(t.dim(&format!(
            "  skipped {} disabled plugins: {}",
            ctx.disabled_plugins.len(),
            ctx.disabled_plugins.join(", ")
        )));
    }
    if ctx.skipped_manual > 0 {
        body.push(t.dim(&format!(
            "  skipped {} user-only skills (disable-model-invocation)",
            ctx.skipped_manual
        )));
    }
    if ctx.skills_by_source.is_empty() {
        body.push(t.dim("  No enabled skills detected."));
    } else {
        let headers: &[(&str, bool)] = &[("source", false), ("skills", true), ("tokens", true)];
        let rows: Vec<Vec<Cell>> = ctx
            .skills_by_source
            .iter()
            .map(|(src, cnt, tok)| {
                vec![
                    Cell::left(src.clone()),
                    Cell::right(cnt.to_string()),
                    Cell::right(style::fmt_compact(*tok)),
                ]
            })
            .collect();
        body.extend(style::render_table(t, headers, &rows));
    }

    body.push(String::new());
    body.push(t.bold(&format!(
        "Configured MCP Servers ({}):",
        ctx.mcp_servers.len()
    )));
    if ctx.mcp_servers.is_empty() {
        body.push(t.dim("  None detected."));
    } else {
        let headers: &[(&str, bool)] = &[("agent", false), ("server", false), ("transport", false)];
        let rows: Vec<Vec<Cell>> = ctx
            .mcp_servers
            .iter()
            .map(|(a, name, tr)| {
                vec![
                    Cell::left(a.clone()),
                    Cell::left(name.clone()),
                    Cell::left(tr.clone()),
                ]
            })
            .collect();
        body.extend(style::render_table(t, headers, &rows));
        body.push(
            t.dim("  Tip: run `trm audit --context --probe-mcp` for live MCP schema tokens."),
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_kpi_cards_and_context_tab_cleanly() {
        let t = Theme { color: true };
        let stats = StatsSnapshot::default();
        let aud = AuditSummary::default();
        let ctx = ContextSnapshot {
            by_agent: vec![("agy".into(), 160, 2800, 2960)],
            skills_by_source: vec![("~/.agents".into(), 32, 2800)],
            enabled_skills: 32,
            disabled_plugins: vec!["genomics".into()],
            skipped_manual: 16,
            mcp_servers: vec![("codex".into(), "node_repl".into(), "stdio: node".into())],
        };

        let overview = render_frame(
            &t,
            0,
            "24h",
            Some("7d"),
            0,
            false,
            120,
            30,
            &stats,
            &aud,
            &ctx,
        );
        assert!(overview.contains("NET SAVED"));
        assert!(overview.contains("TURN-0 TAX"));
        assert!(overview.contains("agy: 3.0K tok"));
        assert!(overview.contains("[24h]"));
        assert!(overview.contains("→7d…"));

        let context = render_frame(&t, 4, "24h", None, 0, true, 120, 30, &stats, &aud, &ctx);
        assert!(context.contains("[PAUSED]"));
        assert!(context.contains("Turn-0 Tax by Agent"));
        assert!(context.contains("node_repl"));

        // Long plugin list + 60-col terminal must leave 1-col right margin (< 60) and stay <= rows
        let wide_ctx = ContextSnapshot {
            disabled_plugins: (0..12)
                .map(|i| format!("very-long-disabled-plugin-name-{i}"))
                .collect(),
            ..ctx
        };
        let narrow = render_frame(
            &t, 4, "24h", None, 0, false, 60, 20, &stats, &aud, &wide_ctx,
        );
        let narrow_lines: Vec<&str> = narrow.lines().collect();
        assert!(
            narrow_lines.len() <= 20,
            "exceeded rows: {}",
            narrow_lines.len()
        );
        assert!(
            narrow_lines[0].contains("5 Ctx"),
            "top tab bar missing: {}",
            narrow_lines[0]
        );
        for (idx, line) in narrow_lines.iter().enumerate() {
            let vw = style::visible_width(line);
            assert!(
                vw < 60,
                "line {idx} hit right edge at 60 cols ({vw}): {line:?}"
            );
        }

        // End-to-end narrow size parsing (40x20) -> render_frame stays strictly < 40 cols
        let (c40, r20) = parse_stty_size(b"20 40\n").unwrap();
        assert_eq!((c40, r20), (40, 20));
        let tiny = render_frame(
            &t, 4, "24h", None, 0, false, c40, r20, &stats, &aud, &wide_ctx,
        );
        for (idx, line) in tiny.lines().enumerate() {
            let vw = style::visible_width(line);
            assert!(vw < 40, "line {idx} overflowed 40 cols ({vw}): {line:?}");
        }
    }
}
