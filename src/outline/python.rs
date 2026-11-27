use super::compact_signature;

pub(super) fn outline_python(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut import_start: Option<usize> = None;
    let mut import_end = 0;
    let mut import_count = 0;

    let flush_imports = |out: &mut Vec<String>,
                         import_start: &mut Option<usize>,
                         import_end: usize,
                         import_count: &mut usize| {
        if let Some(start) = import_start.take() {
            let range = if start == import_end {
                format!("L{}", start + 1)
            } else {
                format!("L{}-{}", start + 1, import_end + 1)
            };
            out.push(format!("{range:>11}| # {} imports", *import_count));
            *import_count = 0;
        }
    };

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        let indent = raw.len() - raw.trim_start().len();

        if indent == 0 && (trimmed.starts_with("import ") || trimmed.starts_with("from ")) {
            if import_start.is_none() {
                import_start = Some(i);
            }
            import_end = i;
            import_count += 1;
            i += 1;
            continue;
        } else if !trimmed.is_empty() {
            flush_imports(&mut out, &mut import_start, import_end, &mut import_count);
        }

        if trimmed.starts_with("class ")
            || trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
        {
            let start = find_python_decorator_start(lines, i, indent);
            // Find end of indented block
            let mut end = i;
            let mut j = i + 1;
            while j < lines.len() {
                let lt = lines[j].trim();
                if lt.is_empty() || lt.starts_with('#') {
                    j += 1;
                    continue;
                }
                let j_indent = lines[j].len() - lines[j].trim_start().len();
                if j_indent <= indent {
                    break;
                }
                end = j;
                j += 1;
            }
            let range = if end > start {
                format!("L{}-{}", start + 1, end + 1)
            } else {
                format!("L{}", start + 1)
            };
            let pad = " ".repeat(indent.min(8));
            out.push(format!("{range:>11}| {pad}{}", compact_signature(trimmed)));
        }

        i += 1;
    }

    flush_imports(&mut out, &mut import_start, import_end, &mut import_count);
    out
}

/// Walk backward from a Python `class`/`def`/`async def` line to include any
/// immediately preceding `@decorator` lines (including stacked and multi-line decorators).
fn find_python_decorator_start(lines: &[&str], decl_idx: usize, decl_indent: usize) -> usize {
    let mut start = decl_idx;
    let mut k = decl_idx;
    let mut paren_depth: i32 = 0;

    while k > 0 {
        k -= 1;
        let line = lines[k];
        let trimmed = line.trim();
        let line_indent = line.len() - line.trim_start().len();

        if paren_depth > 0 {
            paren_depth += python_reverse_paren_delta(trimmed);
            if paren_depth <= 0 {
                paren_depth = 0;
                if trimmed.starts_with('@') && line_indent == decl_indent {
                    start = k;
                } else {
                    break;
                }
            }
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('@') && line_indent == decl_indent {
            start = k;
            continue;
        }

        let delta = python_reverse_paren_delta(trimmed);
        if delta > 0 && line_indent >= decl_indent {
            paren_depth = delta;
            continue;
        }

        break;
    }

    start
}

/// Net `(closing - opening)` paren/bracket/brace count on a Python line (ignoring `#` comments).
fn python_reverse_paren_delta(trimmed: &str) -> i32 {
    let mut delta = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if (in_single || in_double) && ch == '\\' {
            escaped = true;
            continue;
        }
        if !in_double && ch == '\'' {
            in_single = !in_single;
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double {
            continue;
        }
        if ch == '#' {
            break;
        }
        match ch {
            ')' | ']' | '}' => delta += 1,
            '(' | '[' | '{' => delta -= 1,
            _ => {}
        }
    }
    delta
}
