use crate::lang::{CommentStyle, Language};

const LONG_LINE_THRESHOLD: usize = 500;
const LONG_LINE_KEEP: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilteredItem {
    Line { num: usize, text: String },
    Omitted { start: usize, end: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStrength {
    Minimal,
    Aggressive,
}

pub fn filter_lines(
    lines: &[&str],
    start_line_num: usize,
    lang: Language,
    strength: FilterStrength,
) -> Vec<FilteredItem> {
    let stripped: Vec<(usize, Option<String>)> = match lang.comment_style() {
        CommentStyle::Slash { nested_block } => {
            strip_slash_comments(lines, start_line_num, nested_block, strength)
        }
        CommentStyle::Hash { python_docstrings } => {
            strip_hash_comments(lines, start_line_num, python_docstrings, strength)
        }
        CommentStyle::Sql => strip_sql_comments(lines, start_line_num, strength),
        CommentStyle::None => lines
            .iter()
            .enumerate()
            .map(|(i, l)| (start_line_num + i, Some(l.trim_end().to_string())))
            .collect(),
    };

    let mut items = collapse_items(stripped, strength);
    if lang == Language::Generic {
        items = dedup_consecutive_lines(items);
    }
    items
}

/// Collapse runs of >= 3 consecutive identical non-blank lines in plain-text/log files
/// into a single line annotated with `[×N]`.
fn dedup_consecutive_lines(items: Vec<FilteredItem>) -> Vec<FilteredItem> {
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0;
    while i < items.len() {
        if let FilteredItem::Line { num, ref text } = items[i]
            && !text.trim().is_empty()
        {
            let mut j = i + 1;
            while j < items.len() {
                if let FilteredItem::Line {
                    text: ref next_text,
                    ..
                } = items[j]
                    && next_text == text
                {
                    j += 1;
                } else {
                    break;
                }
            }
            let count = j - i;
            if count >= 3 {
                out.push(FilteredItem::Line {
                    num,
                    text: format!("{text}  [×{count}]"),
                });
                i = j;
                continue;
            }
        }
        out.push(items[i].clone());
        i += 1;
    }
    out
}

fn truncate_long_line(s: String) -> String {
    if s.len() <= LONG_LINE_THRESHOLD {
        return s;
    }
    let mut end = LONG_LINE_KEEP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let remaining = s.len().saturating_sub(end);
    format!("{} … [+{remaining} chars]", &s[..end])
}

fn collapse_items(
    rows: Vec<(usize, Option<String>)>,
    strength: FilterStrength,
) -> Vec<FilteredItem> {
    let mut out = Vec::with_capacity(rows.len());
    let mut omit_start: Option<usize> = None;
    let mut omit_end: usize = 0;
    let mut prev_was_blank = false;

    let flush_omit =
        |out: &mut Vec<FilteredItem>, omit_start: &mut Option<usize>, omit_end: usize| {
            if let Some(start) = omit_start.take() {
                let count = omit_end.saturating_sub(start) + 1;
                if count >= 3 {
                    out.push(FilteredItem::Omitted {
                        start,
                        end: omit_end,
                    });
                }
            }
        };

    for (num, maybe_text) in rows {
        match maybe_text {
            None => {
                // Explicitly dropped (comment-only line)
                if omit_start.is_none() {
                    omit_start = Some(num);
                }
                omit_end = num;
            }
            Some(text) => {
                let is_blank = text.trim().is_empty();
                if is_blank {
                    match strength {
                        FilterStrength::Aggressive => {
                            if omit_start.is_none() {
                                omit_start = Some(num);
                            }
                            omit_end = num;
                        }
                        FilterStrength::Minimal => {
                            if prev_was_blank || omit_start.is_some() {
                                if omit_start.is_none() {
                                    omit_start = Some(num);
                                }
                                omit_end = num;
                            } else {
                                prev_was_blank = true;
                                out.push(FilteredItem::Line {
                                    num,
                                    text: String::new(),
                                });
                            }
                        }
                    }
                } else {
                    flush_omit(&mut out, &mut omit_start, omit_end);
                    prev_was_blank = false;
                    out.push(FilteredItem::Line {
                        num,
                        text: truncate_long_line(text),
                    });
                }
            }
        }
    }

    flush_omit(&mut out, &mut omit_start, omit_end);

    // Trim leading/trailing blank lines
    while matches!(out.first(), Some(FilteredItem::Line { text, .. }) if text.is_empty()) {
        out.remove(0);
    }
    while matches!(out.last(), Some(FilteredItem::Line { text, .. }) if text.is_empty()) {
        out.pop();
    }

    out
}

fn is_kept_slash_line_comment(comment: &str) -> bool {
    if comment.starts_with("///") || comment.starts_with("//!") {
        return true;
    }
    let body = comment.trim_start_matches('/').trim_start();
    body.starts_with("MARK:")
        || body.starts_with("TODO:")
        || body.starts_with("FIXME:")
        || body.starts_with("SAFETY:")
}

fn is_regex_preceding_char(ch: Option<char>) -> bool {
    match ch {
        None => true,
        Some(c) => matches!(
            c,
            '(' | '['
                | '{'
                | ','
                | '='
                | ':'
                | ';'
                | '!'
                | '&'
                | '|'
                | '?'
                | '+'
                | '-'
                | '*'
                | '%'
                | '^'
                | '~'
                | '<'
                | '>'
                | '\n'
        ),
    }
}

fn strip_slash_comments(
    lines: &[&str],
    start_line_num: usize,
    nested_block: bool,
    strength: FilterStrength,
) -> Vec<(usize, Option<String>)> {
    let mut results = Vec::with_capacity(lines.len());
    let mut block_depth: usize = 0;
    let mut block_is_doc = false;
    let mut in_triple_double = false; // Swift """..."""
    let mut in_backtick = false; // JS/TS/Go `...`
    let mut raw_hashes: Option<usize> = None; // Swift #"..."# or Rust r#"..."#

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_num = start_line_num + idx;
        let orig_has_non_ws = !raw_line.trim().is_empty();
        let bytes = raw_line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut out = String::with_capacity(len);
        let mut in_double = false;
        let mut in_single = false;
        let mut in_regex = false;
        let mut in_regex_class = false;
        let mut last_non_ws: Option<char> = None;

        while i < len {
            if let Some(hashes) = raw_hashes {
                if bytes[i] == b'"'
                    && i + hashes < len
                    && bytes[i + 1..=i + hashes].iter().all(|&b| b == b'#')
                {
                    out.push('"');
                    for _ in 0..hashes {
                        out.push('#');
                    }
                    raw_hashes = None;
                    i += 1 + hashes;
                    continue;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if block_depth > 0 {
                if nested_block && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    block_depth += 1;
                    if strength == FilterStrength::Minimal && block_is_doc {
                        out.push_str("/*");
                    }
                    i += 2;
                    continue;
                }
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    block_depth -= 1;
                    if strength == FilterStrength::Minimal && block_is_doc {
                        out.push_str("*/");
                    }
                    if block_depth == 0 {
                        block_is_doc = false;
                    }
                    i += 2;
                    continue;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                if strength == FilterStrength::Minimal && block_is_doc {
                    out.push(ch);
                }
                i += ch.len_utf8();
                continue;
            }

            if in_triple_double {
                if i + 2 < len && &bytes[i..i + 3] == b"\"\"\"" {
                    out.push_str("\"\"\"");
                    in_triple_double = false;
                    i += 3;
                    continue;
                }
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_backtick {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'`' {
                    in_backtick = false;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_regex {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'[' {
                    in_regex_class = true;
                } else if bytes[i] == b']' {
                    in_regex_class = false;
                } else if bytes[i] == b'/' && !in_regex_class {
                    in_regex = false;
                    last_non_ws = Some('/');
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_double {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'"' {
                    in_double = false;
                    last_non_ws = Some('"');
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_single {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'\'' {
                    in_single = false;
                    last_non_ws = Some('\'');
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            // Detect Rust raw strings `r#"..."#` / `br#"..."#` and Swift raw strings `#"..."#`
            if bytes[i] == b'r' || bytes[i] == b'#' {
                let mut j = i;
                let mut has_r = false;
                if bytes[j] == b'r' {
                    let prev_ident =
                        i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
                    if !prev_ident {
                        has_r = true;
                        j += 1;
                    }
                }
                let hash_start = j;
                while j < len && bytes[j] == b'#' {
                    j += 1;
                }
                let h_count = j - hash_start;
                if h_count > 0 && j < len && bytes[j] == b'"' && (has_r || nested_block) {
                    out.push_str(&raw_line[i..=j]);
                    raw_hashes = Some(h_count);
                    i = j + 1;
                    continue;
                }
            }

            // Not inside any string or comment
            if i + 2 < len && &bytes[i..i + 3] == b"\"\"\"" {
                in_triple_double = true;
                out.push_str("\"\"\"");
                i += 3;
                continue;
            }
            if bytes[i] == b'"' {
                in_double = true;
                out.push('"');
                i += 1;
                continue;
            }
            if bytes[i] == b'\'' {
                // Avoid treating Rust lifetimes like `'a` as char literals
                let is_lifetime = i + 1 < len
                    && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
                    && !(i + 2 < len && bytes[i + 2] == b'\'');
                if !is_lifetime {
                    in_single = true;
                }
                out.push('\'');
                i += 1;
                continue;
            }
            if bytes[i] == b'`' {
                in_backtick = true;
                out.push('`');
                i += 1;
                continue;
            }

            // Line comment `//`
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                let rest = &raw_line[i..];
                if strength == FilterStrength::Minimal && is_kept_slash_line_comment(rest) {
                    out.push_str(rest);
                }
                break;
            }

            // Block comment `/*`
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                let is_empty_block = i + 3 < len && bytes[i + 2] == b'*' && bytes[i + 3] == b'/';
                let is_doc = !is_empty_block
                    && i + 2 < len
                    && (bytes[i + 2] == b'*' || bytes[i + 2] == b'!');
                block_depth = 1;
                block_is_doc = is_doc;
                if strength == FilterStrength::Minimal && block_is_doc {
                    out.push_str("/*");
                }
                i += 2;
                continue;
            }

            // JS/TS regex literal `/pattern/flags` (when not `//` or `/*` and preceded by operator/delimiter)
            if !nested_block
                && bytes[i] == b'/'
                && is_regex_preceding_char(last_non_ws)
                && i + 1 < len
                && bytes[i + 1] != b'/'
                && bytes[i + 1] != b'*'
                && raw_line[i + 1..].contains('/')
            {
                in_regex = true;
                in_regex_class = false;
                out.push('/');
                i += 1;
                continue;
            }

            let ch = raw_line[i..].chars().next().unwrap();
            if !ch.is_whitespace() {
                last_non_ws = Some(ch);
            }
            out.push(ch);
            i += ch.len_utf8();
        }

        let trimmed_end = out.trim_end().to_string();
        if orig_has_non_ws && trimmed_end.trim().is_empty() {
            results.push((line_num, None));
        } else {
            results.push((line_num, Some(trimmed_end)));
        }
    }

    results
}

fn strip_hash_comments(
    lines: &[&str],
    start_line_num: usize,
    python_docstrings: bool,
    strength: FilterStrength,
) -> Vec<(usize, Option<String>)> {
    let mut results = Vec::with_capacity(lines.len());
    let mut triple_quote: Option<&[u8; 3]> = None;
    let mut skipping_docstring = false;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_num = start_line_num + idx;
        let trimmed = raw_line.trim();
        let orig_has_non_ws = !trimmed.is_empty();

        // Keep shebang on line 1 in minimal mode
        if line_num == 1 && raw_line.starts_with("#!") {
            if strength == FilterStrength::Minimal {
                results.push((line_num, Some(raw_line.trim_end().to_string())));
            } else {
                results.push((line_num, None));
            }
            continue;
        }

        // In aggressive mode for Python, strip standalone docstrings
        if python_docstrings && strength == FilterStrength::Aggressive {
            if skipping_docstring {
                if let Some(q) = triple_quote {
                    let q_str = std::str::from_utf8(q).unwrap();
                    if raw_line.contains(q_str) {
                        skipping_docstring = false;
                        triple_quote = None;
                    }
                }
                results.push((line_num, None));
                continue;
            }
            if triple_quote.is_none()
                && (trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''"))
            {
                let q: &[u8; 3] = if trimmed.starts_with("\"\"\"") {
                    b"\"\"\""
                } else {
                    b"'''"
                };
                let q_str = std::str::from_utf8(q).unwrap();
                let after = &trimmed[3..];
                if !after.contains(q_str) {
                    skipping_docstring = true;
                    triple_quote = Some(q);
                }
                results.push((line_num, None));
                continue;
            }
        }

        let bytes = raw_line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut out = String::with_capacity(len);
        let mut in_double = false;
        let mut in_single = false;

        while i < len {
            if let Some(q) = triple_quote {
                if i + 2 < len && &bytes[i..i + 3] == q {
                    out.push_str(std::str::from_utf8(q).unwrap());
                    triple_quote = None;
                    i += 3;
                    continue;
                }
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_double {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'"' {
                    in_double = false;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if in_single {
                if bytes[i] == b'\\' && i + 1 < len {
                    let ch = raw_line[i + 1..].chars().next().unwrap();
                    out.push('\\');
                    out.push(ch);
                    i += 1 + ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'\'' {
                    in_single = false;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }

            if python_docstrings && i + 2 < len && &bytes[i..i + 3] == b"\"\"\"" {
                triple_quote = Some(b"\"\"\"");
                out.push_str("\"\"\"");
                i += 3;
                continue;
            }
            if python_docstrings && i + 2 < len && &bytes[i..i + 3] == b"'''" {
                triple_quote = Some(b"'''");
                out.push_str("'''");
                i += 3;
                continue;
            }
            if bytes[i] == b'"' {
                in_double = true;
                out.push('"');
                i += 1;
                continue;
            }
            if bytes[i] == b'\'' {
                in_single = true;
                out.push('\'');
                i += 1;
                continue;
            }

            if bytes[i] == b'#' {
                let rest = &raw_line[i..];
                let body = rest.trim_start_matches('#').trim_start();
                if strength == FilterStrength::Minimal
                    && (body.starts_with("TODO:") || body.starts_with("FIXME:"))
                {
                    out.push_str(rest);
                }
                break;
            }

            let ch = raw_line[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }

        let trimmed_end = out.trim_end().to_string();
        if orig_has_non_ws && trimmed_end.trim().is_empty() {
            results.push((line_num, None));
        } else {
            results.push((line_num, Some(trimmed_end)));
        }
    }

    results
}

fn strip_sql_comments(
    lines: &[&str],
    start_line_num: usize,
    _strength: FilterStrength,
) -> Vec<(usize, Option<String>)> {
    let mut results = Vec::with_capacity(lines.len());
    let mut in_block = false;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_num = start_line_num + idx;
        let orig_has_non_ws = !raw_line.trim().is_empty();
        let bytes = raw_line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut out = String::with_capacity(len);
        let mut in_single = false;

        while i < len {
            if in_block {
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    in_block = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            if in_single {
                if bytes[i] == b'\'' {
                    if i + 1 < len && bytes[i + 1] == b'\'' {
                        out.push_str("''");
                        i += 2;
                        continue;
                    }
                    in_single = false;
                }
                let ch = raw_line[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }
            if bytes[i] == b'\'' {
                in_single = true;
                out.push('\'');
                i += 1;
                continue;
            }
            if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                break;
            }
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                in_block = true;
                i += 2;
                continue;
            }
            let ch = raw_line[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }

        let trimmed_end = out.trim_end().to_string();
        if orig_has_non_ws && trimmed_end.trim().is_empty() {
            results.push((line_num, None));
        } else {
            results.push((line_num, Some(trimmed_end)));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept_lines(items: &[FilteredItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|it| match it {
                FilteredItem::Line { text, .. } => Some(text.clone()),
                FilteredItem::Omitted { .. } => None,
            })
            .collect()
    }

    #[test]
    fn rust_raw_strings_lifetimes_and_doc_comments() {
        let lines = [
            "//! Module doc kept in minimal",
            "/// Function doc kept in minimal",
            "// regular comment stripped",
            "pub fn parse<'a>(s: &'a str) -> &'a str { // trailing comment",
            "    let pat = r#\"// not a comment /* also not */\"#;",
            "    let empty = /**/ s;",
            "    pat",
            "}",
        ];
        let min = filter_lines(&lines, 1, Language::Rust, FilterStrength::Minimal);
        let min_text = kept_lines(&min).join("\n");
        assert!(min_text.contains("//! Module doc kept in minimal"));
        assert!(min_text.contains("/// Function doc kept in minimal"));
        assert!(!min_text.contains("regular comment stripped"));
        assert!(!min_text.contains("trailing comment"));
        assert!(min_text.contains("pub fn parse<'a>(s: &'a str) -> &'a str {"));
        assert!(min_text.contains("r#\"// not a comment /* also not */\"#"));
        assert!(min_text.contains("let empty =  s;"));

        let agg = filter_lines(&lines, 1, Language::Rust, FilterStrength::Aggressive);
        let agg_text = kept_lines(&agg).join("\n");
        assert!(!agg_text.contains("Module doc"));
        assert!(!agg_text.contains("Function doc"));
        assert!(agg_text.contains("r#\"// not a comment /* also not */\"#"));
    }

    #[test]
    fn js_regex_literals_division_and_template_strings() {
        let lines = [
            "const re = /https?:\\/\\/[a-z/]+/g; // strip me",
            "const ratio = a / b; // division comment",
            "const tpl = `http://example.com/${a} // not a comment`;",
            "/* block comment",
            "   line 2",
            "   line 3 */",
            "return re;",
        ];
        let items = filter_lines(&lines, 1, Language::TypeScript, FilterStrength::Minimal);
        let text = kept_lines(&items).join("\n");
        assert!(
            text.contains("const re = /https?:\\/\\/[a-z/]+/g;"),
            "got: {text}"
        );
        assert!(!text.contains("strip me"), "got: {text}");
        assert!(text.contains("const ratio = a / b;"), "got: {text}");
        assert!(!text.contains("division comment"), "got: {text}");
        assert!(
            text.contains("http://example.com/${a} // not a comment"),
            "got: {text}"
        );
        assert!(
            items
                .iter()
                .any(|it| matches!(it, FilteredItem::Omitted { start: 4, end: 6 }))
        );
    }

    #[test]
    fn sql_comments_and_escaped_single_quotes() {
        let lines = [
            "-- header comment",
            "SELECT 'it''s -- not a comment' /* inline block */ AS msg",
            "/* multi-line",
            "   block comment */",
            "FROM users; -- trailing",
        ];
        let items = filter_lines(&lines, 1, Language::Sql, FilterStrength::Minimal);
        let text = kept_lines(&items).join("\n");
        assert!(!text.contains("header comment"));
        assert!(!text.contains("inline block"));
        assert!(!text.contains("multi-line"));
        assert!(!text.contains("trailing"));
        assert!(text.contains("'it''s -- not a comment'"));
        assert!(text.contains("FROM users;"));
    }

    #[test]
    fn python_todo_fixme_and_multiline_triple_quotes() {
        let lines = [
            "# regular comment",
            "# TODO: keep in minimal",
            "# FIXME: also keep in minimal",
            "s = '''",
            "# not a comment inside triple single quotes",
            "'''",
            "x = 1  # trailing comment",
        ];
        let min = filter_lines(&lines, 1, Language::Python, FilterStrength::Minimal);
        let min_text = kept_lines(&min).join("\n");
        assert!(!min_text.contains("regular comment"));
        assert!(min_text.contains("# TODO: keep in minimal"));
        assert!(min_text.contains("# FIXME: also keep in minimal"));
        assert!(min_text.contains("# not a comment inside triple single quotes"));
        assert!(min_text.contains("x = 1"));
        assert!(!min_text.contains("trailing comment"));
    }

    #[test]
    fn long_line_truncation_respects_utf8_boundaries() {
        let long = "é".repeat(300); // 600 bytes > LONG_LINE_THRESHOLD (500)
        let lines = [long.as_str()];
        let items = filter_lines(&lines, 1, Language::Generic, FilterStrength::Minimal);
        let text = kept_lines(&items).join("\n");
        assert!(text.contains("… [+"));
        assert!(text.len() < long.len());
    }
}
