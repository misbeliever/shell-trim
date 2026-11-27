use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    Auto,
    Raw,
    Minimal,
    Outline,
    Aggressive,
}

impl ReadMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadMode::Auto => "auto",
            ReadMode::Raw => "raw",
            ReadMode::Minimal => "minimal",
            ReadMode::Outline => "outline",
            ReadMode::Aggressive => "aggressive",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(ReadMode::Auto),
            "raw" | "none" => Ok(ReadMode::Raw),
            "minimal" | "min" => Ok(ReadMode::Minimal),
            "outline" | "symbols" | "map" => Ok(ReadMode::Outline),
            "aggressive" | "agg" => Ok(ReadMode::Aggressive),
            other => Err(format!(
                "invalid mode '{other}' (expected: auto, raw, minimal, outline, aggressive)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: usize,       // 1-based inclusive
    pub end: Option<usize>, // 1-based inclusive; None means EOF
}

impl LineRange {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty line range".into());
        }
        let sep = if s.contains('-') {
            '-'
        } else if s.contains(':') {
            ':'
        } else if s.contains(',') {
            ','
        } else {
            let single: usize = s
                .parse()
                .map_err(|_| format!("invalid line number '{s}'"))?;
            if single == 0 {
                return Err("line numbers are 1-based".into());
            }
            return Ok(LineRange {
                start: single,
                end: Some(single),
            });
        };

        let (left, right) = s.split_once(sep).unwrap();
        let start = if left.is_empty() {
            1
        } else {
            let v: usize = left
                .parse()
                .map_err(|_| format!("invalid range start '{left}'"))?;
            if v == 0 {
                return Err("line numbers are 1-based".into());
            }
            v
        };
        let end = if right.is_empty() {
            None
        } else {
            let v: usize = right
                .parse()
                .map_err(|_| format!("invalid range end '{right}'"))?;
            if v < start {
                return Err(format!("range end ({v}) < start ({start})"));
            }
            Some(v)
        };
        Ok(LineRange { start, end })
    }

    pub fn format(&self) -> String {
        match self.end {
            Some(e) => format!("{}-{e}", self.start),
            None => format!("{}-", self.start),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub paths: Vec<String>,
    pub mode: ReadMode,
    pub range: Option<LineRange>,
    pub symbol: Option<String>,
    pub head: Option<usize>,
    pub tail: Option<usize>,
    pub max_lines: Option<usize>,
    pub max_chars: Option<usize>,
    pub line_numbers: bool,
    pub lang_override: Option<String>,
    pub log_stats: bool,
}

#[derive(Debug, Clone)]
pub struct StatsOptions {
    pub since_secs: Option<u64>,
    pub by_lang: bool,
    pub by_agent: bool,
    pub by_model: bool,
    pub agent_filter: Option<String>,
    pub model_filter: Option<String>,
    pub misses: bool,
    pub recommend: bool,
    pub json: bool,
    pub reset: bool,
    pub reread_window_secs: u64,
}

#[derive(Debug, Clone)]
pub struct AuditOptions {
    pub since_secs: Option<u64>,
    pub agent_filter: Option<String>,
    pub by_model: bool,
    pub bypasses: bool,
    pub json: bool,
    pub context: bool,
    pub probe_mcp: bool,
    pub all: bool,
}

#[derive(Debug, Clone)]
pub struct PipeOptions {
    pub raw: bool,
    pub all: bool,
    pub max_lines: Option<usize>,
    pub log_stats: bool,
}

impl Default for PipeOptions {
    fn default() -> Self {
        Self {
            raw: false,
            all: false,
            max_lines: None,
            log_stats: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub rg_args: Vec<String>,
    pub all: bool,
    pub raw: bool,
    pub block: Option<bool>, // Some(true)=force --block, Some(false)=--no-block, None=auto (1..=3 matches)
    pub max_total: Option<usize>,
    pub per_file: Option<usize>,
    pub max_chars: Option<usize>,
    pub log_stats: bool,
}

#[derive(Debug, Clone)]
pub struct ExecTrimOptions {
    pub args: Vec<String>,
    pub raw: bool,
    pub all: bool,
    pub log_stats: bool,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub dry_run: bool,
    pub only: Option<String>,
    pub hook: bool,
}

#[derive(Debug, Clone)]
pub struct UiOptions {
    pub since_label: Option<String>,
    pub once: bool,
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub since_secs: Option<u64>,
    pub out_path: Option<String>,
    pub redact: bool,
    pub anon: bool,
}

#[derive(Debug, Clone)]
pub struct InspectOptions {
    pub path: String,
}

#[derive(Debug)]
pub enum Command {
    Help,
    Version,
    Read(ReadOptions),
    Search(SearchOptions),
    Swift(&'static str, ExecTrimOptions),
    Cargo(ExecTrimOptions),
    Git(ExecTrimOptions),
    Ls(ExecTrimOptions),
    Find(ExecTrimOptions),
    Pipe(PipeOptions),
    Hook(crate::hook::HookTarget),
    Ui(UiOptions),
    Doctor,
    Stats(StatsOptions),
    Audit(AuditOptions),
    Export(ExportOptions),
    Inspect(InspectOptions),
    Install(InstallOptions),
    Uninstall(InstallOptions),
}

pub const HELP_TEXT: &str = "\
trm — fast, language-aware shell output trimmer for AI coding agents

USAGE:
  trm read <file|-> [files...] [options]   (aliases: trm cat|head|tail|sed -n)
  trm rg [options] <rg-args...>            (alias: trm grep)
  trm swift [options] <args...>            trim swift build/test output (also: trm swiftc)
  trm cargo [options] <args...>            trim cargo build/check/clippy/test output
  trm git [options] <args...>              compact git status/diff/log/show output
  trm ls [options] [args...]               compact directory listing (strips ls -l noise)
  trm find [options] [args...]             directory-grouped find (hides .git/target/node_modules)
  trm pipe [options]                       trim piped stdin (or run bare `cmd | trm`)
  trm hook <agy|claude>                    PreToolUse hook handler (rewrites raw commands via stdin JSON)
  trm ui [-s|--since 1h|24h|7d|all] [--once]  interactive terminal dashboard (alias: trm top)
  trm doctor                               check PATH, agent rules, config & telemetry health
  trm stats [options]                      (alias: trm gain)
  trm audit [options]                      scan Codex/Antigravity/Claude/Copilot/Pi logs for adoption %
  trm export [options]                     export shareable JSON report with full misses & bypasses (alias: trm report)
  trm inspect <report.json|->              render a compact summary from a 'trm export' JSON report
  trm install [options]                    install always-on agent rules
  trm uninstall [options]                  cleanly remove trm rules

READ OPTIONS:
  -m, --mode <mode>      auto (default) | raw | minimal | outline | aggressive
      --raw              shorthand for --mode raw
      --minimal          shorthand for --mode minimal
      --outline          shorthand for --mode outline
      --aggressive       shorthand for --mode aggressive
  -r, --range <A-B>      1-based line range (e.g. 40-120, 40:120, 100-, -50)
  -s, --symbol <name>    extract exact raw declaration/body for a symbol name
  -H, --head <N>         read first N lines (raw)
  -T, --tail <N>         read last N lines (raw)
  -n, --max-lines <N>    cap output lines
  -c, --max-chars <N>    truncate output lines longer than N chars
  -L, --line-numbers     prefix every output line with its 1-based line number
  -l, --lang <ext>       override language detection (useful for stdin '-')
      --no-stats         do not record telemetry for this invocation

SEARCH OPTIONS (trm rg / trm grep):
      --all              disable match caps (show all matches, grouped)
      --block            expand matches to enclosing function/context block (auto for 1-3 hits)
      --no-block         disable auto block expansion on 1-3 hits
      --max <N>          cap total shown matches (default: 50)
      --per-file <N>     cap matches per file (default: 6)
      --max-chars <N>    truncate matched lines longer than N chars (default: 180)
      --raw              pass through raw rg output unmodified
      --no-stats         do not record telemetry
  (all other flags and paths are passed straight to rg)

STATS OPTIONS:
  -s, --since <dur>      filter window: 1h, 24h, 7d, 30d, all (default: all)
  -a, --by-agent         show breakdown by agent (gross/net, misses, drill-downs)
  -m, --by-model         show breakdown by provider/model
  -l, --by-lang          show breakdown by language
      --agent <name>     filter stats to one agent (e.g. codex, agy, claude)
      --model <substr>   filter stats by provider/model substring
      --misses           list detected trim misses (re-reads)
      --recommend        analyze logged data and suggest threshold/config tweaks
      --window <secs>    re-read detection window in seconds (default: 180)
  -j, --json             output stats as JSON
      --reset            clear recorded telemetry

AUDIT OPTIONS:
  -s, --since <dur>      scan logs modified within window (default: 7d)
      --agent <name>     filter to one agent (codex, agy, claude, copilot, pi, cursor)
  -m, --by-model         show adoption breakdown by agent/model
  -b, --bypasses         list recent bypassed cat/head/tail/sed/rg/grep commands
  -j, --json             output audit as JSON
      --context          audit turn-0 token bloat across rules, skills & MCP configs
      --probe-mcp        probe stdio MCP servers for exact tool schema tokens
      --all              show all rules, skills & tools in --context output

EXPORT OPTIONS (trm export / trm report):
  -s, --since <dur>      filter window for misses & bypasses (default: 7d)
  -o, --out <file>       write JSON report to file instead of stdout
      --anon             deterministically hash repo/file paths while keeping extensions
      --no-redact        keep raw $HOME paths and command values unredacted

INSTALL / UNINSTALL OPTIONS:
      --only <target>    limit to: codex | agy | claude | copilot | pi | cursor
      --hook             also install Claude Code PreToolUse rewrite helper script
      --dry-run          preview changes without modifying files

AUTO MODE HEURISTICS:
  - Explicit --range       -> raw (safe for editing)
  - Small files (<=80 L)   -> raw
  - Medium files (81-260)  -> minimal (strips non-doc comments & blank runs)
  - Large files (>260 L)   -> outline (declarations + [Lstart-Lend] ranges)
";

pub fn parse(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        if !std::io::stdin().is_terminal() {
            return Ok(Command::Pipe(PipeOptions::default()));
        }
        return Ok(Command::Help);
    }
    match args[0].as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "read" => parse_read(&args[1..]),
        "cat" => parse_cat_alias(&args[1..]),
        "head" => parse_head_tail_alias(&args[1..], false),
        "tail" => parse_head_tail_alias(&args[1..], true),
        "sed" => parse_sed_alias(&args[1..]),
        "rg" | "grep" => parse_search(&args[1..]),
        "swift" => parse_exec_trim(&args[1..], AllFlagMode::LeadingOnly, |o| {
            Command::Swift("swift", o)
        }),
        "swiftc" => parse_exec_trim(&args[1..], AllFlagMode::LeadingOnly, |o| {
            Command::Swift("swiftc", o)
        }),
        "cargo" => parse_exec_trim(&args[1..], AllFlagMode::LeadingOnly, Command::Cargo),
        "git" => parse_exec_trim(&args[1..], AllFlagMode::GitDiff, Command::Git),
        "ls" => parse_exec_trim_allow_empty(&args[1..], AllFlagMode::Anywhere, Command::Ls),
        "find" => parse_exec_trim_allow_empty(&args[1..], AllFlagMode::Anywhere, Command::Find),
        "pipe" => parse_pipe(&args[1..]),
        "hook" => parse_hook(&args[1..]),
        "ui" | "top" => parse_ui(&args[1..]),
        "doctor" => Ok(Command::Doctor),
        "stats" | "gain" => parse_stats(&args[1..]),
        "audit" => parse_audit(&args[1..]),
        "export" | "report" => parse_export(&args[1..]),
        "inspect" => parse_inspect(&args[1..]),
        "install" => parse_install(&args[1..], false),
        "uninstall" => parse_install(&args[1..], true),
        other => Err(format!("unknown subcommand '{other}'")),
    }
}

fn parse_read(args: &[String]) -> Result<Command, String> {
    let mut paths: Vec<String> = Vec::new();
    let mut mode = ReadMode::Auto;
    let mut range: Option<LineRange> = None;
    let mut symbol: Option<String> = None;
    let mut head: Option<usize> = None;
    let mut tail: Option<usize> = None;
    let mut max_lines: Option<usize> = None;
    let mut max_chars: Option<usize> = None;
    let mut line_numbers = false;
    let mut lang_override: Option<String> = None;
    let mut log_stats = true;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "-m" | "--mode" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --mode")?;
                mode = ReadMode::parse(v)?;
            }
            "--raw" => mode = ReadMode::Raw,
            "--minimal" => mode = ReadMode::Minimal,
            "--outline" | "--symbols" => mode = ReadMode::Outline,
            "--aggressive" => mode = ReadMode::Aggressive,
            "-r" | "--range" | "--lines" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --range")?;
                range = Some(LineRange::parse(v)?);
            }
            "-s" | "--symbol" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --symbol")?;
                if v.trim().is_empty() {
                    return Err("--symbol cannot be empty".into());
                }
                symbol = Some(v.clone());
            }
            "-H" | "--head" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --head")?;
                let n: usize = v.parse().map_err(|_| format!("invalid --head '{v}'"))?;
                if n == 0 {
                    return Err("--head must be >= 1".into());
                }
                head = Some(n);
            }
            "-T" | "--tail" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --tail")?;
                let n: usize = v.parse().map_err(|_| format!("invalid --tail '{v}'"))?;
                if n == 0 {
                    return Err("--tail must be >= 1".into());
                }
                tail = Some(n);
            }
            "-n" | "--max-lines" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --max-lines")?;
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("invalid --max-lines '{v}'"))?;
                max_lines = Some(n);
            }
            "-c" | "--max-chars" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --max-chars")?;
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("invalid --max-chars '{v}'"))?;
                if n < 10 {
                    return Err("--max-chars must be >= 10".into());
                }
                max_chars = Some(n);
            }
            "-L" | "--line-numbers" => line_numbers = true,
            "-N" | "--no-line-numbers" => line_numbers = false,
            "-l" | "--lang" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --lang")?;
                lang_override = Some(v.clone());
            }
            "--no-stats" => log_stats = false,
            "-" => paths.push("-".into()),
            s if s.starts_with("--mode=") => {
                mode = ReadMode::parse(&s["--mode=".len()..])?;
            }
            s if s.starts_with("--range=") => {
                range = Some(LineRange::parse(&s["--range=".len()..])?);
            }
            s if s.starts_with("--symbol=") => {
                let v = &s["--symbol=".len()..];
                if v.trim().is_empty() {
                    return Err("--symbol cannot be empty".into());
                }
                symbol = Some(v.to_string());
            }
            s if s.starts_with("--head=") => {
                let v = &s["--head=".len()..];
                let n: usize = v.parse().map_err(|_| format!("invalid --head '{v}'"))?;
                if n == 0 {
                    return Err("--head must be >= 1".into());
                }
                head = Some(n);
            }
            s if s.starts_with("--tail=") => {
                let v = &s["--tail=".len()..];
                let n: usize = v.parse().map_err(|_| format!("invalid --tail '{v}'"))?;
                if n == 0 {
                    return Err("--tail must be >= 1".into());
                }
                tail = Some(n);
            }
            s if s.starts_with("--max-lines=") => {
                let v = &s["--max-lines=".len()..];
                max_lines = Some(
                    v.parse()
                        .map_err(|_| format!("invalid --max-lines '{v}'"))?,
                );
            }
            s if s.starts_with("--max-chars=") => {
                let v = &s["--max-chars=".len()..];
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("invalid --max-chars '{v}'"))?;
                if n < 10 {
                    return Err("--max-chars must be >= 10".into());
                }
                max_chars = Some(n);
            }
            s if s.starts_with("--lang=") => {
                lang_override = Some(s["--lang=".len()..].to_string());
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'"));
            }
            s => paths.push(s.to_string()),
        }
        i += 1;
    }

    let selectors = usize::from(range.is_some())
        + usize::from(symbol.is_some())
        + usize::from(head.is_some())
        + usize::from(tail.is_some());
    if selectors > 1 {
        return Err("cannot combine --range, --symbol, --head, and --tail".into());
    }

    if paths.is_empty() {
        return Err("missing <file> argument".into());
    }
    if paths.len() > 1 && range.is_some() {
        return Err("--range can only be used with a single file".into());
    }

    Ok(Command::Read(ReadOptions {
        paths,
        mode,
        range,
        symbol,
        head,
        tail,
        max_lines,
        max_chars,
        line_numbers,
        lang_override,
        log_stats,
    }))
}

