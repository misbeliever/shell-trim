use super::compact_signature;
use crate::lang::Language;

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

pub(super) fn outline_brace_lang(lines: &[&str], lang: Language) -> Vec<String> {
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
            out.push(format!(
                "{range:>11}| // {} imports{preview}",
                *import_count
            ));
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
                out.push(format!(
                    "{:>11}| {indent}{}",
                    format!("L{}", i + 1),
                    trimmed
                ));
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
                out.push(format!(
                    "{range:>11}| {indent}{}",
                    compact_signature(trimmed)
                ));
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
                out.push(format!(
                    "{range:>11}| {indent}{}",
                    compact_signature(trimmed)
                ));
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
        if !in_single
            && !in_double
            && !in_backtick
            && i + 1 < len
            && bytes[i] == b'/'
            && bytes[i + 1] == b'/'
        {
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
        return from
            .trim_matches(|c| c == '"' || c == '\'' || c == ';')
            .to_string();
    }
    s.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>')
        .to_string()
}
