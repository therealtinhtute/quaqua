# Wave 2 Restructure — JSON Parser Elimination, State IO Normalization, Cleanup Fix

## Changes
- **Eliminated `crates/quaqua/src/json.rs`** (-276 LOC dead weight):
  - Removed `pub mod json;` from `crates/quaqua/src/lib.rs`.
  - Deleted `crates/quaqua/src/json.rs`.
  - Migrated all 7 call sites across `src/cap.rs`, `src/lead/cleanup.rs`, `src/lead/spawn.rs`, `src/worker/cleanup.rs`, `src/worker/prompt.rs`, `src/worker/spawn.rs`, and `src/worker/stop.rs` to standard `serde_json::Value`.
- **Created `crates/quaqua/src/state.rs`** (+322 LOC):
  - Provided `atomic_write(path, content)` with temporary file in the same directory and atomic rename.
  - Defined `WorkerMetadata` helpers for reading/writing `.owner`, `.grid`, `.{name}.budget`, `.{name}.grace`, `.{name}.autoext`, `.{name}.cost80`, `.{name}.cost100`, `.{name}.provider_retry`, etc.
  - Defined `ExecutionState` (Pipeline & Taskforce) with typed fields and automatic owner extraction, simplifying `cap.rs` by ~100 LOC.
- **Fixed `worker/cleanup.rs` (Intentional Fix c)**:
  - `WorkerMetadata::cleanup_worker_files` now cleans up all 4 previously missed file types (`.{name}.budget`, `.{name}.grace`, `.{name}.cost80`, `.{name}.cost100`) as well as all transient worker files.
- **Added tests in `crates/quaqua/tests/worker_unit.rs`** (+123 LOC):
  - `test_cleanup_worker_cleans_budget_grace_cost_and_metadata`: asserts cleanup of all 4 files plus metadata files.
  - `test_state_atomic_write_and_metadata_roundtrip`: verifies atomic write, metadata get/set, and execution state parsing.

## Evidence
- Commit SHA: `dfff1381c53eac663352756f6ae08b3956037c01` on branch `quaqua/sude-w2`.
- `git diff --stat HEAD~1`:
  ```
   crates/quaqua/src/cap.rs            | 246 ++++++++++-----------------
   crates/quaqua/src/json.rs           | 276 -------------------------------
   crates/quaqua/src/lead/cleanup.rs   |  62 +++----
   crates/quaqua/src/lead/spawn.rs     |  12 +-
   crates/quaqua/src/lib.rs            |   2 +-
   crates/quaqua/src/state.rs          | 322 ++++++++++++++++++++++++++++++++++++
   crates/quaqua/src/worker/cleanup.rs |  32 +---
   crates/quaqua/src/worker/prompt.rs  |  67 ++++----
   crates/quaqua/src/worker/spawn.rs   |  19 ++-
   crates/quaqua/src/worker/stop.rs    |   6 +-
   crates/quaqua/tests/worker_unit.rs  | 123 ++++++++++++++
   11 files changed, 624 insertions(+), 543 deletions(-)
  ```
- Grep proof for `crate::json`: 0 occurrences. `crates/quaqua/src/json.rs` deleted.
- Test counts: 148 passed / 0 failed (full workspace suite).
- Targeted test suites: 59 passed / 0 failed (`--test worker_unit --test config --test cli`).
- Section (c) verification: `test_cleanup_worker_cleans_budget_grace_cost_and_metadata` passes cleanly.

## Test cmd+result
- Full test suite:
  `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`
  Result: **148 passed (10 suites, 0.00s), 0 failed** (baseline 147 passed).
- Targeted test suites:
  `cargo test --locked --test worker_unit --test config --test cli`
  Result: **59 passed (3 suites, 0.03s), 0 failed** (baseline 57 passed).
- Shell integration test:
  `tests/test_cleanup_worker.sh`
  Result: **PASS**.

## Risks
- Low risk: All disk file formats (`<owner>\n`, `<grid>\n`, `<budget>\n`, empty grace sentinel, `<autoext>\n`, `<cost>\n`, `key=value\n` state lines) retain 100% byte-for-byte backward compatibility with existing readers and writers.

## Lessons
- Centralizing multi-file worker lifecycle state into a single module (`state.rs`) prevents leaky cleanup omissions that happen when writer and cleaner maintain separate file lists — 2026-10-18, runs/sude-w2.md.

## Blockers
- none