fn parse_stats(args: &[String]) -> Result<Command, String> {
    let mut since_secs: Option<u64> = None;
    let mut by_lang = false;
    let mut by_agent = false;
    let mut by_model = false;
    let mut agent_filter: Option<String> = None;
    let mut model_filter: Option<String> = None;
    let mut misses = false;
    let mut recommend = false;
    let mut json = false;
    let mut reset = false;
    let mut reread_window_secs = crate::config::Config::load().reread_window_secs;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "-s" | "--since" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -s/--since")?;
                since_secs = parse_duration(v)?;
            }
            s if s.starts_with("--since=") => {
                since_secs = parse_duration(&s["--since=".len()..])?;
            }
            "-l" | "--by-lang" => by_lang = true,
            "-a" | "--by-agent" => by_agent = true,
            "-m" | "--by-model" => by_model = true,
            "--agent" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --agent")?;
                agent_filter = Some(normalize_agent_name(v));
            }
            s if s.starts_with("--agent=") => {
                agent_filter = Some(normalize_agent_name(&s["--agent=".len()..]));
            }
            "--model" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --model")?;
                model_filter = Some(v.to_lowercase());
            }
            s if s.starts_with("--model=") => {
                model_filter = Some(s["--model=".len()..].to_lowercase());
            }
            "--misses" => misses = true,
            "--recommend" | "--hints" => recommend = true,
            "-j" | "--json" => json = true,
            "--reset" => reset = true,
            "--window" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --window")?;
                reread_window_secs = v.parse().map_err(|_| format!("invalid --window '{v}'"))?;
            }
            other => return Err(format!("unknown stats option '{other}'")),
        }
        i += 1;
    }

    Ok(Command::Stats(StatsOptions {
        since_secs,
        by_lang,
        by_agent,
        by_model,
        agent_filter,
        model_filter,
        misses,
        recommend,
        json,
        reset,
        reread_window_secs,
    }))
}

