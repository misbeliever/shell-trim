# `trm` Design Decisions & Empirical Lessons

Why `trm` is built the way it is—based on real benchmarks and live session audits across Codex, Antigravity, Claude Code, Copilot CLI, and Pi. Future contributors (human or AI) should preserve these invariants unless new data disproves them.

---

## 1. Preserve Muscle Memory: Prefix Original Commands (`trm <cmd>`)

- **Finding:** Live `trm audit` runs on Codex showed that polite rules asking the model to translate syntax (`sed -n '880,960p' App.swift` → `trm read App.swift -r 880-960`) drifted back to raw `sed -n` and `rg -n` after 15–20 turns (~30% adoption).
- **Decision:** Keep `trm read` for power features (`-s <symbol>`, `-c 300`, `--outline`), **and** support drop-in 1-token prefixes with original arguments: `trm cat`, `trm head`, `trm tail`, `trm sed -n 'A,Bp'`, `trm rg`, `trm grep`, `trm ls`, `trm find`, `trm git`, `trm swift`, `trm swiftc`, `trm cargo`.
- **Result:** Post-install Codex adoption jumped to **100%**.

## 2. Always-On Fenced Rules (`AGENTS.md` / `GEMINI.md` / `CLAUDE.md`) Over `SKILL.md`

- **Finding:** Agent skills (`SKILL.md`) use progressive disclosure—only their 1-line description is in context until the model consciously opens them. Quick muscle-memory shell calls (`head -n 5`, `grep`) bypass skills completely.
- **Decision:** `trm install` manages a compact, **4-line `MANDATORY` rule block** fenced by `<!-- trm:start --> ... <!-- trm:end -->` in each agent's global always-on rules file (`~/.codex/AGENTS.md`, `~/.gemini/config/GEMINI.md`, `~/.claude/CLAUDE.md`, `~/.copilot/copilot-instructions.md`, `~/.pi/agent/AGENTS.md`, `~/.cursor/rules/trm.mdc`).
- **Rule of thumb:** Keep the rule block ultra-short (every extra line costs prompt tokens on every turn across every conversation), and never wrap tiny 1-line commands like `wc` whose rule line would cost more than it saves.

## 3. Opt-In Line Numbers (`-L`) & Mandatory Never-Expand Guard

- **Finding:** Benchmarking `trm read` on 74 real Swift files showed that default per-line number padding (` 42|`) added ~10% byte overhead—causing clean files in `minimal` and `raw` mode to **expand by -10.1%** compared to raw `cat`.
- **Decision:**
  - Make `-L` / `--line-numbers` opt-in for `raw`/`minimal`/`aggressive` (while `outline` always prints `Lstart-end|` ranges and `minimal` gap markers print `Lstart-Lend`).
  - Enforce a **never-expand guard** across every trimmer (`read`, `rg`, `git`, `swift`, `cargo`, `ls`, `find`): if trimmed output is `>= raw_bytes`, emit raw output transparently.
- **Result:** Real Swift codebase savings jumped from **18.6% → 49.8%** (`90.6%` on files > 200 lines) with zero negative-savings reads.

## 4. Edit-Safe by Default (Avoiding the "Re-Read Tax")

- **Finding:** Stripping comments or whitespace from code right before an exact-match edit (`edit_file` / `apply_patch`) causes patch failures and forces a full raw re-read—costing _more_ tokens than reading raw once.
- **Decision:**
  - Explicit slices (`-r`, `--head`, `--tail`, `-s <symbol>`, `trm sed -n`, `trm head`, `trm tail`) always output **exact unmodified `raw` lines**.
  - Small files (`≤ 80` lines) pass through `raw` untouched.
  - Large files (`> 260` lines, configurable via `~/.config/trm/config.toml`) default to `outline` with `[Lstart-Lend]` ranges so agents drill into the exact slice they need.

## 5. Sandbox-Safe Telemetry Bridge (`/tmp` → `~/.local/share/trm`)

- **Finding:** Sandboxed agent shells (Codex and Antigravity) cannot write to `~/.local/share/trm/events.jsonl`. Additionally, on macOS `std::env::temp_dir()` (`$TMPDIR` under `/var/folders/...`) can differ between a sandboxed shell and the user's terminal.
- **Decision:** Write to `~/.local/share/trm/events.jsonl` when writable; if blocked by sandbox, spill silently to literal `/tmp/trm-events-<user>.jsonl` (writable in both sandboxes and the user's shell). Whenever `trm stats` or `trm ui` runs in the user's terminal, it automatically drains and appends `/tmp/trm-events-<user>.jsonl` into `~/.local/share/trm/events.jsonl`. Telemetry I/O errors never fail a `trm` command.

## 6. Measure What Actually Matters (`net%`, Per-Session Misses, Per-Line Audit, Model Backfill)

- **Net vs. Gross (`trm stats`):** Gross byte reduction hides failed trims. `trm stats` correlates follow-up reads within 180s scoped to the **same `(agent, session, path)`**:
  - `outline` → narrow `-r` slice (`< 85%` of file) = **outline drill-down (hit)**.
  - Trimmed read → full `raw` re-read (or `rg` → `rg --all`/`--raw`) = **trim miss**, subtracting the wasted first read from `net%`.
- **Automatic model backfill:** Most agents don't export their active model name to child shells. `trm stats` resolves `model: "unknown"` automatically from each agent's session transcript using the logged `session` ID.
- **Per-line audit timestamps (`trm audit`):** Filtering only by session file `mtime` mixes pre-install turns into `--since 30m` windows. `trm audit` parses each turn's ISO-8601 `timestamp`/`created_at` so short windows reflect true live adoption.

## 7. Zero External Crates (Including `trm ui`) & TTY-Aware Output

- **Zero dependencies:** Pure Rust `std` across the entire CLI—including JSONL parsing, ISO-8601 dates, ANSI 256-color styling, and the full-screen `trm ui` TUI (`stty` + alternate screen + RAII terminal restore). Keeps startup `< 2ms`, binary ~850 KB, and offline builds `< 1s`.
- **TTY-aware styling (`src/style.rs`):** Colors and Unicode progress bars activate only when stdout is a TTY and `NO_COLOR` is unset—agent tool calls and pipes always receive 100% plain text (zero ANSI escape tokens).

## 8. Collapse `rg → read` Automatically (Zero New Flags to Memorize)
- **Finding:** Scanning 117 recent Codex sessions showed `rg → cat` (145×) + `rg → sed` (127×) = **272 occurrences in 7 days**—by far the #1 consecutive command pair. Every extra turn forces the model to re-read the entire conversation history (30k–80k input tokens).
- **Decision:** Keep broad searches (> 3 hits) as compact grouped 1-liners, but when `trm rg` returns **1–3 focused hits**, automatically expand each hit to its **enclosing function/method body** (≤ 60 lines, capped at 120 lines total; `--block`/`--no-block` overrides). Models get edit-ready code in 1 call without having to learn a new combined command.

## 9. Scope & Non-Goals (Repo & Shell Traces Only)
- **In scope:** Deterministic, lossless/drill-down compaction of local repo & shell outputs — source files, code searches, directory trees, git status/diff/log, compiler/test runners, and plain-text logs (consecutive-line deduplication).
- **Out of scope (left unfiltered):** External documentation, web fetches/HTML, and local PDFs/specs. Unlike structured code and traces, stripping prose risks silently dropping requirements or edge cases and would break `trm`'s zero-dependency, `< 2ms` startup design.
