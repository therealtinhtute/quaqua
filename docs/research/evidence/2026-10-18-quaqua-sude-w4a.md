# sude-w4a — Wave 4a Restructure (worker/prompt module split)

## Summary
- Split monolithic `crates/quaqua/src/worker/prompt.rs` (1,002 LOC) into structured submodule `crates/quaqua/src/worker/prompt/`:
  - `observer.rs` (301 LOC): `observe()`, body text fingerprinting, digit normalization, pane inspection, git diff hash/diffstat, liveness checks (`question_seen`, `is_provider_timeout`, `lead_waiting`, `report_ready`).
  - `watchdog.rs` (477 LOC): watchdog loop preserving exact check ordering and exit codes 2..6, stall/idle/budget deadline handling, grace period, autoext diff tracking, soft nudge, and report validation.
  - `cost.rs` (145 LOC): cost budget parsing (`brief_cost_budget`, `resolve_cost_budget`), SQLite and opencode API cost reader (`read_worker_cost`), and `CostTracker` state for 80%/100% budget warnings.
  - `mod.rs` (291 LOC): CLI argument parsing (`parse_prompt_args`), entrypoint (`prompt_worker`), brief checking (`do_check_brief`, `brief_headers_ok`, `brief_budget_sec`), and re-exports of public API surface.
- Enhanced report validation matcher (`is_lessons_heading` & `validate_report_content`): accepts both exact `## Lessons` and numbered headings like `## 5. Lessons`, rejecting reports without a valid Lessons heading or invalid depth (`###`).
- Zero behavior change for exit codes (2..6), timing, and check sequence; full backward compatibility for all callers and tests via re-exports in `prompt/mod.rs`.
- Added unit tests in `crates/quaqua/tests/worker_unit.rs` covering heading matcher variants and CLI argument parsing.

## Evidence
- Commit SHA: `5d22e4b79f1aaf7e8e8b108ed6368fe47f00a87b`
- Branch: `quaqua/sude-w4a` (from `quaqua/clean-integ`, no push)
- `git diff --stat HEAD~1`:
  ```
   crates/quaqua/src/worker/prompt.rs          | 1002 ---------------------------
   crates/quaqua/src/worker/prompt/cost.rs     |  145 ++++
   crates/quaqua/src/worker/prompt/mod.rs      |  291 ++++++++
   crates/quaqua/src/worker/prompt/observer.rs |  301 ++++++++
   crates/quaqua/src/worker/prompt/watchdog.rs |  477 +++++++++++++
   crates/quaqua/tests/worker_unit.rs          |  103 +++
   6 files changed, 1317 insertions(+), 1002 deletions(-)
  ```
- Directory structure:
  ```
  crates/quaqua/src/worker/prompt/
  ├── cost.rs
  ├── mod.rs
  ├── observer.rs
  └── watchdog.rs
  ```
- Build check: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo check --locked` -> Finished in 0.07s (OK, zero errors/warnings).
- Targeted tests: `cargo test --locked --test worker_unit --test pipeline_prompt` -> 58 passed (2 suites: worker_unit 50 passed, pipeline_prompt 8 passed).
- Full suite baseline check: `cargo test --locked` -> 150 passed (10 suites, 0 failed, baseline was 148 passed).

## Risks
- Watchdog timing or exit code regression: mitigated by keeping the loop sequence, branch conditions, and exit code mappings identical to the original monolithic function.
- Public API drift: mitigated by re-exporting all original functions, constants, types, and util re-exports from `worker::prompt::mod`.

## Blockers
none

## Lessons
- Extracting submodules under a folder module with comprehensive re-exports preserves external caller contracts while eliminating god-files.
