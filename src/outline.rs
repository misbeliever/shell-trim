use crate::lang::Language;

/// Produces a compact structural outline of a file with exact 1-based line ranges.
pub fn extract_outline(lines: &[&str], lang: Language) -> Vec<String> {
    match lang {
        Language::Python => outline_python(lines),
        Language::Markdown => outline_markdown(lines),
        Language::Json => outline_json(lines),
        Language::Yaml => outline_yaml(lines),
        Language::Swift
        | Language::TypeScript
        | Language::JavaScript
        | Language::Rust
        | Language::Go
        | Language::CLike => outline_brace_lang(lines, lang),
        _ => Vec::new(),
    }
}

/// Find 1-based inclusive `(start, end)` line ranges for declarations matching `symbol`.
/// Prefers exact identifier matches; falls back to case-insensitive substring matches.
pub fn find_symbol_ranges(lines: &[&str], lang: Language, symbol: &str) -> Vec<(usize, usize)> {
    let entries = extract_outline(lines, lang);
    let mut exact = Vec::new();
    let mut fuzzy = Vec::new();
    let sym_lower = symbol.to_lowercase();

    for entry in entries {
        let Some((range_part, decl)) = entry.split_once('|') else {
            continue;
        };
        let decl_trimmed = decl.trim();
        if decl_trimmed.starts_with("//") || decl_trimmed.starts_with('#') && lang != Language::Markdown {
            continue;
        }
        let Some((start, end)) = parse_outline_range(range_part.trim()) else {
            continue;
        };
        if has_word_match(decl_trimmed, symbol) {
            exact.push((start, end));
        } else if decl_trimmed.to_lowercase().contains(&sym_lower) {
            fuzzy.push((start, end));
        }
    }

    if !exact.is_empty() {
        exact
    } else {
        fuzzy
    }
}

/// Suggest up to `max_results` closest outline declarations when `-s <symbol>` misses,
/// formatted as `[Lstart-Lend] <decl>` so agents can jump directly instead of reading raw.
pub fn suggest_symbols(
    lines: &[&str],
    lang: Language,
    symbol: &str,
    max_results: usize,
) -> Vec<String> {
    let entries = extract_outline(lines, lang);
    let q_lower = symbol.to_lowercase();
    let q_words = split_ident_words(symbol);
    let mut scored: Vec<(usize, String)> = Vec::new();

    for entry in entries {
        let Some((range_part, decl)) = entry.split_once('|') else {
            continue;
        };
        let decl_trimmed = decl.trim();
        if decl_trimmed.starts_with("//") || (decl_trimmed.starts_with('#') && lang != Language::Markdown) {
            continue;
        }
        let Some((start, end)) = parse_outline_range(range_part.trim()) else {
            continue;
        };

        let d_lower = decl_trimmed.to_lowercase();
        let d_words = split_ident_words(decl_trimmed);

        // Score higher for shared camelCase/snake_case words, prefix matches, and low edit distance
        let mut score: usize = 0;
        for qw in &q_words {
            if d_words.iter().any(|dw| dw == qw) {
                score += 40;
            } else if d_words.iter().any(|dw| dw.starts_with(qw) || qw.starts_with(dw)) {
                score += 20;
            } else if d_lower.contains(qw.as_str()) {
                score += 10;
            }
        }
        // Check best identifier word edit distance
        let best_dist = d_words
            .iter()
            .map(|dw| levenshtein(&q_lower, dw))
            .min()
            .unwrap_or(usize::MAX);
        if best_dist <= 3 {
            score += (4 - best_dist) * 15;
        }
        // Also compare whole compact identifier (removing underscores)
        let q_compact: String = q_lower.chars().filter(|c| c.is_alphanumeric()).collect();
        if !q_compact.is_empty() {
            for tok in decl_trimmed.split(|c: char| !c.is_alphanumeric() && c != '_') {
                let t_compact: String = tok.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
                if t_compact.len() >= 2 {
                    let d = levenshtein(&q_compact, &t_compact);
                    let max_len = q_compact.len().max(t_compact.len());
                    if d * 2 <= max_len {
                        score += (max_len.saturating_sub(d)) * 6;
                    }
                }
            }
        }

        if score > 0 {
            scored.push((score, format!("[L{start}-L{end}] {decl_trimmed}")));
        }
    }

    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().take(max_results).map(|(_, s)| s).collect()
}

fn split_ident_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in s.chars() {
        if !ch.is_alphanumeric() {
            if cur.len() >= 2 {
                words.push(cur.to_lowercase());
            }
            cur.clear();
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            if cur.len() >= 2 {
                words.push(cur.to_lowercase());
            }
            cur.clear();
        }
        prev_lower = ch.is_lowercase();
        cur.push(ch);
    }
    if cur.len() >= 2 {
        words.push(cur.to_lowercase());
    }
    words
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Find the tightest enclosing declaration `(start, end)` (1-based inclusive) containing `line_1based`,
/// bounded to `max_span` lines so it returns a function/method rather than a giant enclosing class.
pub fn enclosing_block(
    lines: &[&str],
    lang: Language,
    line_1based: usize,
    max_span: usize,
) -> Option<(usize, usize)> {
    let entries = extract_outline(lines, lang);
    let mut best: Option<(usize, usize)> = None;
    let mut best_span = usize::MAX;

    for entry in entries {
        let Some((range_part, decl)) = entry.split_once('|') else {
            continue;
        };
        let decl_trimmed = decl.trim();
        if decl_trimmed.starts_with("//") {
            continue;
        }
        let Some((start, end)) = parse_outline_range(range_part.trim()) else {
            continue;
        };
        if start <= line_1based && line_1based <= end {
            let span = end.saturating_sub(start) + 1;
            if span <= max_span && span < best_span {
                best = Some((start, end));
                best_span = span;
            }
        }
    }
    best
}

