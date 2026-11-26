# Changelog

## v0.1.0 — Initial Release
- **`trm read`** (aliases: `trm cat`, `trm head`, `trm tail`, `trm sed -n`): language-aware `auto`, `raw`, `minimal`, `outline`, `aggressive`, `-r` slices, `-s` symbol jump, `--head`/`--tail`/`-c`, multi-file reads, never-expand guard, Swift/Rust raw string & JS regex-safe lexing.
- **`trm rg`** (alias: `trm grep`): file-grouped capped search, directory-grouped `--files`, automatic enclosing-block expansion on 1–3 focused hits (`--block`/`--no-block`).
- **`trm git`**, **`trm swift` / `trm swiftc`**, **`trm cargo`**, **`trm ls`**, **`trm find`**.
- **`trm stats`** (`--by-agent`, `--by-model` with per-event session timeline backfill, `--misses`, `--recommend`), **`trm audit`** (per-line timestamp adoption across Codex, Antigravity, Claude, Copilot CLI, Pi), **`trm ui`** (zero-dep flicker-free live TUI with 7-day sparkline), **`trm doctor`**, and **`~/.config/trm/config.toml`**.
- **`trm install` / `trm uninstall`**: tailored fenced always-on rules for Codex, Antigravity, Claude Code (+ optional `--hook`), Copilot CLI, Pi, and Cursor.
