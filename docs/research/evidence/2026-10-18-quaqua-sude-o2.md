# sude-o2 — Prune dead code in `tests/test_grid_workers.sh`

- task: sude-o2
- repo: ~/.wt/sude-o2
- branch: `quaqua/sude-o2`
- base: `quaqua/clean-integ` (`e88fad3`)
- commit: `d1de5f5` (`test(grid): prune dead code in self-skipping test_grid_workers.sh`)

## Summary of Changes
Cleaned up dead code in `tests/test_grid_workers.sh`:
1. **Removed Dead Test Body & Herdr Stub**:
   - `scripts/worker/grid-workers.sh` was replaced by `quaqua grid reconcile` in the R6 cutover, making the script unconditionally exit at line ~93 with `SKIP`.
   - Removed ~1,000 lines of dead bash code below the skip: herdr stub (`pane/agent/tab/api`), fixture builder (`mkstate`), helper functions (`plan`, `run`, `run_fail`), and 53 obsolete bash test cases.
   - Removed unused temp directory creation (`mktemp -d`) and trap handler.
2. **Retained Audit Mapping & Skip Contract**:
   - Kept shebang, error flags (`set -euo pipefail`), and full Finding 2 (suhuynh-r6r1) case mapping comment documenting corresponding Rust implementations in `crates/quaqua/tests/grid.rs` (33 tests) and `tests/test_herdr_grid_rust.sh`.
   - Kept exact `SKIP: grid-workers replaced by quaqua grid reconcile (covered by test_herdr_grid_rust.sh)` output and `exit 0` matching `tests/run.sh` skip detection.

## Dead Code Analysis
- Checked callers via grep: no other test or script sources `tests/test_grid_workers.sh`.
- Evaluated dead body utility: the deleted stubs and helper functions specifically targeted the legacy bash script `grid-workers.sh` and its CLI arguments (`--repair`, `--dry-run`, `--check`, `--only`). All valid invariants are already covered in Rust unit tests (`crates/quaqua/tests/grid.rs`) and `tests/test_herdr_grid_rust.sh`; none are needed by other bash suites.

## Evidence
- Commit SHA: `d1de5f5ce795f823421fa9676de7779361add0ce`
- `git diff --stat`:
  ```text
  tests/test_grid_workers.sh | 1035 --------------------------------------------
  1 file changed, 1035 deletions(-)
  ```
- Line counts (`wc -l`):
  - Before: 1121 lines
  - After: 86 lines (-1035 lines)
- Test verification:
  ```text
  $ bash tests/test_grid_workers.sh; echo "exit=$?"
  SKIP: grid-workers replaced by quaqua grid reconcile (covered by test_herdr_grid_rust.sh)
  exit=0
  ```
  Matches `tests/run.sh` check (`grep -q '^SKIP'` on log, increments skipped count, exit 0).

## Risks
- None. Suite behavior is unchanged: self-skips cleanly with exit 0 and prints the exact expected SKIP message.

## Blockers
none

## Lessons
- Pruning unreachable code below an early self-skip `exit 0` removes test maintenance burden and eliminates unnecessary tempdir creation during test runs.
