mod brace;
mod data;
mod python;

use crate::lang::Language;

/// Produces a compact structural outline of a file with exact 1-based line ranges.
pub fn extract_outline(lines: &[&str], lang: Language) -> Vec<String> {
    match lang {
        Language::Python => python::outline_python(lines),
        Language::Markdown => data::outline_markdown(lines),
        Language::Json => data::outline_json(lines),
        Language::Yaml => data::outline_yaml(lines),
        Language::Swift
        | Language::TypeScript
        | Language::JavaScript
        | Language::Rust
        | Language::Go
        | Language::CLike => brace::outline_brace_lang(lines, lang),
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
        if decl_trimmed.starts_with("//")
            || decl_trimmed.starts_with('#') && lang != Language::Markdown
        {
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

    if !exact.is_empty() { exact } else { fuzzy }
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
        if decl_trimmed.starts_with("//")
            || (decl_trimmed.starts_with('#') && lang != Language::Markdown)
        {
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
            } else if d_words
                .iter()
                .any(|dw| dw.starts_with(qw) || qw.starts_with(dw))
            {
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
                let t_compact: String = tok
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
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
    scored
        .into_iter()
        .take(max_results)
        .map(|(_, s)| s)
        .collect()
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

pub(super) fn compact_signature(line: &str) -> String {
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
        assert!(
            joined.contains("L4-9| pub(crate) async fn tricky"),
            "got: {joined}"
        );
        assert!(
            joined.contains("L11| pub struct Holder<T>(T);"),
            "got: {joined}"
        );
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
        assert!(
            joined.contains("L2|   func run() async throws"),
            "got: {joined}"
        );
        assert!(joined.contains("L3|   func stop()"), "got: {joined}");
        assert!(
            joined.contains("L5-12| final class Impl: Worker"),
            "got: {joined}"
        );
        assert!(
            joined.contains("L6-8|   class func make() -> Impl"),
            "got: {joined}"
        );
        assert!(
            joined.contains("L9-11|   var body: some View"),
            "got: {joined}"
        );
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
        assert!(
            ts_out.contains("L3-5|   async fetch(id: string): Promise<void>"),
            "got: {ts_out}"
        );
        assert!(
            ts_out.contains("L7-9| export const helper = () =>"),
            "got: {ts_out}"
        );

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
        assert!(
            go_out.contains("L9-11| func (s *Server) Start()"),
            "got: {go_out}"
        );

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
        assert_eq!(
            find_symbol_ranges(&lines, Language::Rust, "run"),
            vec![(2, 4)]
        );
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
        assert_eq!(
            find_symbol_ranges(&ts, Language::TypeScript, "Foo"),
            vec![(1, 9)]
        );

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
        assert_eq!(
            find_symbol_ranges(&ts, Language::TypeScript, "onClick"),
            vec![(3, 6)]
        );

        let swift = [
            "final class Model {",
            "    @Published var x = 1",
            "    @objc func tap() {",
            "        x += 1",
            "    }",
            "}",
        ];
        assert_eq!(
            find_symbol_ranges(&swift, Language::Swift, "tap"),
            vec![(3, 5)]
        );

        let rs = [
            "#[allow(dead_code)] const X: i32 = 1;",
            "#[inline]",
            "fn work() -> i32 {",
            "    X",
            "}",
        ];
        assert_eq!(
            find_symbol_ranges(&rs, Language::Rust, "work"),
            vec![(2, 5)]
        );
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
        assert_eq!(
            find_symbol_ranges(&rs, Language::Rust, "Point"),
            vec![(3, 8)]
        );

        let ts = [
            "/**",
            " * Formats a greeting.",
            " */",
            "export function greet(name: string) {",
            "  return name;",
            "}",
        ];
        assert_eq!(
            find_symbol_ranges(&ts, Language::TypeScript, "greet"),
            vec![(1, 6)]
        );

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
