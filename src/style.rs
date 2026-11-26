use std::env;
use std::io::{self, IsTerminal};

/// Zero-dependency ANSI styling helper.
/// Automatically enabled only when stdout is a TTY and `NO_COLOR` is unset,
/// so piped output and AI agent tool calls stay 100% plain text (zero token waste).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub color: bool,
}

impl Theme {
    pub fn detect() -> Self {
        let no_color = env::var_os("NO_COLOR").is_some();
        let force = env::var("TRM_COLOR").is_ok_and(|v| v == "1" || v == "always");
        let color = force || (!no_color && io::stdout().is_terminal());
        Self { color }
    }

    pub fn brand(&self, s: &str) -> String {
        self.wrap("1;38;5;208", s) // bold Rust orange (#F97316)
    }

    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }

    pub fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }

    pub fn green(&self, s: &str) -> String {
        self.wrap("38;5;42", s) // emerald
    }

    pub fn cyan(&self, s: &str) -> String {
        self.wrap("38;5;80", s) // soft teal/cyan
    }

    pub fn amber(&self, s: &str) -> String {
        self.wrap("38;5;214", s)
    }

    pub fn red(&self, s: &str) -> String {
        self.wrap("38;5;203", s)
    }

    pub fn pct_color(&self, pct: f64, text: &str) -> String {
        if pct >= 70.0 {
            self.green(text)
        } else if pct >= 30.0 {
            self.cyan(text)
        } else if pct > 0.0 {
            self.amber(text)
        } else if pct < 0.0 {
            self.red(text)
        } else {
            self.dim(text)
        }
    }

    pub fn bar(&self, pct: f64, width: usize) -> String {
        let clamped = pct.clamp(0.0, 100.0);
        let filled = ((clamped / 100.0) * width as f64).round() as usize;
        let empty = width.saturating_sub(filled);
        let full_str = "█".repeat(filled);
        let empty_str = "░".repeat(empty);
        if !self.color {
            return format!("{full_str}{empty_str}");
        }
        let colored_full = if clamped >= 70.0 {
            self.green(&full_str)
        } else if clamped >= 30.0 {
            self.cyan(&full_str)
        } else if clamped > 0.0 {
            self.amber(&full_str)
        } else {
            self.dim(&full_str)
        };
        format!("{colored_full}{}", self.dim(&empty_str))
    }

    pub fn header_rule(&self, title: &str, subtitle: &str) -> String {
        if !self.color {
            return format!(
                "{title} ({subtitle})\n─────────────────────────────────────────────────────────────────"
            );
        }
        let line = self.dim("─────────────────────────────────────────────────────────────────");
        format!(
            "{} {}\n{line}",
            self.brand(title),
            self.dim(&format!("({subtitle})"))
        )
    }

    fn wrap(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

pub fn fmt_compact(n: usize) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn fmt_compact_signed(n: isize) -> String {
    if n < 0 {
        format!("-{}", fmt_compact(n.unsigned_abs()))
    } else {
        fmt_compact(n as usize)
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    pub width: usize,
    pub right: bool,
}

impl Cell {
    pub fn left(s: impl Into<String>) -> Self {
        let text = s.into();
        let width = text.chars().count();
        Self {
            text,
            width,
            right: false,
        }
    }

    pub fn right(s: impl Into<String>) -> Self {
        let text = s.into();
        let width = text.chars().count();
        Self {
            text,
            width,
            right: true,
        }
    }

    pub fn styled(text: String, visible_width: usize, right: bool) -> Self {
        Self {
            text,
            width: visible_width,
            right,
        }
    }
}

/// Renders a rounded Unicode box table in TTY mode, or cleanly aligned plain columns when piped.
pub fn render_table(theme: &Theme, headers: &[(&str, bool)], rows: &[Vec<Cell>]) -> Vec<String> {
    let ncols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|(h, _)| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(ncols) {
            if cell.width > widths[i] {
                widths[i] = cell.width;
            }
        }
    }

    let mut out = Vec::new();
    if !theme.color {
        let hdr_cells: Vec<String> = headers
            .iter()
            .enumerate()
            .map(|(i, (h, r))| pad_cell(h, h.chars().count(), widths[i], *r))
            .collect();
        out.push(format!("  {}", hdr_cells.join("  ")));
        for row in rows {
            let line_cells: Vec<String> = row
                .iter()
                .enumerate()
                .take(ncols)
                .map(|(i, c)| pad_cell(&c.text, c.width, widths[i], c.right))
                .collect();
            out.push(format!("  {}", line_cells.join("  ")));
        }
        return out;
    }

    let bar_seg = |w: usize| "─".repeat(w + 2);
    let top = format!(
        "╭{}╮",
        widths.iter().map(|&w| bar_seg(w)).collect::<Vec<_>>().join("┬")
    );
    let mid = format!(
        "├{}┤",
        widths.iter().map(|&w| bar_seg(w)).collect::<Vec<_>>().join("┼")
    );
    let bot = format!(
        "╰{}╯",
        widths.iter().map(|&w| bar_seg(w)).collect::<Vec<_>>().join("┴")
    );
    let v = theme.dim("│");

    out.push(theme.dim(&top));
    let hdr_parts: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, (h, r))| {
            let padded = pad_cell(h, h.chars().count(), widths[i], *r);
            format!(" {} ", theme.dim(&padded))
        })
        .collect();
    out.push(format!("{v}{}{v}", hdr_parts.join(&v)));
    out.push(theme.dim(&mid));

    for row in rows {
        let parts: Vec<String> = row
            .iter()
            .enumerate()
            .take(ncols)
            .map(|(i, c)| {
                let padded = pad_cell(&c.text, c.width, widths[i], c.right);
                format!(" {padded} ")
            })
            .collect();
        out.push(format!("{v}{}{v}", parts.join(&v)));
    }
    out.push(theme.dim(&bot));
    out
}

fn pad_cell(text: &str, visible_width: usize, target_width: usize, right: bool) -> String {
    let pad = target_width.saturating_sub(visible_width);
    if right {
        format!("{}{text}", " ".repeat(pad))
    } else {
        format!("{text}{}", " ".repeat(pad))
    }
}