fn parse_audit(args: &[String]) -> Result<Command, String> {
    let mut since_secs: Option<u64> = Some(7 * 86_400);
    let mut agent_filter: Option<String> = None;
    let mut by_model = false;
    let mut bypasses = false;
    let mut json = false;
    let mut context = false;
    let mut probe_mcp = false;
    let mut all = false;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "-s" | "--since" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -s/--since")?;
                since_secs = parse_duration(v)?;
            }
            s if s.starts_with("--since=") => {
                since_secs = parse_duration(&s["--since=".len()..])?;
            }
            "--agent" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --agent")?;
                agent_filter = Some(normalize_agent_name(v));
            }
            s if s.starts_with("--agent=") => {
                agent_filter = Some(normalize_agent_name(&s["--agent=".len()..]));
            }
            "-m" | "--by-model" => by_model = true,
            "-b" | "--bypasses" => bypasses = true,
            "-j" | "--json" => json = true,
            "--context" => context = true,
            "--all" => all = true,
            "--probe-mcp" => {
                context = true;
                probe_mcp = true;
            }
            other => return Err(format!("unknown audit option '{other}'")),
        }
        i += 1;
    }

    Ok(Command::Audit(AuditOptions {
        since_secs,
        agent_filter,
        by_model,
        bypasses,
        json,
        context,
        probe_mcp,
        all,
    }))
}

