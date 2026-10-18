# Fix Report: Task `staletests` (Round 1 Fix)

## Summary
- Branch: `quaqua/sude-staletests` at tip `a708fcd` (parent `bd68969`, base `de20949`).
- Reviewer finding addressed:
  - Finding 1: In `crates/quaqua/tests/pipeline_prompt.rs:493-557` (`pipeline_resume_at_phase_review_with_delivered_report_avoids_exit_6`), neutralized grid check via `GRID_SKIP=1 QUAQUA_TEST=1` (preventing early exit 5), configured fake `prompt-worker.sh` to exit 6 strictly on reviewer wait-file resume while exiting 0 on builder fix prompt, and asserted delivered report consumption (`rounds_run=1`, `last_findings=2`, verdict line in `pipeline-<task>.md`, exit 0). Tested that without the production bypass the test cleanly fails on exit 6.
  - Note 1: Updated `tests/test_spawn_lead.sh:6` header comment to describe `LEAD_KIND` override instead of obsolete `LEAD_MODEL`/`LEAD_EFFORT`.
  - Note 3: Updated `crates/quaqua/src/pipeline/mod.rs:1264` to match verdict line case-insensitively (`to_ascii_lowercase().starts_with("verdict:")`).
- All targeted test suites pass foreground verification:
  - `bash tests/test_spawn_worker.sh`: PASS
  - `bash tests/test_spawn_lead.sh`: PASS
  - `bash tests/test_pipeline.sh`: PASS
  - `cargo test --locked -p quaqua`: 144 passed (10 suites, 0.00s)

## Changes
- `crates/quaqua/src/pipeline/mod.rs`: Case-insensitive match on delivered reviewer verdict prefix.
- `crates/quaqua/tests/pipeline_prompt.rs`: Rigorous regression test for wrinkle (ii) with grid check neutralized and accurate assertion on delivered report processing.
- `tests/test_spawn_lead.sh`: Header comment text alignment with `LEAD_KIND`.

## Verification Commands & Results
- `QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_spawn_worker.sh` -> PASS
- `QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_spawn_lead.sh` -> PASS
- `cargo test --locked -p quaqua` -> 144 passed; 0 failed

## Risks
- None. Changes harden test assertions and improve resilience against letter-casing in reviewer reports.

## Lessons
- none
