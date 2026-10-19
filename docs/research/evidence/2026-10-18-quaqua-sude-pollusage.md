# sude-pollusage — Port audit/usage.py to Rust `quaqua usage` subcommand

Branch: `quaqua/sude-pollusage` · commit `49db2fd` (base `8f155d2`)

## Summary of Changes
- **Dependencies**: Added single pinned dependency `ureq = "=2.10.1"` (features `["tls", "json"]` using rustls) in `crates/quaqua/Cargo.toml` and updated `Cargo.lock`. No other dependencies added.
- **Fixtures**: Relocated offline fixtures `scripts/audit/tests/fixtures/{meter.json,request_logs.json}` → `crates/quaqua/tests/fixtures/usage/`.
- **Implementation (`crates/quaqua/src/usage.rs`)**:
  - Full CLI interface parity: `--days`, `--until`, `--json`, `--fixture-dir`, `--local-version`, `--db`, `--repo`, `--runs-dir`.
  - Console request logs: paged backwards (safety limit 200 pages, page limit 100), deduplicated by ID, runtime token via `opencode auth export opencode` (never printed or leaked).
  - Go meter `/inference/go/v1/usage`: parses windows (rolling 5h, weekly 7d, monthly 1m), calculates log costs and range completeness.
  - Local attribution: SQLite read-only mode via CLI (`session_v2`, `session_message`), idle outcome classification, >30m idle gap tracking, product vs process tool-call weighting.
  - Tool classification: pure Rust pattern matching without regex dependencies, validated with 0 diffs across 4,451 tools in live DB.
  - Civil date & UTC+7 calendar math: Howard Hinnant date algorithms for ISO-8601 formatting, UTC parsing, and VN midnight cutoffs.
  - Resilience & degradation: `SourceUnavailable` with per-source isolation. HTTP 5xx/429/network errors and bad-JSON retried up to 10 attempts with capped backoff `[1, 2, 4, 4, ...]`. HTTP 4xx fails fast (1 attempt). Partial logs render report with `[PARTIAL]`, missing meter renders with `[UNAVAILABLE]`, exit code is `0` when logs exist and `2` when request-logs are missing entirely.
  - Tripwire: `contains_token` rejecting output with tokens matching `(?:st_|rt_)[A-Za-z0-9._-]{10,}`.
- **Registration**: Minimal additions to `crates/quaqua/src/main.rs` (12 lines added, no reformatting) and `crates/quaqua/src/lib.rs` (`pub mod usage;`).
- **Tests (`crates/quaqua/tests/usage.rs`)**: 7 comprehensive offline tests covering text report, JSON report, 1-day window, meter unavailable (bad JSON degradation, exit 0), request-logs missing (exit 2 but renders), HTTP retry semantics (5x attempts -> sleeps `[1, 2, 4, 4]`), and token tripwire.
- **Cleanup**: `scripts/audit/` completely removed in final commit `49db2fd`.

## Evidence

### 1. Git Commit & Diff Stat
- Commit: `49db2fd` (`feat(usage): port scripts/audit/usage.py to quaqua usage subcommand with exact parity`)
- Diff stat (`git diff --stat 8f155d2..HEAD`):
```
 Cargo.lock                                         |  528 +++++-
 crates/quaqua/Cargo.toml                           |    1 +
 crates/quaqua/src/lib.rs                           |    1 +
 crates/quaqua/src/main.rs                          |   12 +
 crates/quaqua/src/usage.rs                         | 1830 ++++++++++++++++++++
 crates/quaqua/tests/config.rs                      |    2 +-
 .../quaqua/tests/fixtures/usage}/meter.json        |    0
 .../quaqua/tests/fixtures/usage}/request_logs.json |    0
 crates/quaqua/tests/usage.rs                       |  378 ++++
 scripts/audit/tests/test_usage.py                  |  260 ---
 scripts/audit/usage.py                             |  785 ---------
 11 files changed, 2749 insertions(+), 1048 deletions(-)
```

### 2. Test Verification
Command: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`
Result: `cargo test: 169 passed (11 suites, 0.00s)` (0 failed; baseline was 162 passed).

### 3. Parity Table (Live `--days 1 --until 1792326438330`)

| Metric | Python (`scripts/audit/usage.py`) | Rust (`quaqua usage`) | Diff | % Diff |
| :--- | :--- | :--- | :--- | :--- |
| **Total Cost** | $1.5719 | $1.5719 | $0.0000 | **0.0000%** |
| **Total Requests** | 934 | 934 | 0 | 0.0000% |
| **Input Tokens** | 4,283,983 | 4,283,983 | 0 | 0.0000% |
| **Output Tokens** | 1,073,710 | 1,073,710 | 0 | 0.0000% |
| **Reasoning Tokens** | 842,400 | 842,400 | 0 | 0.0000% |
| **Cache Read Tokens** | 188,784,640 | 188,784,640 | 0 | 0.0000% |
| **Product Calls** | 534 (36% calls, 42% $) | 534 (36% calls, 42% $) | 0 | 0.0000% |
| **Process Calls** | 477 (32% calls, 27% $) | 477 (32% calls, 27% $) | 0 | 0.0000% |
| **Other Calls** | 469 (32% calls, 31% $) | 469 (32% calls, 31% $) | 0 | 0.0000% |
| **Tasks Count** | 19 | 19 | 0 | 0.0000% |
| **Exit Code** | 0 | 0 | 0 | identical |

Parity is exact (0.0000% cost variance, well within ±2% limit).

### 4. Token Leakage Check
- Text output: `grep -cE 'st_|rt_' /tmp/rs_usage.txt` → `0`
- JSON output: `grep -cE 'st_|rt_' /tmp/rs_usage.json` → `0`

### 5. Demo JSON Output (Snapshot)
```json
{
  "generated": "2026-10-18T19:34:59+07:00",
  "offline": true,
  "days": 3,
  "until": "2026-10-16T18:00:00+07:00",
  "cutoff": "2026-10-14T00:00:00+07:00",
  "local_version": "2.0.32",
  "totals": {
    "requests": 5,
    "cost": 0.4,
    "tokens": {
      "input": 42000,
      "output": 3900,
      "reasoning": 1510,
      "cache_read": 390000
    }
  },
  "warn_other_machine": true,
  "machines": [
    {
      "ua": "opencode/latest/2.0.32/cli",
      "version": "2.0.32",
      "requests": 3,
      "cost": 0.33,
      "local": true
    }
  ],
  "exit_code": 0
}
```

### 6. Directory Status
- `scripts/audit/`: deleted (`ls -d scripts/audit` returns non-zero).
- `crates/quaqua/tests/fixtures/usage/`: intact and tracked (`meter.json`, `request_logs.json`).

## Blockers
none

## Lessons
- Porting regex-heavy python audits to Rust without external regex crates is cleanest with targeted substring and boundary checks validated against live sqlite corpus.