fn parse_export(args: &[String]) -> Result<Command, String> {
    let mut since_secs: Option<u64> = Some(7 * 86_400);
    let mut out_path: Option<String> = None;
    let mut redact = true;
    let mut anon = false;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "-s" | "--since" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -s/--since")?;
                since_secs = parse_duration(v)?;
            }
            s if s.starts_with("--since=") => {
                since_secs = parse_duration(&s["--since=".len()..])?;
            }
            "-o" | "--out" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -o/--out")?;
                out_path = Some(v.clone());
            }
            s if s.starts_with("--out=") => {
                out_path = Some(s["--out=".len()..].to_string());
            }
            "--no-redact" => redact = false,
            "--anon" | "--hash-paths" => anon = true,
            other => return Err(format!("unknown export option '{other}'")),
        }
        i += 1;
    }

    Ok(Command::Export(ExportOptions {
        since_secs,
        out_path,
        redact,
        anon,
    }))
}

fn parse_pipe(args: &[String]) -> Result<Command, String> {
    let mut opts = PipeOptions::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "--raw" => opts.raw = true,
            "--all" => opts.all = true,
            "--no-stats" => opts.log_stats = false,
            "-n" | "--max-lines" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -n/--max-lines")?;
                opts.max_lines = Some(
                    v.parse()
                        .map_err(|_| format!("invalid --max-lines '{v}'"))?,
                );
            }
            s if s.starts_with("--max-lines=") => {
                let v = &s["--max-lines=".len()..];
                opts.max_lines = Some(
                    v.parse()
                        .map_err(|_| format!("invalid --max-lines '{v}'"))?,
                );
            }
            other => return Err(format!("unknown pipe option '{other}'")),
        }
        i += 1;
    }
    Ok(Command::Pipe(opts))
}

