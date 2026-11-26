# `trm` Before & After Examples

Side-by-side examples showing raw shell output vs. `trm` trimmed output for every command.

---

## 1. Large File Read &nbsp; ![94% saved](https://img.shields.io/badge/saved-~94%25-10B981?style=flat-square) ![trm read](https://img.shields.io/badge/cmd-trm%20read-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-cat%20src%2Ftelemetry.rs-DC2626?style=flat-square)
Dumps **1,204 lines (~9,200 tokens)** into agent context just to find where a function lives:
```rust
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
// ... 1,198 more lines ...
```

### ![After](https://img.shields.io/badge/AFTER-trm%20read%20src%2Ftelemetry.rs-10B981?style=flat-square)
Auto-selects `outline` (> 260 lines) — **~40 lines (~550 tokens)** with exact line ranges:
```text
       L1-8| // 7 imports (std::collections::BTreeMap, std::env, std::fs::{self,, std::io::Write, +3 more)
     L11-28| pub struct Event
     L30-42| impl Event
     L31-41|   fn model_key(&self) -> String
     L45-53| struct TrimMiss
     L56-63| struct Bucket
     L65-67| fn non_empty_env(key: &str) -> Option<String>
    L69-110| fn detect_agent() -> String
   L112-120| fn detect_session() -> Option<String>
   L122-168| fn detect_provider_and_model() -> (String, String)
   L170-187| fn infer_provider(model: &str) -> String
   L189-199| fn primary_log_path() -> PathBuf
   L201-207| fn fallback_log_path() -> Option<PathBuf>
   L209-219| fn append_line(path: &PathBuf, line: &str) -> bool
   ...
[trm: outline of 1204 lines (rust) — 'trm read src/telemetry.rs -r START-END' or '-s <symbol>' for raw code]
```

---

## 2. Symbol Jump &nbsp; ![1 call](https://img.shields.io/badge/round--trips-1%20call-10B981?style=flat-square) ![trm read -s](https://img.shields.io/badge/cmd-trm%20read%20--s-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-rg%20%2B%20sed%20--n-DC2626?style=flat-square)
Agent reads the full file (or runs `rg -n` first, then `sed -n '255,265p'`).

### ![After](https://img.shields.io/badge/AFTER-trm%20read%20src%2Ftelemetry.rs%20--s%20estimate__tokens-10B981?style=flat-square)
Jumps directly to the exact, edit-safe raw body in **1 call**:
```rust
[L257-L259]
pub fn estimate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}
```

---

## 3. Wide JSONL / Log Peek &nbsp; ![99% saved](https://img.shields.io/badge/saved-~99%25-10B981?style=flat-square) ![trm read --head](https://img.shields.io/badge/cmd-trm%20read%20--head%20--c-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-head%20--n%203%20events.jsonl-DC2626?style=flat-square)
Single JSONL lines often exceed 5,000–30,000 characters and flood context.

### ![After](https://img.shields.io/badge/AFTER-trm%20read%20events.jsonl%20--head%203%20--c%20100-10B981?style=flat-square)
```text
{"type":"message","id":"f5b53b9d","parentId":"b91b6604","timestamp":"2026-11-25T03:03:17.118Z","mess… [+28827 chars]
{"type":"toolCall","id":"call_1","name":"bash","arguments":{"command":"trm read src/main.rs"}}
{"type":"toolResult","id":"call_1","output":"fn main() ..."}
```

---

## 4. Code Search &nbsp; ![90% saved](https://img.shields.io/badge/saved-~90%25-10B981?style=flat-square) ![trm rg](https://img.shields.io/badge/cmd-trm%20rg-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-rg%20--n%20%22pub%20fn%22%20src%2F-DC2626?style=flat-square)
Repeats the full file path and leading indentation on every single match, with no cap on match floods or wide lines:
```text
src/cli.rs:11:    pub fn as_str(&self) -> &'static str {
src/cli.rs:21:    pub fn parse(s: &str) -> Result<Self, String> {
src/cli.rs:42:    pub fn parse(s: &str) -> Result<Self, String> {
src/cli.rs:92:    pub fn format(&self) -> String {
src/cli.rs:246:pub fn parse(args: &[String]) -> Result<Command, String> {
src/swift.rs:7:pub fn run(bin: &str, opts: &ExecTrimOptions) -> Result<ExitCode, String> {
src/swift.rs:124:pub fn trim_swift_build(raw: &str, success: bool) -> String {
src/swift.rs:171:pub fn trim_swift_test(raw: &str, success: bool) -> String {
```

### ![After](https://img.shields.io/badge/AFTER-trm%20rg%20%22pub%20fn%22%20src%2F-10B981?style=flat-square)
Groups by file, strips indentation noise, caps at 6/file & 50 total (with overflow summary + `--all` escape hatch):
```text
src/cli.rs (5)
  11: pub fn as_str(&self) -> &'static str {
  21: pub fn parse(s: &str) -> Result<Self, String> {
  42: pub fn parse(s: &str) -> Result<Self, String> {
  92: pub fn format(&self) -> String {
  246: pub fn parse(args: &[String]) -> Result<Command, String> {
src/search.rs:14: pub fn run(opts: &SearchOptions) -> Result<ExitCode, String> {
src/swift.rs (3)
  7: pub fn run(bin: &str, opts: &ExecTrimOptions) -> Result<ExitCode, String> {
  124: pub fn trim_swift_build(raw: &str, success: bool) -> String {
  171: pub fn trim_swift_test(raw: &str, success: bool) -> String {
```

