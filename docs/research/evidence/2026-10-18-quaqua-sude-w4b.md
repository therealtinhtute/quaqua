# sude-w4b — Wave 4b Restructure (worktree lifecycle extraction)

## Summary
- Extracted git worktree lifecycle management out of `crates/quaqua/src/worker/spawn.rs` into dedicated module `crates/quaqua/src/worker/worktree.rs` (186 LOC):
  - Path canonicalization & resolution: `canonicalize_repo_path`, `resolve_worktree_path`, `resolve_paths`.
  - Commit-ish verification: `validate_base_ref` validating `--base <ref>` against repo commits before any worktree/slot allocation.
  - Worktree creation & reuse: `prepare_worktree` (aliased as `setup_worktree`) handling registered worktree reuse (resumed spawn), foreign directory refusal, existing branch reuse for same-name retries (`worktree add` without `-b`), and fresh worktree creation.
  - Security push block: `apply_push_block` ensuring core.bare and core.worktree fail-closed guards, enabling `extensions.worktreeConfig`, and blocking pushurl for all configured remotes.
  - Worktree removal & dirty checks: `remove_worktree` and `is_worktree_dirty`.
- Refactored `crates/quaqua/src/worker/spawn.rs` to call `canonicalize_repo_path`, `resolve_worktree_path`, `validate_base_ref`, and `prepare_worktree`, reducing `spawn.rs` by 74 LOC while preserving exact ordering, error messages, and git arguments.
- Re-exported worktree functions in `crates/quaqua/src/worker/mod.rs` and `spawn.rs` for full backward compatibility across all modules and tests.
- Added comprehensive unit tests in `crates/quaqua/tests/worker_unit.rs` covering path resolution, base ref validation, foreign dir refusal, worktree reuse, push blocking, dirty checking, and worktree removal.

## Evidence
- Commit SHA: `c404f9b09ff32a38e3cbf2a1ef8d84806ecde461`
- Branch: `quaqua/sude-w4b` (clean, unpushed, based on `quaqua/clean-integ` at `156771c`)
- `git diff --stat HEAD~1`:
  ```
   crates/quaqua/src/worker/mod.rs      |   6 +
   crates/quaqua/src/worker/spawn.rs    |  82 ++-----------
   crates/quaqua/src/worker/worktree.rs | 186 ++++++++++++++++++++++++++++++
   crates/quaqua/tests/worker_unit.rs   | 215 +++++++++++++++++++++++++++++++++++
   4 files changed, 415 insertions(+), 74 deletions(-)
  ```
- Cargo check: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo check --locked` → Finished in 0.01s (OK, zero errors/warnings).
- Targeted test suite: `cargo test --locked --test worker_unit` → 58 passed (1 suite, 0.18s; 55 baseline + 3 new tests, 0 failed).
- Bash spawn worker suite: `tests/test_spawn_worker.sh` → all 56 assertions PASS.

## Risks
- Low: Extracted functions retain 100% identical git arguments, sequencing, and exact error text. All tests across unit and integration suites pass.

## Blockers
none

## Lessons
- Isolating git worktree lifecycle into its own domain module decouples spawn orchestration from filesystem/VCS mechanics while keeping preconditions hermetically testable.