fn parse_hook(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Err("missing hook target (expected: agy or claude)".into());
    }
    match args[0].as_str() {
        "-h" | "--help" => Ok(Command::Help),
        "agy" | "antigravity" => Ok(Command::Hook(crate::hook::HookTarget::Agy)),
        "claude" => Ok(Command::Hook(crate::hook::HookTarget::Claude)),
        other => Err(format!(
            "unknown hook target '{other}' (expected: agy or claude)"
        )),
    }
}

fn parse_inspect(args: &[String]) -> Result<Command, String> {
    let mut path: Option<String> = None;
    for a in args {
        match a.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown inspect option '{other}'"));
            }
            p => {
                if path.is_some() {
                    return Err(format!("unexpected extra argument '{p}'"));
                }
                path = Some(p.to_string());
            }
        }
    }
    let path = path.ok_or("missing report path (usage: trm inspect <report.json|->)")?;
    Ok(Command::Inspect(InspectOptions { path }))
}

fn parse_install(args: &[String], uninstall: bool) -> Result<Command, String> {
    let mut dry_run = false;
    let mut only: Option<String> = None;
    let mut hook = false;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(Command::Help),
            "--dry-run" => dry_run = true,
            "--hook" => hook = true,
            "--only" => {
                i += 1;
                let v = normalize_agent_name(args.get(i).ok_or("missing value for --only")?);
                validate_install_target(&v)?;
                only = Some(v);
            }
            s if s.starts_with("--only=") => {
                let v = normalize_agent_name(&s["--only=".len()..]);
                validate_install_target(&v)?;
                only = Some(v);
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }

    let opts = InstallOptions {
        dry_run,
        only,
        hook,
    };
    Ok(if uninstall {
        Command::Uninstall(opts)
    } else {
        Command::Install(opts)
    })
}

fn normalize_agent_name(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "antigravity" => "agy".to_string(),
        other => other.to_string(),
    }
}

fn validate_install_target(s: &str) -> Result<(), String> {
    match s {
        "codex" | "agy" | "claude" | "copilot" | "pi" | "cursor" => Ok(()),
        other => Err(format!(
            "invalid --only target '{other}' (expected: codex, agy, claude, copilot, pi, cursor)"
        )),
    }
}

