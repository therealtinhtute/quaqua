# Wave 3 Restructure: Launch Engine Extraction & Pure Rust INDEX.md Lock (Fix Round 1)

## Summary
Resolved the 2 review findings from `suhuynh-w3r1`:
1. **F1 [Resolved]**: Removed unused imports `claude_trust_dialog_up` and `LaunchCmd` in `crates/quaqua/tests/worker_unit.rs`. Verified `cargo check --locked --all-targets` produces **0 warnings, 0 errors**.
2. **F2 [Resolved]**: Refined the doc comments for `update_project_index_owner` and `update_index_content` in `crates/quaqua/src/lead/spawn.rs` to explicitly specify the domain: standard LF markdown files with well-formed table rows (`| slug | repo | owner | ... |`). Added a unit test in `worker_unit.rs` pinning the behavior on malformed rows (< 4 fields) confirming they remain untouched.

---

## Evidence

### Commits & Diffs
- Base: `quaqua/clean-integ` (`20cd4d2`)
- Commits on branch `quaqua/sude-w3`:
  - `f33ddfa` `refactor(quaqua): extract launch engine and pure rust index lock (wave 3)`
  - `463732f` `fix(quaqua): clean up unused test imports and refine table update doc contract`
- `git diff --stat 20cd4d2..HEAD`:
```text
 crates/quaqua/src/launch.rs        | 170 +++++++++++++++++++++++++++++++++++++
 crates/quaqua/src/lead/cleanup.rs  |  44 +---------
 crates/quaqua/src/lead/spawn.rs    | 122 +++++++++++++++++---------
 crates/quaqua/src/lib.rs           |   1 +
 crates/quaqua/src/worker/spawn.rs  | 163 +----------------------------------
 crates/quaqua/tests/worker_unit.rs | 136 ++++++++++++++++++++++++++++-
 tests/test_cleanup_lead.sh         |  31 +------
 tests/test_spawn_lead.sh           |  32 +------
 8 files changed, 394 insertions(+), 305 deletions(-)
```

### Checks & Test Gates
- Compiler check:
  `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo check --locked --all-targets`
  Result: **0 warnings, 0 errors**.
- Cargo test suite:
  `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`
  Result: **150 passed; 0 failed; 0 ignored** across 10 test suites.
- Worker unit suite:
  `cargo test --locked --test worker_unit`
  Result: **49 passed; 0 failed; 0 ignored**.
- Grep constraints:
  - `git grep 'Command::new("awk")' crates/quaqua/src/lead/` -> 0 matches.
  - `git grep 'crate::worker::spawn' crates/quaqua/src/lead/` -> 0 matches.
- Shell test gates:
  - `tests/test_cleanup_lead.sh` -> PASS
  - `tests/test_spawn_lead.sh` -> PASS

---

## Blockers
none

## Lessons
- Running `cargo check --all-targets` catches unused imports and warnings in test files that standard `cargo check` ignores.
