# Review — task `staletests` (quaqua/sude-staletests, tip bd68969 vs base de2094)

Diff under review: `git diff de2094995a2640e7d53cea75c1ecea3b2c98b1a5` → 5 files, +160/−36, all within the brief's allowed file list (no scope creep):
`crates/quaqua/src/pipeline/mod.rs`, `crates/quaqua/tests/pipeline_prompt.rs`, `tests/test_pipeline.sh`, `tests/test_spawn_lead.sh`, `tests/test_spawn_worker.sh`. `test_r3_shims.sh`/`test_r4_shims.sh` unchanged (they delegate).

## Verification run (reviewer, foreground)

| Command | Result |
|---|---|
| `bash tests/test_spawn_worker.sh` (QUAQUA_BIN=worktree target/debug) | rc=0 PASS |
| `bash tests/test_spawn_lead.sh` | rc=0 PASS |
| `bash tests/test_pipeline.sh` | rc=0 PASS |
| `bash tests/test_r3_shims.sh` | rc=0 ALL TARGET SUITES PASSED |
| `cargo test --locked -p quaqua --test pipeline_prompt` | 8 passed, 0 failed (both new tests included), 59.5s |
| Scenario replica of new test #2, fixed binary (env identical to the Rust test) | **exit 5**, stderr `warning: grid check failed (herdr api snapshot failed: …)`, state `phase=review` — exited at the grid hook, before the code under test |
| Same scenario, temp-build variant with only the delivered-report bypass reverted | **exit 5** (same assertion outcome as the fixed binary → test would pass on the buggy code too) |
| Same variant + `GRID_SKIP=1 QUAQUA_TEST=1` | **exit 6** (wrinkle still reproduces without the fix) |
| Same scenario, fixed binary + `GRID_SKIP=1 QUAQUA_TEST=1` (fake writes a fix report) | exit 0; `pipeline-pfx8.md` gets `- review round 1: Verdict: 2 findings`; state `phase=fix`, `rounds=2` → the production bypass works |

## Findings

1. **The wrinkle-(ii) regression test cannot fail on the buggy code — it passes for the wrong reason, and cannot pass on the intended path.** `crates/quaqua/tests/pipeline_prompt.rs:493-538`.
   - In the test env the pipeline exits **5** at the grid hook (`crates/quaqua/src/pipeline/mod.rs:1250-1254`), which runs *before* the changed block (`:1261-1269`); `assert_ne!(out.status.code(), Some(6))` (`:527-532`) therefore holds with and without the fix. Proven: replica of the test env on the fixed binary → exit 5; temp-build variant with the bypass reverted → exit 5 too; the bug only shows (exit 6) when the grid check is neutralized (`GRID_SKIP=1 QUAQUA_TEST=1`).
   - The fake `prompt-worker.sh` exits 6 on **every** call (`:514-517`), so if the grid hook did pass, the FIX phase's `wait_file` (`crates/quaqua/src/pipeline/mod.rs:1367`) would exit 6 anyway — the test can only pass because of the unrelated grid failure. The comment "advance phase to fix" (`:519`) is not asserted anywhere.
   - Why it matters: wrinkle (ii)'s production fix is correct (verified above on the intended path), but no test guards it — a future removal of the bypass would go unnoticed.
   - Suggested shape of the fix (builder's call): neutralize the grid hook the way other tests do (fake herdr with a valid snapshot via `HERDR_BIN_PATH`, or `GRID_SKIP=1`+`QUAQUA_TEST=1`), make the fake exit 6 only for `--resume` and exit 0 (or write a report) for the fresh fix prompt, and assert the review phase actually consumed the delivered report (e.g. `rounds_run`/`phase=fix`, or a `review round 1` line in `pipeline-<task>.md`).

## Notes (not counted as findings)

- `tests/test_spawn_lead.sh:6` — header comment still says "the LEAD_MODEL/LEAD_EFFORT overrides"; those env vars no longer exist (dc3368d); the section it describes now tests `LEAD_KIND` (`:177-186`). Stale text only.
- `tests/test_pipeline.sh:213` — the reviewer assertion changed from `--permission-mode dontAsk` (claude read-only profile) to `--kind opencode`. Today's opencode review launch has no enforced read-only profile (`crates/quaqua/src/worker/spawn.rs:201-215` ignores `review`; the claude branch `:239-251` adds `dontAsk`+Edit denies), so the old property no longer exists on this path. Consequently the pipeline's reviewer read-only property is now asserted only for claude kinds (in `test_spawn_worker.sh`). Not a defect of this diff (pre-existing since the Claude-out config change); flagging for the Lead.
- `crates/quaqua/src/pipeline/mod.rs:1262-1265` — the delivered-report predicate accepts only `Verdict:` (capital), while the verdict parser (`:1300-1306`) also accepts `verdict:`. A report carrying only the lowercase form would still be re-waited via `prompt-worker.sh --resume` (the exact code path the fix avoids). Low risk: the review prompt mandates the capital form.

## Assessment of the rest of the diff

- Wrinkle (i) `--resume … --rounds N` (`crates/quaqua/src/pipeline/mod.rs:443-512`): parse handles `--rounds`/`--rounds=N`, rejects missing/zero/non-numeric, keeps positional/unknown-flag behavior, and only raises `state.rounds`. `pipeline_resume_with_rounds_raises_cap` (`crates/quaqua/tests/pipeline_prompt.rs:473-490`) is a valid guard: pre-fix the resume form was rejected (usage error, exit 1, `rounds` unchanged), so it does fail on the old code.
- Wrinkle (ii) fix (`crates/quaqua/src/pipeline/mod.rs:1261-1269`): correct on the intended path (replica above); no stale-report hazard found — round-scoped `revname` and the spawn-time `archive_stale` (`:1201-1204`) make a pre-prompt leftover impossible without a hand-edited state.
- Shell-test expectation updates match today's real behavior (verified by the passing suites): new `--review` refusal message (`spawn.rs:349-355`), same-harness pair tieude/suhuynh (both opencode in `team.toml`), `can_review = false` message (`pipeline/mod.rs:623-625`), opencode lead defaults + `LEAD_KIND` override, trust-dialog cases moved to a claude kind (`truonglao`).

## Blockers

- `bash tests/test_r4_shims.sh` did not complete within the reviewer's shell timeout (it delegates to `test_spawn_lead.sh`, which passes — not observed directly).
- Full `cargo test --locked -p quaqua` (all test targets) and `bash tests/run.sh --slow` (25/0/3 gate) were not run — the latter is the Lead's gate. Only the pipeline_prompt target was run plus the four shell suites above.
- Verification built `target/` inside the review worktree (build artifacts only; no source or report files touched outside `~/quaqua/runs`).

Verdict: 1 findings