fn parse_search(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Err("missing search pattern (e.g. 'trm rg pattern [path]')".into());
    }
    let mut rg_args = Vec::with_capacity(args.len());
    let mut all = false;
    let mut raw = false;
    let mut block: Option<bool> = None;
    let mut max_total = None;
    let mut per_file = None;
    let mut max_chars = None;
    let mut log_stats = true;

    let mut i = 0;
    let mut passthrough_rest = false;
    while i < args.len() {
        let a = args[i].as_str();
        if passthrough_rest {
            rg_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match a {
            "--" => {
                passthrough_rest = true;
                rg_args.push(args[i].clone());
            }
            "-h" | "--help" if rg_args.is_empty() => return Ok(Command::Help),
            "--all" => all = true,
            "--raw" => raw = true,
            "--block" => block = Some(true),
            "--no-block" => block = Some(false),
            "--no-stats" => log_stats = false,
            "--max" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --max")?;
                max_total = Some(v.parse().map_err(|_| format!("invalid --max '{v}'"))?);
            }
            "--per-file" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --per-file")?;
                per_file = Some(v.parse().map_err(|_| format!("invalid --per-file '{v}'"))?);
            }
            "--max-chars" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for --max-chars")?;
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("invalid --max-chars '{v}'"))?;
                if n < 20 {
                    return Err("--max-chars must be >= 20".into());
                }
                max_chars = Some(n);
            }
            s if s.starts_with("--max=") => {
                let v = &s["--max=".len()..];
                max_total = Some(v.parse().map_err(|_| format!("invalid --max '{v}'"))?);
            }
            s if s.starts_with("--per-file=") => {
                let v = &s["--per-file=".len()..];
                per_file = Some(v.parse().map_err(|_| format!("invalid --per-file '{v}'"))?);
            }
            s if s.starts_with("--max-chars=") => {
                let v = &s["--max-chars=".len()..];
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("invalid --max-chars '{v}'"))?;
                if n < 20 {
                    return Err("--max-chars must be >= 20".into());
                }
                max_chars = Some(n);
            }
            _ => rg_args.push(args[i].clone()),
        }
        i += 1;
    }

    if rg_args.is_empty() {
        return Err("missing search pattern".into());
    }

    Ok(Command::Search(SearchOptions {
        rg_args,
        all,
        raw,
        block,
        max_total,
        per_file,
        max_chars,
        log_stats,
    }))
}

#[derive(Clone, Copy)]
enum AllFlagMode {
    LeadingOnly,
    GitDiff,
    Anywhere,
}

fn parse_exec_trim<F>(args: &[String], all_mode: AllFlagMode, wrap: F) -> Result<Command, String>
where
    F: FnOnce(ExecTrimOptions) -> Command,
{
    if args.is_empty() {
        return Ok(Command::Help);
    }
    parse_exec_trim_allow_empty(args, all_mode, wrap)
}

fn parse_exec_trim_allow_empty<F>(
    args: &[String],
    all_mode: AllFlagMode,
    wrap: F,
) -> Result<Command, String>
where
    F: FnOnce(ExecTrimOptions) -> Command,
{
    let mut forwarded = Vec::with_capacity(args.len());
    let mut raw = false;
    let mut all = false;
    let mut log_stats = true;
    let mut after_double_dash = false;

    for a in args {
        if after_double_dash {
            forwarded.push(a.clone());
            continue;
        }
        if a == "--" {
            after_double_dash = true;
            forwarded.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--raw" => raw = true,
            "--all" => {
                let consume = match all_mode {
                    AllFlagMode::LeadingOnly => forwarded.is_empty(),
                    AllFlagMode::GitDiff => {
                        forwarded.is_empty() || git_subcommand(&forwarded) == Some("diff")
                    }
                    AllFlagMode::Anywhere => true,
                };
                if consume {
                    all = true;
                } else {
                    forwarded.push(a.clone());
                }
            }
            "--no-stats" => log_stats = false,
            _ => forwarded.push(a.clone()),
        }
    }

    Ok(wrap(ExecTrimOptions {
        args: forwarded,
        raw,
        all,
        log_stats,
    }))
}

fn git_subcommand(forwarded: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < forwarded.len() {
        let a = forwarded[i].as_str();
        if a == "-C" || a == "-c" || a == "--git-dir" || a == "--work-tree" {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(a);
    }
    None
}

fn parse_ui(args: &[String]) -> Result<Command, String> {
    let mut since_label = None;
    let mut once = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--once" => once = true,
            "-s" | "--since" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -s/--since")?;
                since_label = Some(v.to_lowercase());
            }
            s if s.starts_with("--since=") => {
                since_label = Some(s["--since=".len()..].to_lowercase());
            }
            other => return Err(format!("unknown ui option '{other}'")),
        }
        i += 1;
    }
    Ok(Command::Ui(UiOptions { since_label, once }))
}

fn parse_cat_alias(args: &[String]) -> Result<Command, String> {
    let mut paths = Vec::new();
    let mut line_numbers = false;
    let mut log_stats = true;
    for a in args {
        match a.as_str() {
            "-n" | "-b" => line_numbers = true,
            "--no-stats" => log_stats = false,
            s if s.starts_with('-') && s != "-" => {} // ignore cosmetic cat flags
            s => paths.push(s.to_string()),
        }
    }
    if paths.is_empty() {
        if !std::io::stdin().is_terminal() {
            paths.push("-".to_string());
        } else {
            return Err("trm cat: missing <file>".into());
        }
    }
    Ok(Command::Read(ReadOptions {
        paths,
        mode: ReadMode::Auto,
        range: None,
        symbol: None,
        head: None,
        tail: None,
        max_lines: None,
        max_chars: None,
        line_numbers,
        lang_override: None,
        log_stats,
    }))
}

