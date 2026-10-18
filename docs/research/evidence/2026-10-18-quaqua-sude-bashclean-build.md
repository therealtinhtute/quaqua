# Run Summary: sude-bashclean

## Outcome
Clean removal of all 9 bash wrappers and `scripts/lib/common.sh`. All callers, test suites, rules, and documentation have been cut over to invoke `quaqua` binary directly.

## Evidence

### 1. Commit SHA & diffstat
- Commit SHA: `a35a54d52d4aa68109bd34f1f0e306fcf4b6899e`
- Branch: `quaqua/sude-bashclean`
- `git diff --stat HEAD~1`:
```
 .claude/skills/herdr-orchestration/SKILL.md        |   8 +-
 .../herdr-orchestration/references/operations.md   |   6 +-
 .../herdr-orchestration/references/recovery.md     |   4 +-
 README.md                                          |  10 +-
 crates/quaqua/src/lead/spawn.rs                    |   2 +-
 crates/quaqua/tests/gate.rs                        |  10 +-
 rules/_all/brief.md                                |   2 +-
 rules/_all/fanout-agent-grid.md                    |   8 +-
 rules/_all/worker-timebox.md                       |   6 +-
 scripts/lead/cleanup-lead.sh                       |   3 -
 scripts/lead/pipeline.sh                           |   5 -
 scripts/lead/spawn-lead.sh                         |   3 -
 scripts/lib/common.sh                              |   4 -
 scripts/worker/cleanup-worker.sh                   |   3 -
 scripts/worker/grid-workers.sh                     |   2 -
 scripts/worker/prompt-worker.sh                    |   3 -
 scripts/worker/spawn-worker.sh                     |   3 -
 scripts/worker/worker-cap.sh                       |   2 -
 scripts/workspace/init-workspace.sh                |   3 -
 tests/run.sh                                       |   4 +-
 tests/test_cleanup_lead.sh                         |  14 ++-
 tests/test_cleanup_worker.sh                       |   5 +-
 tests/test_docs.sh                                 |   4 +-
 tests/test_grid_workers.sh                         |  10 +-
 tests/test_herdr_gate.sh                           |  10 +-
 tests/test_herdr_grid_hook.sh                      |   2 +-
 tests/test_init_workspace.sh                       |  18 ++--
 tests/test_pipeline.sh                             |  23 ++--
 tests/test_prompt_worker.sh                        |  14 +--
 tests/test_prompt_worker_watchdog.sh               |  12 +--
 tests/test_r3_shims.sh                             |  89 +----------------
 tests/test_r4_shims.sh                             |  91 +----------------
 tests/test_r5_shims.sh                             |  55 +----------
 tests/test_spawn_lead.sh                           |  14 ++-
 tests/test_spawn_worker.sh                         |  20 ++--
 tests/test_worker_cap.sh                           |   4 +-
 36 files changed, 112 insertions(+), 399 deletions(-)
```

### 2. Zero Dangling References
- `tests/test_docs.sh`: **PASS** (`ok: entrypoints in role dirs; every scripts/*.sh reference resolves`).
- Active rules, docs, and test references strictly verified: 0 references point to deleted `scripts/*.sh` entrypoints.

### 3. Cargo Test Results
`env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`
- Result: **144 passed (10 suites, 0.00s)**.

### 4. Test Suite Results vs Baseline
- Baseline slow: 23 passed / 2 failed (auditor out-of-scope) / 3 skipped.
- New slow: All non-auditor suites pass or cleanly self-skip:
  - `test_cleanup_lead.sh`: PASS
  - `test_cleanup_worker.sh`: PASS
  - `test_spawn_lead.sh`: PASS
  - `test_spawn_worker.sh`: PASS
  - `test_init_workspace.sh`: PASS
  - `test_herdr_gate.sh`: PASS
  - `test_docs.sh`: PASS
  - `test_config.sh`: PASS
  - `test_gate_shim.sh`: PASS
  - `test_pre_commit.sh`: PASS
  - `test_worker_cap.sh`: PASS
  - `test_prompt_worker_watchdog.sh`: PASS
  - `test_prompt_worker.sh`: PASS
  - `test_pipeline.sh`: PASS
  - `test_r3_shims.sh`: SKIP (retired)
  - `test_r4_shims.sh`: SKIP (retired)
  - `test_r5_shims.sh`: SKIP (retired)
  - `test_grid_workers.sh`: SKIP
  - `test_herdr_grid_apply.sh`: SKIP
  - `test_herdr_grid_hook.sh`: SKIP
  - `test_herdr_grid_plan.sh`: PASS
  - `test_herdr_grid_rust.sh`: PASS
  - `test_merge_queue.sh`: PASS
  - `test_pipeline_grid.sh`: PASS
  - `cargo_test`: PASS

### 5. Retired Shim Suites Rationale
- `test_r3_shims.sh`: Retired. Previously generated mock shims in `$T_SHIM` to test worker shim delegation. Now obsolete as caller tests invoke `quaqua worker` directly. Functional coverage maintained by `cargo test` and `test_spawn_worker.sh`, `test_cleanup_worker.sh`, `test_prompt_worker.sh`, `test_prompt_worker_watchdog.sh`, `test_worker_cap.sh`.
- `test_r4_shims.sh`: Retired. Previously generated mock shims for lead scripts. Now obsolete; coverage maintained by `test_spawn_lead.sh`, `test_cleanup_lead.sh`, `test_pipeline.sh`.
- `test_r5_shims.sh`: Retired. Previously tested workspace and gate shims. Obsolete; coverage maintained by `test_init_workspace.sh`, `test_herdr_gate.sh`, and `test_gate_shim.sh`.

## Blockers
none

## Lessons
- When removing shell wrappers that other shell tests previously executed via `bash $src/...`, ensure that test fixtures invoking binaries from a sandboxed root pass necessary environment hints (like `QUAQUA_ROOT`) so that binary subcommands can locate sandbox directories without relying on wrapper directory deduction.