> **Focused search bonus (1–3 hits):** When `trm rg` matches 1–3 locations, it automatically expands each hit into its enclosing function body (`src/Calc.swift [L4-L7] (match L5): ...`) so the agent can edit in **1 tool call** instead of running `rg` followed by `read`.

---

## 5. File Listing &nbsp; ![65% saved](https://img.shields.io/badge/saved-~65%25-10B981?style=flat-square) ![trm rg --files](https://img.shields.io/badge/cmd-trm%20rg%20--files-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-rg%20--files%20Sources%2F-DC2626?style=flat-square)
Repeats the full directory prefix on every line:
```text
Sources/App/Coordinators/AppDelegate.swift
Sources/App/Coordinators/DictationController.swift
Sources/App/Coordinators/RecordingController.swift
Sources/App/Coordinators/WindowCoordinator.swift
```

### ![After](https://img.shields.io/badge/AFTER-trm%20rg%20--files%20Sources%2F-10B981?style=flat-square)
Directory-grouped:
```text
Sources/App/Coordinators/ AppDelegate.swift, DictationController.swift, RecordingController.swift, WindowCoordinator.swift
```

---

## 6. Swift Build & Test &nbsp; ![95% saved](https://img.shields.io/badge/saved-~95%25-10B981?style=flat-square) ![trm swift](https://img.shields.io/badge/cmd-trm%20swift-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-swift%20test-DC2626?style=flat-square)
```text
Building for debugging...
[1/18] Compiling DictationCore Backend.swift
[2/18] Compiling DictationCore Activity.swift
...
[18/18] Linking AppTests
Build complete! (2.14s)
Test Suite 'All tests' started at 2026-11-26 00:45:10.112
Test Case '-[AppTests testStart]' started.
Test Case '-[AppTests testStart]' passed (0.001 seconds).
... (80 more passing tests) ...
Test Case '-[AppTests testRetry]' started.
Sources/Tests.swift:42: error: -[AppTests testRetry] : XCTAssertEqual failed: ("1") is not equal to ("2")
Test Case '-[AppTests testRetry]' failed (0.003 seconds).
Executed 82 tests, with 1 failure (0 unexpected) in 0.142 (0.155) seconds
```

### ![After](https://img.shields.io/badge/AFTER-trm%20swift%20test-10B981?style=flat-square)
Strips all compile progress and passing tests, keeping **only** failures + summary:
```text
Sources/Tests.swift:42: error: -[AppTests testRetry] : XCTAssertEqual failed: ("1") is not equal to ("2")
Test Case '-[AppTests testRetry]' failed (0.003 seconds).
Executed 82 tests, with 1 failure (0 unexpected) in 0.142 (0.155) seconds
```

---

## 7. Rust Build & Test &nbsp; ![96% saved](https://img.shields.io/badge/saved-~96%25-10B981?style=flat-square) ![trm cargo](https://img.shields.io/badge/cmd-trm%20cargo-F97316?style=flat-square)

### ![Before](https://img.shields.io/badge/BEFORE-cargo%20test-DC2626?style=flat-square)
```text
   Compiling trm v0.1.0 (/path/to/shell-trim)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.44s
     Running unittests src/main.rs (target/debug/deps/trm-...)

running 2 tests
test swift::tests::trims_swift_build_noise_and_keeps_errors ... ok
test swift::tests::trims_xctest_and_swift_testing_passes_but_keeps_failures ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/integration.rs (target/debug/deps/integration-...)

running 17 tests
test binary_file_is_safely_skipped ... ok
... (16 more passing tests) ...

test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

### ![After](https://img.shields.io/badge/AFTER-trm%20cargo%20test-10B981?style=flat-square)
```text
test result: ok. 19 passed; 0 failed
```

---

## 8. Net Savings & Real Adoption &nbsp; ![trm stats](https://img.shields.io/badge/cmd-trm%20stats-F97316?style=flat-square) ![trm audit](https://img.shields.io/badge/cmd-trm%20audit-3B82F6?style=flat-square)

```text
$ trm stats --by-agent --by-model --recommend
trm stats (5 commands)
─────────────────────────────────────────────────────────────────
  Raw input:         24,389 tokens  (95.3 KB)
  Trimmed output:     2,905 tokens  (11.3 KB)
  Gross saved:       21,484 tokens  (88.1%)
  Re-read waste:          0 tokens  (0 trim misses, 1 outline drill-downs)
  Net saved:         21,484 tokens  (88.1%)

By model:
  provider/model              reads   raw_tok   out_tok misses drill    net%
  google/gemini-4-neon-medium     5    24,389     2,905      0     1   88.1%
```