fn parse_head_tail_alias(args: &[String], is_tail: bool) -> Result<Command, String> {
    let mut paths = Vec::new();
    let mut count: usize = 10;
    let mut log_stats = true;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-n" | "--lines" => {
                i += 1;
                let v = args.get(i).ok_or("missing value for -n")?;
                count = v
                    .trim_start_matches('+')
                    .parse()
                    .map_err(|_| format!("invalid line count '{v}'"))?;
            }
            "--no-stats" => log_stats = false,
            s if s.starts_with("-n") && s.len() > 2 => {
                count = s[2..]
                    .trim_start_matches('+')
                    .parse()
                    .map_err(|_| format!("invalid line count '{s}'"))?;
            }
            s if s.starts_with('-')
                && s.len() > 1
                && s[1..].bytes().all(|b| b.is_ascii_digit()) =>
            {
                count = s[1..]
                    .parse()
                    .map_err(|_| format!("invalid line count '{s}'"))?;
            }
            s if s.starts_with('-') && s != "-" => {}
            s => paths.push(s.to_string()),
        }
        i += 1;
    }
    if paths.is_empty() {
        if !std::io::stdin().is_terminal() {
            paths.push("-".to_string());
        } else {
            return Err(format!(
                "trm {}: missing <file>",
                if is_tail { "tail" } else { "head" }
            ));
        }
    }
    let count = count.max(1);
    Ok(Command::Read(ReadOptions {
        paths,
        mode: ReadMode::Auto,
        range: None,
        symbol: None,
        head: (!is_tail).then_some(count),
        tail: is_tail.then_some(count),
        max_lines: None,
        max_chars: None,
        line_numbers: false,
        lang_override: None,
        log_stats,
    }))
}

fn parse_sed_alias(args: &[String]) -> Result<Command, String> {
    // Supports `trm sed -n '880,960p' file`, `trm sed -n '42p' file`, `trm sed -n '100,$p' file`
    let mut expr: Option<String> = None;
    let mut paths = Vec::new();
    let mut log_stats = true;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-n" | "--quiet" | "--silent" => {}
            "-e" => {
                i += 1;
                let v = args.get(i).ok_or("missing expression for -e")?;
                expr = Some(v.clone());
            }
            "--no-stats" => log_stats = false,
            s if s.starts_with('-') && s != "-" => {}
            s => {
                if expr.is_none() {
                    expr = Some(s.to_string());
                } else {
                    paths.push(s.to_string());
                }
            }
        }
        i += 1;
    }

    let expr = expr.ok_or("trm sed: missing range expression (e.g. '1,80p')")?;
    if paths.is_empty() {
        return Err("trm sed: missing <file>".into());
    }

    // Parse sed print expression like `880,960p`, `42p`, `100,$p`, `1,50p;51q`
    let first_part = expr.split(';').next().unwrap_or(&expr).trim();
    let clean = first_part.trim_end_matches('p').trim();
    let range = if let Some((a, b)) = clean.split_once(',') {
        let start: usize = a
            .trim()
            .parse()
            .map_err(|_| format!("trm sed: unsupported expression '{expr}'"))?;
        let end = if b.trim() == "$" {
            None
        } else {
            Some(
                b.trim()
                    .parse::<usize>()
                    .map_err(|_| format!("trm sed: unsupported expression '{expr}'"))?,
            )
        };
        if start == 0 {
            return Err("line numbers are 1-based".into());
        }
        LineRange { start, end }
    } else {
        let n: usize = clean
            .parse()
            .map_err(|_| format!("trm sed: unsupported expression '{expr}'"))?;
        if n == 0 {
            return Err("line numbers are 1-based".into());
        }
        LineRange {
            start: n,
            end: Some(n),
        }
    };

    Ok(Command::Read(ReadOptions {
        paths,
        mode: ReadMode::Auto,
        range: Some(range),
        symbol: None,
        head: None,
        tail: None,
        max_lines: None,
        max_chars: None,
        line_numbers: false,
        lang_override: None,
        log_stats,
    }))
}

