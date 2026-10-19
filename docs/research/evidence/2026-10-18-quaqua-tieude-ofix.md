# Run Summary: tieude-ofix — stale post-cutover refs cleanup (O1+O5, bashclean r1)

Branch `quaqua/tieude-ofix` (base `quaqua/clean-integ`, 79f8197 → commit 83a8c56). Note: the brief says branch `quaqua/sude-ofix`, but no such branch exists and the spawn placed this worktree on `quaqua/tieude-ofix`; committed there. No push.

## Changes (commit 83a8c56, 5 files, +9/−9)

- `scripts/hooks/herdr-gate.sh:34` — fallback message: dropped the 5 deleted wrapper names; now "only via the quaqua binary — install quaqua or set QUAQUA_BIN, then retry". Gate contract untouched: same stdin grep, exit 2 blocked / exit 0 allow; the `not found` + `fail-closed` diagnostics test_gate_shim.sh greps live on the untouched line 33.
- `scripts/hooks/pre-commit:27` — comment: `# Installed by `quaqua init` via `core.hooksPath`.` (was init-workspace.sh).
- `scripts/auditor/auditor.sh:153` — auditor prompt: pipeline runs "written by `quaqua pipeline`" (was scripts/lead/pipeline.sh).
- `tests/run.sh:25-27` — comment: suites drive `quaqua worker prompt` (in-process pre-prompt grid check) / `quaqua pipeline` (plus the worker steps it runs in-process).
- `tests/test_spawn_worker.sh:2` — header: "Sandbox test for `quaqua worker spawn` name validation"; `:535` also refreshed (`cleanup-worker.sh` → `quaqua worker cleanup`) — one extra comment line beyond the 5 listed spots, same stale class inside a scope file; trivially revertable.
- Untouched by design: `crates/quaqua/src/pipeline/mod.rs` (`pipeline.sh --resume` hints left for the final pass).

## Evidence

Commit: `83a8c5693a0fdc653d8d860346784dc97cdc1aa4`

```
 scripts/auditor/auditor.sh  | 2 +-
 scripts/hooks/herdr-gate.sh | 2 +-
 scripts/hooks/pre-commit    | 2 +-
 tests/run.sh                | 6 +++---
 tests/test_spawn_worker.sh  | 6 +++---
 5 files changed, 9 insertions(+), 9 deletions(-)
```

Targeted suites (foreground; all PASS):
- `bash tests/test_docs.sh` → PASS
- `bash tests/test_gate_shim.sh` → PASS (hook contract intact)
- `bash tests/test_pre_commit.sh` → PASS (pre-commit touched)
- `bash tests/test_spawn_worker.sh` → PASS
- `QUAQUA_BIN=$PWD/target/debug/quaqua bash tests/test_prompt_worker.sh` → PASS

Bare `bash tests/test_prompt_worker.sh` fails at the first arg check with QUAQUA_BIN unset: `${QUAQUA_BIN:-quaqua}` resolves the mise shim, which errors under the suite's sandboxed HOME (`mise ERROR quaqua is not a valid shim`, exit 1, no usage on stderr). Not the diff — `src/` and this suite are untouched; `tests/run.sh:66-67` exports `PATH`+`QUAQUA_BIN`, and the shim error reproduces standalone under a temp HOME.

Grep proof over the 5 files:
- strict `\b(spawn-worker|prompt-worker|cleanup-worker|grid-workers|worker-cap|spawn-lead|cleanup-lead|pipeline|init-workspace|common)\.sh` → no matches (rc=1).
- the brief's plain `prompt-worker\.sh|pipeline\.sh|spawn-worker\.sh` → only `test_pipeline.sh` / `test_prompt_worker.sh` suite filenames remain (exempt: real files, required by run.sh's slow_cover).

## Risks

- No behavior change anywhere: every edit is comment or diagnostic text; gate/exit codes, pre-commit logic, auditor prompt semantics, and test logic are byte-identical outside the strings.
- The herdr-gate fallback message is user-facing; no test asserts its content (test_gate_shim.sh asserts only `not found`/`fail-closed`, both preserved).

## Left standing (found while grepping; outside this task's 5 files — not done here)

- Stale refs in other test headers/comments: test_prompt_worker.sh:2,27; test_prompt_worker_watchdog.sh:2,122,542; test_cleanup_worker.sh:2,116; test_cleanup_lead.sh:2; test_spawn_lead.sh:2; test_init_workspace.sh (~14 lines); test_pre_commit.sh:10,29; test_herdr_grid_hook.sh:52,579,658,664; test_pipeline.sh:2,38,143,157,283-284 (+ its :317 sandbox fixture); test_pipeline_grid.sh stubs; `.claude/skills/herdr-orchestration/references/operations.md:29,38`.
- Pinned — do not clean blind: `tests/test_config.sh:98` asserts 4 `worker-cap.sh` rows in the rule→key table; `docs/research/herdr-grid-rust-migration-plan.md` is historical (per suhuynh-bashcleanr1); `tests/test_grid_workers.sh` body is dead below its unconditional skip (O2).

## Blockers
none

## Lessons
- Run targeted suites the way the runner does — export `QUAQUA_BIN` (tests/run.sh:66-67); a bare `bash tests/test_*.sh` can bind the mise shim and fail spuriously under the sandboxed HOME.