fn parse_outline_range(s: &str) -> Option<(usize, usize)> {
    let rest = s.strip_prefix('L')?;
    if let Some((a, b)) = rest.split_once('-') {
        Some((a.parse().ok()?, b.parse().ok()?))
    } else {
        let n: usize = rest.parse().ok()?;
        Some((n, n))
    }
}

fn has_word_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let mut pos = 0;
    while let Some(idx) = haystack[pos..].find(needle) {
        let start = pos + idx;
        let end = start + needle.len();
        let left_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let right_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        pos = start + 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Pre-compute brace delta (`{` minus `}`) and whether `{` appeared on each line,
/// ignoring braces inside strings and comments.
fn scan_braces(lines: &[&str], nested_block: bool) -> Vec<(i32, bool)> {
    let mut out = Vec::with_capacity(lines.len());
    let mut block_depth: usize = 0;
    let mut in_triple = false;
    let mut in_backtick = false;

    for raw in lines {
        let b = raw.as_bytes();
        let len = b.len();
        let mut i = 0;
        let mut delta: i32 = 0;
        let mut saw_open = false;
        let mut in_double = false;
        let mut in_single = false;

        while i < len {
            if block_depth > 0 {
                if nested_block && i + 1 < len && b[i] == b'/' && b[i + 1] == b'*' {
                    block_depth += 1;
                    i += 2;
                    continue;
                }
                if i + 1 < len && b[i] == b'*' && b[i + 1] == b'/' {
                    block_depth -= 1;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            if in_triple {
                if i + 2 < len && &b[i..i + 3] == b"\"\"\"" {
                    in_triple = false;
                    i += 3;
                    continue;
                }
                if b[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            if in_backtick {
                if b[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if b[i] == b'`' {
                    in_backtick = false;
                }
                i += 1;
                continue;
            }
            if in_double {
                if b[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    in_double = false;
                }
                i += 1;
                continue;
            }
            if in_single {
                if b[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if b[i] == b'\'' {
                    in_single = false;
                }
                i += 1;
                continue;
            }

            if i + 2 < len && &b[i..i + 3] == b"\"\"\"" {
                in_triple = true;
                i += 3;
                continue;
            }
            if b[i] == b'"' {
                in_double = true;
                i += 1;
                continue;
            }
            if b[i] == b'\'' {
                let is_lifetime = i + 1 < len
                    && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_')
                    && !(i + 2 < len && b[i + 2] == b'\'');
                if !is_lifetime {
                    in_single = true;
                }
                i += 1;
                continue;
            }
            if b[i] == b'`' {
                in_backtick = true;
                i += 1;
                continue;
            }
            if i + 1 < len && b[i] == b'/' && b[i + 1] == b'/' {
                break;
            }
            if i + 1 < len && b[i] == b'/' && b[i + 1] == b'*' {
                block_depth = 1;
                i += 2;
                continue;
            }
            if b[i] == b'{' {
                delta += 1;
                saw_open = true;
            } else if b[i] == b'}' {
                delta -= 1;
            }
            i += 1;
        }

        out.push((delta, saw_open));
    }

    out
}

/// Find the 0-based closing line of a block starting at `start_idx` at `base_depth`.
fn find_block_end(brace_info: &[(i32, bool)], start_idx: usize, base_depth: i32) -> usize {
    let mut depth = base_depth;
    let mut opened = false;
    for (i, &(delta, saw_open)) in brace_info.iter().enumerate().skip(start_idx) {
        if saw_open {
            opened = true;
        }
        depth += delta;
        if !opened && depth < base_depth {
            // Crossed an enclosing closing brace before opening one (e.g. protocol requirement)
            return start_idx;
        }
        if opened && depth <= base_depth {
            return i;
        }
        // If no `{` appeared within 20 lines (e.g. protocol method declaration or typealias), stop.
        if !opened && i >= start_idx + 20 {
            return start_idx;
        }
    }
    start_idx
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclKind {
    Import,
    Container, // struct, class, enum, protocol, extension, impl, trait, interface, mod
    Callable,  // func, fn, init, function, method, arrow fn
    Marker,    // // MARK: ...
    Property,  // typealias, const, top-level var/let
    None,
}

fn classify_brace_line(trimmed: &str, lang: Language, depth: i32) -> DeclKind {
    if trimmed.is_empty() {
        return DeclKind::None;
    }

    if lang == Language::Swift && trimmed.starts_with("//") {
        let body = trimmed.trim_start_matches('/').trim_start();
        if body.starts_with("MARK:") {
            return DeclKind::Marker;
        }
        return DeclKind::None;
    }

    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
        return DeclKind::None;
    }

    match lang {
        Language::Swift => classify_swift(trimmed, depth),
        Language::Rust => classify_rust(trimmed, depth),
        Language::TypeScript | Language::JavaScript => classify_ts_js(trimmed, depth),
        Language::Go => classify_go(trimmed, depth),
        Language::CLike => classify_c_like(trimmed, depth),
        _ => DeclKind::None,
    }
}

fn strip_leading_words<'a>(mut s: &'a str, prefixes: &[&str]) -> &'a str {
    loop {
        let mut changed = false;
        s = s.trim_start();
        // Skip Swift/Java/TS attributes like @MainActor, @Observable, @discardableResult
        if s.starts_with('@')
            && let Some(end) = skip_attribute(s)
        {
            s = &s[end..];
            continue;
        }
        for &p in prefixes {
            if let Some(rest) = s.strip_prefix(p) {
                let next_ok = rest.is_empty()
                    || rest.starts_with(char::is_whitespace)
                    || rest.starts_with('(');
                if next_ok {
                    // Handle Swift `private(set)` or Rust `pub(crate)`
                    if rest.starts_with('(') {
                        if let Some(close) = rest.find(')') {
                            s = &rest[close + 1..];
                            changed = true;
                            break;
                        }
                    } else {
                        s = rest;
                        changed = true;
                        break;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    s.trim_start()
}

fn skip_attribute(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'@') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
    {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'(' {
        let mut paren = 1;
        i += 1;
        while i < bytes.len() && paren > 0 {
            if bytes[i] == b'(' {
                paren += 1;
            } else if bytes[i] == b')' {
                paren -= 1;
            }
            i += 1;
        }
    }
    Some(i)
}

fn starts_with_kw(s: &str, kw: &str) -> bool {
    if let Some(rest) = s.strip_prefix(kw) {
        rest.is_empty()
            || !rest
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
    } else {
        false
    }
}

fn classify_swift(trimmed: &str, depth: i32) -> DeclKind {
    if trimmed.starts_with("import ") || trimmed.starts_with("@testable import ") {
        return DeclKind::Import;
    }
    let core = strip_leading_words(
        trimmed,
        &[
            "public",
            "private",
            "fileprivate",
            "internal",
            "open",
            "final",
            "static",
            "class",
            "override",
            "mutating",
            "nonmutating",
            "convenience",
            "required",
            "indirect",
            "nonisolated",
            "isolated",
            "lazy",
            "weak",
            "unowned",
            "async",
            "distributed",
            "package",
        ],
    );
    // Note: "class" was in modifiers (for `class func`); check original for `class Foo`
    let core_no_class_mod = strip_leading_words(
        trimmed,
        &[
            "public",
            "private",
            "fileprivate",
            "internal",
            "open",
            "final",
            "indirect",
            "nonisolated",
            "distributed",
            "package",
        ],
    );

    if starts_with_kw(core_no_class_mod, "struct")
        || starts_with_kw(core_no_class_mod, "class")
        || starts_with_kw(core_no_class_mod, "actor")
        || starts_with_kw(core_no_class_mod, "enum")
        || starts_with_kw(core_no_class_mod, "protocol")
        || starts_with_kw(core_no_class_mod, "extension")
    {
        // Guard against `class func` or `class var`
        if starts_with_kw(core_no_class_mod, "class") {
            let after_class = core_no_class_mod["class".len()..].trim_start();
            if starts_with_kw(after_class, "func")
                || starts_with_kw(after_class, "var")
                || starts_with_kw(after_class, "let")
            {
                return if starts_with_kw(after_class, "func") {
                    DeclKind::Callable
                } else {
                    DeclKind::Property
                };
            }
        }
        return DeclKind::Container;
    }

    if starts_with_kw(core, "func")
        || starts_with_kw(core, "init")
        || starts_with_kw(core, "deinit")
        || starts_with_kw(core, "subscript")
    {
        return DeclKind::Callable;
    }

    if depth == 0 && starts_with_kw(core, "typealias") {
        return DeclKind::Property;
    }

    // Computed properties like `var body: some View {` at depth 1
    if depth == 1 && starts_with_kw(core, "var") && trimmed.contains("body:") {
        return DeclKind::Callable;
    }

    DeclKind::None
}

fn classify_rust(trimmed: &str, depth: i32) -> DeclKind {
    if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
        return DeclKind::Import;
    }
    let core = strip_leading_words(
        trimmed,
        &["pub", "async", "unsafe", "const", "extern", "default"],
    );
    if starts_with_kw(core, "struct")
        || starts_with_kw(core, "enum")
        || starts_with_kw(core, "trait")
        || starts_with_kw(core, "impl")
        || (starts_with_kw(core, "mod") && trimmed.contains('{'))
    {
        return DeclKind::Container;
    }
    if starts_with_kw(core, "fn") || starts_with_kw(core, "macro_rules!") {
        return DeclKind::Callable;
    }
    if depth == 0
        && (starts_with_kw(core, "type")
            || starts_with_kw(core, "mod")
            || starts_with_kw(core, "static"))
    {
        return DeclKind::Property;
    }
    DeclKind::None
}

fn classify_ts_js(trimmed: &str, depth: i32) -> DeclKind {
    if trimmed.starts_with("import ") || trimmed.starts_with("import{") {
        return DeclKind::Import;
    }
    let core = strip_leading_words(
        trimmed,
        &[
            "export",
            "default",
            "declare",
            "abstract",
            "async",
            "public",
            "private",
            "protected",
            "static",
            "readonly",
            "override",
        ],
    );
    if starts_with_kw(core, "class")
        || starts_with_kw(core, "interface")
        || starts_with_kw(core, "enum")
        || starts_with_kw(core, "namespace")
    {
        return DeclKind::Container;
    }
    if starts_with_kw(core, "function") {
        return DeclKind::Callable;
    }
    if depth == 0 && starts_with_kw(core, "type") {
        return DeclKind::Property;
    }
    if depth == 0
        && (starts_with_kw(core, "const") || starts_with_kw(core, "let"))
        && (trimmed.contains("=>") || trimmed.contains("function("))
    {
        return DeclKind::Callable;
    }
    // Class methods at depth 1: `foo(bar: string) {` or `constructor(...) {`
    if depth == 1
        && !trimmed.starts_with("if ")
        && !trimmed.starts_with("for ")
        && !trimmed.starts_with("while ")
        && !trimmed.starts_with("switch ")
        && !trimmed.starts_with("catch")
        && !trimmed.starts_with("return")
        && let Some(paren) = core.find('(')
    {
        let name = core[..paren].trim();
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == ' ')
            && (trimmed.ends_with('{') || trimmed.contains("):"))
        {
            return DeclKind::Callable;
        }
    }
    DeclKind::None
}

fn classify_go(trimmed: &str, depth: i32) -> DeclKind {
    if trimmed.starts_with("import ") || trimmed == "import (" {
        return DeclKind::Import;
    }
    if depth == 0 && trimmed.starts_with("package ") {
        return DeclKind::Property;
    }
    if depth == 0 && starts_with_kw(trimmed, "type") {
        if trimmed.contains("struct") || trimmed.contains("interface") {
            return DeclKind::Container;
        }
        return DeclKind::Property;
    }
    if depth == 0 && starts_with_kw(trimmed, "func") {
        return DeclKind::Callable;
    }
    DeclKind::None
}

fn classify_c_like(trimmed: &str, depth: i32) -> DeclKind {
    if trimmed.starts_with("#include ") || trimmed.starts_with("#import ") {
        return DeclKind::Import;
    }
    let core = strip_leading_words(
        trimmed,
        &[
            "public",
            "private",
            "protected",
            "static",
            "final",
            "abstract",
            "virtual",
            "inline",
            "extern",
            "override",
            "async",
            "sealed",
            "open",
            "data",
            "internal",
        ],
    );
    if starts_with_kw(core, "class")
        || starts_with_kw(core, "struct")
        || starts_with_kw(core, "enum")
        || starts_with_kw(core, "interface")
        || starts_with_kw(core, "namespace")
        || trimmed.starts_with("@interface")
        || trimmed.starts_with("@implementation")
        || trimmed.starts_with("@protocol")
    {
        return DeclKind::Container;
    }
    // Obj-C methods `- (void)foo` / `+ (instancetype)bar`
    if (trimmed.starts_with("- (") || trimmed.starts_with("+ (")) && depth <= 1 {
        return DeclKind::Callable;
    }
    // Top-level or class-level C/C++/Java/Kotlin function with `(` and `{`
    if depth <= 1
        && trimmed.contains('(')
        && trimmed.ends_with('{')
        && !starts_with_kw(core, "if")
        && !starts_with_kw(core, "for")
        && !starts_with_kw(core, "while")
        && !starts_with_kw(core, "switch")
        && !starts_with_kw(core, "catch")
    {
        return DeclKind::Callable;
    }
    DeclKind::None
}

fn compact_signature(line: &str) -> String {
    let trimmed = line.trim();
    let clean = trimmed.trim_end_matches('{').trim_end();
    if clean.len() > 110 {
        let mut end = 107;
        while end > 0 && !clean.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &clean[..end])
    } else {
        clean.to_string()
    }
}

fn outline_brace_lang(lines: &[&str], lang: Language) -> Vec<String> {
    let nested = matches!(lang, Language::Swift | Language::Rust);
    let brace_info = scan_braces(lines, nested);
    let mut out = Vec::new();

    let mut depth: i32 = 0;
    let mut i = 0;
    let mut import_start: Option<usize> = None;
    let mut import_end: usize = 0;
    let mut import_count: usize = 0;
    let mut import_samples: Vec<String> = Vec::new();

    let flush_imports = |out: &mut Vec<String>,
                         import_start: &mut Option<usize>,
                         import_end: usize,
                         import_count: &mut usize,
                         import_samples: &mut Vec<String>| {
        if let Some(start) = import_start.take() {
            let range = if start == import_end {
                format!("L{}", start + 1)
            } else {
                format!("L{}-{}", start + 1, import_end + 1)
            };
            let preview = if import_samples.is_empty() {
                String::new()
            } else if *import_count > import_samples.len() {
                format!(
                    " ({}, +{} more)",
                    import_samples.join(", "),
                    *import_count - import_samples.len()
                )
            } else {
                format!(" ({})", import_samples.join(", "))
            };
            out.push(format!("{range:>11}| // {} imports{preview}", *import_count));
            *import_count = 0;
            import_samples.clear();
        }
    };

    while i < lines.len() {
        let trimmed = lines[i].trim();
        let (delta, _) = brace_info[i];

        // Handle Go multi-line `import ( ... )`
        if lang == Language::Go && trimmed == "import (" && depth == 0 {
            let start = i;
            let mut count = 0;
            i += 1;
            while i < lines.len() && lines[i].trim() != ")" {
                let t = lines[i].trim();
                if !t.is_empty() && !t.starts_with("//") {
                    count += 1;
                }
                i += 1;
            }
            out.push(format!(
                "{:>11}| // {count} imports",
                format!("L{}-{}", start + 1, (i + 1).min(lines.len()))
            ));
            i += 1;
            continue;
        }

        let kind = if depth <= 2 {
            classify_brace_line(trimmed, lang, depth)
        } else {
            DeclKind::None
        };

        if kind == DeclKind::Import && depth == 0 {
            if import_start.is_none() {
                import_start = Some(i);
            }
            import_end = i;
            import_count += 1;
            if import_samples.len() < 4 {
                let name = extract_import_name(trimmed);
                if !name.is_empty() {
                    import_samples.push(name);
                }
            }
            depth += delta;
            i += 1;
            continue;
        } else if !trimmed.is_empty() {
            flush_imports(
                &mut out,
                &mut import_start,
                import_end,
                &mut import_count,
                &mut import_samples,
            );
        }

        match kind {
            DeclKind::Marker => {
                let indent = "  ".repeat(depth.max(0) as usize);
                out.push(format!("{:>11}| {indent}{}", format!("L{}", i + 1), trimmed));
                depth += delta;
                i += 1;
            }
            DeclKind::Container => {
                let start_idx = find_brace_decorator_start(lines, i, lang);
                let end_idx = find_block_end(&brace_info, i, depth);
                let range = if end_idx > start_idx {
                    format!("L{}-{}", start_idx + 1, end_idx + 1)
                } else {
                    format!("L{}", start_idx + 1)
                };
                let indent = "  ".repeat(depth.max(0) as usize);
                out.push(format!("{range:>11}| {indent}{}", compact_signature(trimmed)));
                depth += delta;
                i += 1;
            }
            DeclKind::Callable => {
                let start_idx = find_brace_decorator_start(lines, i, lang);
                let end_idx = find_block_end(&brace_info, i, depth);
                let range = if end_idx > start_idx {
                    format!("L{}-{}", start_idx + 1, end_idx + 1)
                } else {
                    format!("L{}", start_idx + 1)
                };
                let indent = "  ".repeat(depth.max(0) as usize);
                out.push(format!("{range:>11}| {indent}{}", compact_signature(trimmed)));
                if end_idx > i {
                    // Skip the callable's body
                    for &(d, _) in &brace_info[i..=end_idx] {
                        depth += d;
                    }
                    i = end_idx + 1;
                } else {
                    depth += delta;
                    i += 1;
                }
            }
            DeclKind::Property => {
                let indent = "  ".repeat(depth.max(0) as usize);
                out.push(format!(
                    "{:>11}| {indent}{}",
                    format!("L{}", i + 1),
                    compact_signature(trimmed)
                ));
                depth += delta;
                i += 1;
            }
            DeclKind::Import | DeclKind::None => {
                depth += delta;
                i += 1;
            }
        }
    }

    flush_imports(
        &mut out,
        &mut import_start,
        import_end,
        &mut import_count,
        &mut import_samples,
    );

    out
}

/// Walk backward from a brace-language declaration to include any immediately
/// preceding standalone decorators/attributes (e.g. TS `@Component({...})`,
/// Swift `@MainActor`, Rust `#[derive(...)]`, Java/Kotlin `@Override`),
/// stopping if a line has code after the attribute (e.g. `@Input() a = 1;` or
/// `@Published var x = 1`).
fn find_brace_decorator_start(lines: &[&str], decl_idx: usize, lang: Language) -> usize {
    let decl_line = lines[decl_idx];
    let decl_indent = decl_line.len() - decl_line.trim_start().len();

    if matches!(lang, Language::Go) {
        let mut start = decl_idx;
        let mut k = decl_idx;
        while k > 0 {
            k -= 1;
            let line = lines[k];
            let trimmed = line.trim();
            let line_indent = line.len() - line.trim_start().len();
            if trimmed.starts_with("//") && line_indent == decl_indent {
                start = k;
            } else {
                break;
            }
        }
        return start;
    }

    let mut start = decl_idx;
    let mut k = decl_idx;
    let mut paren_depth: i32 = 0;

    while k > 0 {
        k -= 1;
        let line = lines[k];
        let trimmed = line.trim();
        let line_indent = line.len() - line.trim_start().len();

        if paren_depth > 0 {
            paren_depth += brace_reverse_paren_delta(trimmed);
            if paren_depth <= 0 {
                paren_depth = 0;
                if is_multiline_attr_open(trimmed, lang) && line_indent == decl_indent {
                    start = k;
                } else {
                    break;
                }
            }
            continue;
        }

        if trimmed.is_empty() {
            break;
        }
        if trimmed.starts_with("///") && line_indent == decl_indent {
            start = k;
            continue;
        }
        if trimmed.starts_with("//") {
            let body = trimmed.trim_start_matches('/').trim_start();
            if body.starts_with("MARK:") {
                break;
            }
            continue;
        }

        // Single-line or multi-line block doc comment `/** ... */` (not `/**/` or `/*!`)
        if trimmed.ends_with("*/") {
            if trimmed.starts_with("/**")
                && !trimmed.starts_with("/**/")
                && !trimmed.starts_with("/*!")
                && line_indent == decl_indent
            {
                start = k;
                continue;
            }
            if !trimmed.contains("/*") {
                // Walk backward to find the opening `/**`
                let mut m = k;
                let mut found_doc = None;
                while m > 0 {
                    m -= 1;
                    let mt = lines[m].trim();
                    if mt.contains("/*") {
                        let m_indent = lines[m].len() - lines[m].trim_start().len();
                        if mt.starts_with("/**")
                            && !mt.starts_with("/**/")
                            && !mt.starts_with("/*!")
                            && m_indent == decl_indent
                        {
                            found_doc = Some(m);
                        }
                        break;
                    }
                }
                if let Some(doc_start) = found_doc {
                    start = doc_start;
                    k = doc_start;
                    continue;
                }
            }
            break;
        }

        if is_standalone_attr_line(trimmed, lang) && line_indent == decl_indent {
            start = k;
            continue;
        }

        // Multi-line decorator/attribute closing line: `})` / `)` for `@...(...)` or `]` for `#[...]`
        let code_part = strip_trailing_line_comment(trimmed);
        let closes_attr = match lang {
            Language::Rust => code_part.ends_with(']'),
            _ => code_part.ends_with(')'),
        };
        if closes_attr && line_indent >= decl_indent {
            let delta = brace_reverse_paren_delta(code_part);
            if delta > 0 {
                paren_depth = delta;
                continue;
            }
        }

        break;
    }

    start
}

fn is_multiline_attr_open(trimmed: &str, lang: Language) -> bool {
    match lang {
        Language::Rust => trimmed.starts_with("#["),
        Language::TypeScript | Language::JavaScript | Language::Swift | Language::CLike => {
            trimmed.starts_with('@')
                && !trimmed.starts_with("@interface")
                && !trimmed.starts_with("@implementation")
                && !trimmed.starts_with("@protocol")
                && !trimmed.starts_with("@end")
        }
        _ => false,
    }
}

/// Returns true iff `trimmed` consists solely of one or more complete attributes/decorators
/// (and optional trailing `//` comment), with no trailing code such as `@Input() a = 1;`.
fn is_standalone_attr_line(trimmed: &str, lang: Language) -> bool {
    let code = strip_trailing_line_comment(trimmed);
    if code.is_empty() {
        return false;
    }
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut count = 0usize;

    match lang {
        Language::Rust => {
            while i < len {
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i >= len {
                    break;
                }
                if !(i + 1 < len && bytes[i] == b'#' && bytes[i + 1] == b'[') {
                    return false;
                }
                let Some(next) = skip_balanced_delim(bytes, i + 1, b'[', b']') else {
                    return false;
                };
                i = next;
                count += 1;
            }
            count > 0
        }
        Language::TypeScript | Language::JavaScript | Language::Swift | Language::CLike => {
            while i < len {
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i >= len {
                    break;
                }
                if bytes[i] != b'@' {
                    return false;
                }
                let rest = &code[i..];
                if rest.starts_with("@interface")
                    || rest.starts_with("@implementation")
                    || rest.starts_with("@protocol")
                    || rest.starts_with("@end")
                {
                    return false;
                }
                i += 1;
                let name_start = i;
                while i < len
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'.'
                        || bytes[i] == b'$')
                {
                    i += 1;
                }
                if i == name_start {
                    return false;
                }
                if i < len && bytes[i] == b'(' {
                    let Some(next) = skip_balanced_delim(bytes, i, b'(', b')') else {
                        return false;
                    };
                    i = next;
                }
                count += 1;
            }
            count > 0
        }
        _ => false,
    }
}

fn skip_balanced_delim(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let len = bytes.len();
    let mut i = start + 1;
    let mut depth = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;

    while i < len && depth > 0 {
        if (in_single || in_double || in_backtick) && bytes[i] == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if !in_double && !in_backtick && bytes[i] == b'\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if !in_single && !in_backtick && bytes[i] == b'"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double && bytes[i] == b'`' {
            in_backtick = !in_backtick;
            i += 1;
            continue;
        }
        if !in_single && !in_double && !in_backtick {
            if bytes[i] == open {
                depth += 1;
            } else if bytes[i] == close {
                depth -= 1;
            }
        }
        i += 1;
    }
    if depth == 0 { Some(i) } else { None }
}

fn strip_trailing_line_comment(trimmed: &str) -> &str {
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    while i < len {
        if (in_single || in_double || in_backtick) && bytes[i] == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if !in_double && !in_backtick && bytes[i] == b'\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if !in_single && !in_backtick && bytes[i] == b'"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double && bytes[i] == b'`' {
            in_backtick = !in_backtick;
            i += 1;
            continue;
        }
        if !in_single && !in_double && !in_backtick && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return trimmed[..i].trim_end();
        }
        i += 1;
    }
    trimmed
}

/// Net `(closing - opening)` bracket count on a brace-language line (ignoring `//` comments and strings).
fn brace_reverse_paren_delta(trimmed: &str) -> i32 {
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut delta = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;

    while i < len {
        if (in_single || in_double || in_backtick) && bytes[i] == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if !in_double && !in_backtick && bytes[i] == b'\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if !in_single && !in_backtick && bytes[i] == b'"' {
            in_double = !in_double;
            i += 1;
            continue;
        }
        if !in_single && !in_double && bytes[i] == b'`' {
            in_backtick = !in_backtick;
            i += 1;
            continue;
        }
        if in_single || in_double || in_backtick {
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            break;
        }
        match bytes[i] {
            b')' | b']' | b'}' => delta += 1,
            b'(' | b'[' | b'{' => delta -= 1,
            _ => {}
        }
        i += 1;
    }
    delta
}

fn extract_import_name(trimmed: &str) -> String {
    let s = trimmed
        .trim_start_matches("@testable ")
        .trim_start_matches("pub ")
        .trim_start_matches("import ")
        .trim_start_matches("use ")
        .trim_start_matches("#include ")
        .trim_start_matches("#import ")
        .trim_end_matches(';')
        .trim();
    // For JS/TS `import { a } from "pkg"`
    if let Some((_, from)) = s.rsplit_once(" from ") {
        return from.trim_matches(|c| c == '"' || c == '\'' || c == ';').to_string();
    }
    s.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>')
        .to_string()
}

fn outline_python(lines: &[&str]) -> Vec<String> {
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

fn outline_markdown(lines: &[&str]) -> Vec<String> {
    let mut headings: Vec<(usize, usize, &str)> = Vec::new();
    let mut in_fence = false;
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if t.starts_with('#') {
            let level = t.bytes().take_while(|&b| b == b'#').count();
            if level <= 4 && t.as_bytes().get(level) == Some(&b' ') {
                headings.push((i, level, t.trim_end()));
            }
        }
    }

    let mut out = Vec::with_capacity(headings.len());
    for (idx, &(line_idx, level, text)) in headings.iter().enumerate() {
        let end_idx = headings
            .iter()
            .skip(idx + 1)
            .find(|&&(_, l, _)| l <= level)
            .map(|&(next_i, _, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = format!("L{}-{}", line_idx + 1, end_idx + 1);
        out.push(format!("{range:>11}| {text}"));
    }
    out
}

fn outline_json(lines: &[&str]) -> Vec<String> {
    // Emit top-level keys (indentation 1-4 spaces) with approximate ranges
    let mut keys: Vec<(usize, String)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let indent = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim();
        if (indent == 2 || indent == 4 || indent == 1)
            && trimmed.starts_with('"')
            && let Some(colon) = trimmed.find("\":")
        {
            let key = &trimmed[..colon + 1];
            let val_hint = trimmed[colon + 2..].trim();
            let preview = if val_hint.len() > 40 {
                format!("{}…", &val_hint[..37])
            } else {
                val_hint.to_string()
            };
            keys.push((i, format!("{key}: {preview}")));
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    for (idx, (line_idx, label)) in keys.iter().enumerate() {
        let end_idx = keys
            .get(idx + 1)
            .map(|(next_i, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = if end_idx > *line_idx {
            format!("L{}-{}", line_idx + 1, end_idx + 1)
        } else {
            format!("L{}", line_idx + 1)
        };
        out.push(format!("{range:>11}| {label}"));
    }
    out
}

fn outline_yaml(lines: &[&str]) -> Vec<String> {
    let mut keys: Vec<(usize, String)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if raw.is_empty() || raw.starts_with(' ') || raw.starts_with('\t') || raw.starts_with('#') {
            continue;
        }
        if raw.starts_with("---") || raw.starts_with("...") {
            continue;
        }
        if let Some((k, v)) = raw.split_once(':') {
            let v = v.trim();
            let preview = if v.len() > 50 {
                format!("{}…", &v[..47])
            } else {
                v.to_string()
            };
            keys.push((i, format!("{k}: {preview}")));
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    for (idx, (line_idx, label)) in keys.iter().enumerate() {
        let end_idx = keys
            .get(idx + 1)
            .map(|(next_i, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = if end_idx > *line_idx {
            format!("L{}-{}", line_idx + 1, end_idx + 1)
        } else {
            format!("L{}", line_idx + 1)
        };
        out.push(format!("{range:>11}| {label}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_outline_ignores_braces_in_strings_and_comments() {
        let lines = [
            "use std::io;",
            "use std::fs;",
            "",
            "pub(crate) async fn tricky<'a>(x: &'a str) {",
            "    let s = \"}}} not closing\";",
            "    // } also not closing",
            "    let c = '}';",
            "    println!(\"{s} {c} {x}\");",
            "}",
            "",
            "pub struct Holder<T>(T);",
        ];
        let out = extract_outline(&lines, Language::Rust);
        let joined = out.join("\n");
        assert!(joined.contains("L1-2| // 2 imports"), "got: {joined}");
        assert!(joined.contains("L4-9| pub(crate) async fn tricky"), "got: {joined}");
        assert!(joined.contains("L11| pub struct Holder<T>(T);"), "got: {joined}");
    }

    #[test]
    fn swift_protocol_requirements_and_class_func() {
        let lines = [
            "protocol Worker {",
            "    func run() async throws",
            "    func stop()",
            "}",
            "final class Impl: Worker {",
            "    class func make() -> Impl {",
            "        Impl()",
            "    }",
            "    var body: some View {",
            "        Text(\"hi\")",
            "    }",
            "}",
        ];
        let out = extract_outline(&lines, Language::Swift);
        let joined = out.join("\n");
        assert!(joined.contains("L1-4| protocol Worker"), "got: {joined}");
        assert!(joined.contains("L2|   func run() async throws"), "got: {joined}");
        assert!(joined.contains("L3|   func stop()"), "got: {joined}");
        assert!(joined.contains("L5-12| final class Impl: Worker"), "got: {joined}");
        assert!(joined.contains("L6-8|   class func make() -> Impl"), "got: {joined}");
        assert!(joined.contains("L9-11|   var body: some View"), "got: {joined}");
    }

    #[test]
    fn ts_go_markdown_and_yaml_outlines() {
        let ts = [
            "import { a } from \"pkg-a\";",
            "export class Api {",
            "  async fetch(id: string): Promise<void> {",
            "    return;",
            "  }",
            "}",
            "export const helper = () => {",
            "  return 1;",
            "};",
        ];
        let ts_out = extract_outline(&ts, Language::TypeScript).join("\n");
        assert!(ts_out.contains("// 1 imports (pkg-a)"), "got: {ts_out}");
        assert!(ts_out.contains("L2-6| export class Api"), "got: {ts_out}");
        assert!(ts_out.contains("L3-5|   async fetch(id: string): Promise<void>"), "got: {ts_out}");
        assert!(ts_out.contains("L7-9| export const helper = () =>"), "got: {ts_out}");

        let go = [
            "package main",
            "import (",
            "    \"fmt\"",
            "    \"os\"",
            ")",
            "type Server struct {",
            "    port int",
            "}",
            "func (s *Server) Start() {",
            "    fmt.Println(os.Args)",
            "}",
        ];
        let go_out = extract_outline(&go, Language::Go).join("\n");
        assert!(go_out.contains("L2-5| // 2 imports"), "got: {go_out}");
        assert!(go_out.contains("L6-8| type Server struct"), "got: {go_out}");
        assert!(go_out.contains("L9-11| func (s *Server) Start()"), "got: {go_out}");

        let md = [
            "# Title",
            "```bash",
            "# not a heading",
            "```",
            "## Section",
            "body",
        ];
        let md_out = extract_outline(&md, Language::Markdown).join("\n");
        assert!(md_out.contains("L1-6| # Title"), "got: {md_out}");
        assert!(md_out.contains("L5-6| ## Section"), "got: {md_out}");
        assert!(!md_out.contains("not a heading"), "got: {md_out}");

        let yaml = [
            "---",
            "# comment",
            "name: demo",
            "jobs:",
            "  build:",
            "    runs-on: macos",
            "env: prod",
        ];
        let yaml_out = extract_outline(&yaml, Language::Yaml).join("\n");
        assert!(yaml_out.contains("L3| name: demo"), "got: {yaml_out}");
        assert!(yaml_out.contains("L4-6| jobs:"), "got: {yaml_out}");
        assert!(yaml_out.contains("L7| env: prod"), "got: {yaml_out}");
    }

    #[test]
    fn python_unrelated_multiline_call_not_attached_to_def() {
        let lines = [
            "register(",
            "    \"alpha\",",
            ")",
            "def handler():",
            "    return 1",
        ];
        let out = extract_outline(&lines, Language::Python).join("\n");
        assert!(out.contains("L4-5| def handler():"), "got: {out}");
    }

    #[test]
    fn symbol_lookup_prefers_exact_word_and_enclosing_block_picks_tightest() {
        let lines = [
            "struct Store {",
            "    fn run(&self) {",
            "        let x = 1;",
            "    }",
            "    fn run_all(&self) {",
            "        self.run();",
            "    }",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&lines, Language::Rust, "run"), vec![(2, 4)]);
        assert_eq!(enclosing_block(&lines, Language::Rust, 3, 60), Some((2, 4)));
        let sugg = suggest_symbols(&lines, Language::Rust, "runAll", 2);
        assert!(sugg.iter().any(|s| s.contains("run_all")), "got: {sugg:?}");
    }

    #[test]
    fn ts_and_rust_decorators_and_attributes_are_included() {
        let ts = [
            "@Component({",
            "  selector: 'app-foo',",
            "})",
            "export class Foo {",
            "  @Log()",
            "  run() {",
            "    return 1;",
            "  }",
            "}",
        ];
        let ts_out = extract_outline(&ts, Language::TypeScript).join("\n");
        assert!(ts_out.contains("L1-9| export class Foo"), "got: {ts_out}");
        assert!(ts_out.contains("L5-8|   run()"), "got: {ts_out}");
        assert_eq!(find_symbol_ranges(&ts, Language::TypeScript, "Foo"), vec![(1, 9)]);

        let rs = [
            "#[derive(",
            "    Debug,",
            "    Clone,",
            ")]",
            "pub struct Item {",
            "    id: u64,",
            "}",
        ];
        let rs_out = extract_outline(&rs, Language::Rust).join("\n");
        assert!(rs_out.contains("L1-7| pub struct Item"), "got: {rs_out}");
    }

    #[test]
    fn attribute_decorated_previous_member_is_not_pulled_into_next_symbol() {
        let ts = [
            "export class Button {",
            "  @Input() a = 1;",
            "  @HostListener(\"click\", [\"$event\"])",
            "  onClick(e: Event) {",
            "    return e;",
            "  }",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&ts, Language::TypeScript, "onClick"), vec![(3, 6)]);

        let swift = [
            "final class Model {",
            "    @Published var x = 1",
            "    @objc func tap() {",
            "        x += 1",
            "    }",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&swift, Language::Swift, "tap"), vec![(3, 5)]);

        let rs = [
            "#[allow(dead_code)] const X: i32 = 1;",
            "#[inline]",
            "fn work() -> i32 {",
            "    X",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&rs, Language::Rust, "work"), vec![(2, 5)]);
    }

    #[test]
    fn doc_comments_above_symbols_are_included() {
        let rs = [
            "/// Detached doc",
            "",
            "/// Doc for Point",
            "/// Second line",
            "#[derive(Debug)]",
            "pub struct Point {",
            "    x: f64,",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&rs, Language::Rust, "Point"), vec![(3, 8)]);

        let ts = [
            "/**",
            " * Formats a greeting.",
            " */",
            "export function greet(name: string) {",
            "  return name;",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&ts, Language::TypeScript, "greet"), vec![(1, 6)]);

        let go = [
            "// Detached comment",
            "",
            "// Sum adds.",
            "// Second line.",
            "func Sum(a, b int) int {",
            "    return a + b",
            "}",
        ];
        assert_eq!(find_symbol_ranges(&go, Language::Go, "Sum"), vec![(3, 7)]);
    }
}