fn parse_duration(s: &str) -> Result<Option<u64>, String> {
    let s = s.trim().to_lowercase();
    if s == "all" {
        return Ok(None);
    }
    if let Some(num) = s.strip_suffix('d') {
        let n: u64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
        return Ok(Some(n * 86_400));
    }
    if let Some(num) = s.strip_suffix('h') {
        let n: u64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
        return Ok(Some(n * 3_600));
    }
    if let Some(num) = s.strip_suffix('m') {
        let n: u64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
        return Ok(Some(n * 60));
    }
    if let Some(num) = s.strip_suffix('s') {
        let n: u64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
        return Ok(Some(n));
    }
    Err(format!(
        "invalid duration '{s}' (use e.g. 24h, 7d, 30d, or all)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn line_range_parses_all_separators_and_rejects_invalid() {
        assert_eq!(
            LineRange::parse("40-120").unwrap(),
            LineRange {
                start: 40,
                end: Some(120)
            }
        );
        assert_eq!(
            LineRange::parse("40:120").unwrap(),
            LineRange {
                start: 40,
                end: Some(120)
            }
        );
        assert_eq!(
            LineRange::parse("40,120").unwrap(),
            LineRange {
                start: 40,
                end: Some(120)
            }
        );
        assert_eq!(
            LineRange::parse("42").unwrap(),
            LineRange {
                start: 42,
                end: Some(42)
            }
        );
        assert_eq!(
            LineRange::parse("100-").unwrap(),
            LineRange {
                start: 100,
                end: None
            }
        );
        assert_eq!(
            LineRange::parse("-50").unwrap(),
            LineRange {
                start: 1,
                end: Some(50)
            }
        );
        assert!(LineRange::parse("0").is_err());
        assert!(LineRange::parse("0-10").is_err());
        assert!(LineRange::parse("50-10").is_err());
        assert!(LineRange::parse("").is_err());
    }

    #[test]
    fn drop_in_aliases_parse_sed_head_tail_and_duration() {
        let Ok(Command::Read(sed_opts)) = parse(&s(&["sed", "-n", "10,30p", "src/main.rs"])) else {
            panic!("expected Read for sed -n");
        };
        assert_eq!(
            sed_opts.range,
            Some(LineRange {
                start: 10,
                end: Some(30)
            })
        );
        assert_eq!(sed_opts.paths, vec!["src/main.rs"]);

        let Ok(Command::Read(head_opts)) = parse(&s(&["head", "-n", "25", "a.rs"])) else {
            panic!("expected Read for head");
        };
        assert_eq!(head_opts.head, Some(25));

        let Ok(Command::Read(tail_opts)) = parse(&s(&["tail", "-15", "a.rs"])) else {
            panic!("expected Read for tail");
        };
        assert_eq!(tail_opts.tail, Some(15));

        assert_eq!(parse_duration("all").unwrap(), None);
        assert_eq!(parse_duration("24h").unwrap(), Some(86_400));
        assert_eq!(parse_duration("7d").unwrap(), Some(7 * 86_400));
        assert!(parse_duration("5w").is_err());
    }

    #[test]
    fn exec_trim_recognizes_trailing_all_and_raw_flags() {
        // trm git diff --all and --raw consume flags without forwarding to git
        let Ok(Command::Git(g_all)) = parse(&s(&["git", "diff", "--all"])) else {
            panic!("expected Git");
        };
        assert!(g_all.all);
        assert_eq!(g_all.args, vec!["diff"]);

        let Ok(Command::Git(g_raw)) = parse(&s(&["git", "diff", "--raw"])) else {
            panic!("expected Git");
        };
        assert!(g_raw.raw);
        assert_eq!(g_raw.args, vec!["diff"]);

        // trm git log --all preserves git's own --all flag
        let Ok(Command::Git(g_log)) = parse(&s(&["git", "log", "--all"])) else {
            panic!("expected Git");
        };
        assert!(!g_log.all);
        assert_eq!(g_log.args, vec!["log", "--all"]);

        // trm find . --all and trm cargo test --raw work in trailing position
        let Ok(Command::Find(f_all)) = parse(&s(&["find", ".", "--all"])) else {
            panic!("expected Find");
        };
        assert!(f_all.all);
        assert_eq!(f_all.args, vec!["."]);

        let Ok(Command::Cargo(c_raw)) = parse(&s(&["cargo", "test", "--raw"])) else {
            panic!("expected Cargo");
        };
        assert!(c_raw.raw);
        assert_eq!(c_raw.args, vec!["test"]);

        // Args after `--` are forwarded verbatim
        let Ok(Command::Git(g_dash)) = parse(&s(&["git", "diff", "--", "--all"])) else {
            panic!("expected Git");
        };
        assert!(!g_dash.all);
        assert_eq!(g_dash.args, vec!["diff", "--", "--all"]);
    }

    #[test]
    fn short_flags_work_for_stats_audit_export_and_ui() {
        let Ok(Command::Stats(st)) = parse(&s(&["stats", "-s", "24h", "-a", "-m", "-l", "-j"]))
        else {
            panic!("expected Stats");
        };
        assert_eq!(st.since_secs, Some(86_400));
        assert!(st.by_agent && st.by_model && st.by_lang && st.json);

        let Ok(Command::Audit(au)) = parse(&s(&["audit", "-s", "7d", "-m", "-b", "-j"])) else {
            panic!("expected Audit");
        };
        assert_eq!(au.since_secs, Some(7 * 86_400));
        assert!(au.by_model && au.bypasses && au.json);

        let Ok(Command::Export(ex)) = parse(&s(&["export", "-s", "24h", "-o", "out.json"])) else {
            panic!("expected Export");
        };
        assert_eq!(ex.since_secs, Some(86_400));
        assert_eq!(ex.out_path.as_deref(), Some("out.json"));
        assert!(ex.redact);

        let Ok(Command::Ui(ui)) = parse(&s(&["ui", "-s", "24h"])) else {
            panic!("expected Ui");
        };
        assert_eq!(ui.since_label.as_deref(), Some("24h"));
    }
}
