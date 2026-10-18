# w5 review — round 1/2 (suhuynh-w5r1)

**Diff reviewed:** `quaqua/sude-w5` tip `7d3dc4e` vs base `6048a28` — 11 files, +1631/−1150.
**Scope check:** all touched paths are inside the brief's allowed set (`crates/quaqua/src/pipeline/**`, `crates/quaqua/src/worker/cleanup.rs`, `tests/pipeline_prompt.rs`, `tests/worker_unit.rs`); `src/worker/prompt.rs` untouched. ✓
**Brief evidence greps (re-verified):** 0 `find_engine_script`, 0 `harness_of(` in `crates/`, 0 `Command::new("bash")` under `src/pipeline/`. ✓

**Tests I re-ran (this worktree, tip; base was not runnable in read-only review):**
- `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **148 passed; 0 failed** (10 targets) — matches the builder's reported 148.
- same env, `cargo test --locked -p quaqua --test pipeline_prompt --test worker_unit` → **9 + 46 passed; 0 failed**.
- Gate evidence: on this diff `git diff --stat`'s last integer = **1150**, `--numstat` add+del sum = **2781** (basis of F1).

## Verified OK
- (1) decomposition: `pipeline/mod.rs` 1406→569 LOC dispatcher; `state.rs` (load/save moved verbatim); `phases/{spawn,plan,build,review,fix}.rs` ≤218 LOC; exit codes 0/1/3/4/5 and the corrupt-phase Err path preserved (`mod.rs:541-568`).
- (2) trampoline removal is complete; env scoping saved/restored (`spawn.rs:24-58`, `review.rs:31-65`, `review.rs:117-140`, `phases/mod.rs:200-217,229-246`); `PIPELINE_TASK` still set during spawn so `.{name}.pipeline`/cap accounting keeps working (`spawn.rs:28`).
- (3) `harness_of` table gone; `harness_of_kind` = `cfg.kinds` (`phases/mod.rs:544-546`).
- (4) failing-first: base `has_real_blockers` (`6048a28:…/mod.rs:232-256`) returns `true` for `- None. All done...`, so the new informational-note asserts (`worker_unit.rs:794-799`) fail on base. ✓

## Findings

**F1 (High) — auto review gate compares the wrong number (behavior change beyond (4)+(5)).**
`phases/build.rs:53-90` replaces the base numstat gate (`6048a28:…/pipeline/mod.rs:1127-1159`) with `git diff --stat <base> HEAD`, then keeps the **last** parseable integer (`build.rs:76-84`). In a mixed diff that last integer is the **deletion count**: on this very diff the gate now sees 1150 instead of the old 2781. A 200-insertion/10-deletion change auto-**skips** review where base ran it; `--review always|skip` are unaffected. Also dropped vs base: the auditable `- review gate: N changed lines, D dirs …` pfile line (`6048a28:…:1158`) and the root-file `"."` dir counting (`6048a28:…:1150-1154`), so a root file + one dir change no longer forces review. No test pins the auto-gate decision (no test references the gate). Fix: restore `--numstat` add+del summing (and either the gate log line or a documented new rule).

**F2 (Medium) — goal (5) is behavior-neutral vs base, and its test is not failing-first.**
The new pipeline reset (`phases/mod.rs:189-194`) runs immediately before `prompt_worker(…, resume=false, …)` — which already writes `.budget` and removes `.elapsed/.nudged/.grace/.autoext` at `worker/prompt.rs:665-677` (unchanged in this diff). Base reached the same function on every fresh prompt (`6048a28:…/mod.rs:844-857`; the surviving on-disk shim also `exec`s `quaqua worker prompt`), so no run sees different files than base. The cited `rem=max(900-962,1)=1s` starvation is computed in prompt_worker's **resume** branch (`prompt.rs:714-717` on `.budget/.elapsed`); the pipeline still keeps those files on resume (`phases/mod.rs:225`), so same-phase `--resume` starvation is untouched. `pipeline_per_phase_budget_reset_grants_fresh_allowance` (`tests/pipeline_prompt.rs:591-637`) asserts `.budget==2700`/`.grace` gone — the unchanged fresh branch produces those values on base too, so the test cannot fail on base (Change Policy: bug fix starts with a repro that fails on old code). Options: make the reset phase-change-aware on the resume path + add a repro that fails on base, or drop the change and record goal (5) as not-proven/not-needed.

**F3 (Medium) — telemetry `review` field regressed to always `skip`.**
Base set `state.review_run = "run"/"skip"` in the build arm (`6048a28:…:1128-1129,1163-1165`) and telemetry serializes it (`phases/mod.rs:110`). New `build.rs:86-108` keeps a local `run_review` bool and never assigns `state.review_run` (only writes left: init `mod.rs:463` + state save/load), so every reviewed pipeline now logs `"review":"skip"` in `runs/telemetry.jsonl`. Fix: set `state.review_run` before finishing.

**F4 (Low) — `--resume --rounds N` can now lower the cap (undocumented semantics change).**
`mod.rs:234-236` assigns unconditionally; base only raised (`6048a28:…:495-499`). `tests/pipeline_prompt.rs:502-521` covers raising only. Restore raise-only or pin the new semantics with a test + a note.

**F5 (Medium, fix-shaped change outside the brief) — build "Outcome: no changes" now ends the pipeline.**
New `check_impl` returns `Done` (`phases/mod.rs:332-334`) and `build.rs:43-45` returns immediately. Base's `finish(clean)` returned `Ok(())` and the build arm continued (`6048a28:…:955` → same arm `1120-1122`): with `auto` it double-finished (duplicate `- status:` + telemetry line), with `always` it spawned a reviewer on an empty diff. New behavior is saner, but it is a state-transition delta and untested. Ask: accept as a documented fix (+test) or restore.

**F6 (Low) — plan/spawn artifacts changed (state + log text) outside (4)+(5).**
`plan.rs:20-24`: `rename`→`copy`; the leftover `{bname}.md` is then archived as stale at build start (`phases/mod.rs:184-186`, extra `- archived stale report …` line); `state.planfile` is now a bare filename (base: absolute path, `6048a28:…:1063`); the `- plan: <file>` pfile line is gone (`6048a28:…:1064-1066`); plan-ready stderr (`plan.rs:47-50`) and the build prompt (`build.rs:15-18`) now show a bare name. `spawn.rs:82-98`: base_sha failure sentinel `"unknown"`/empty → `"?"` and the spawn log line rewritten. Cosmetic, but observable.

**F7 (Low) — test-file staleness / dead fixtures.**
`tests/pipeline_prompt.rs:7-11` still documents the removed `prompt-worker.sh` subprocess path; `resume(…, script_dir)` (`282-290`) and the `QUAQUA_SCRIPT_DIR`/fake-script plumbing (`422,461,486,508,559`) no longer select any engine script; pfx8's fake script (`545-554`) can no longer produce the "watchdog exit 6" it claims; the resend test's call expectation dropped 2→1 (`331-336`) because the failed pre-dispatch attempt is now unobservable — run-1's exit-code + `prompted=0` asserts (`302-313`) are the only proof it happened. `tests/worker_unit.rs:882-886` now asserts through ambient `load_team_config()` (reads `$HOME/quaqua/team.toml` when present) — non-hermetic.

**F8 (Low) — `has_real_blockers` false-negative class beyond the tested forms.**
`phases/mod.rs:397-415`: any line whose leading clause (before `. : ; — …` or `" - "`/`" — "`) is `none` is skipped, so a real blocker phrased `- none: <problem>` or `- None - <problem>` is missed. Tests pin only the informational forms (`worker_unit.rs:778-800`). Intentional-looking; recording the trade-off.

## Risks / notes
- No base-tree execution was possible (read-only review): F2/F5 base-behavior claims come from reading `6048a28` and the unchanged `worker/prompt.rs`.
- F1 is the one I would insist on before merge; the auto-gate decision has no test coverage.
- `planfile`/`review_run` have no consumers outside the pipeline (grep), so F3/F6 break nothing downstream today.

## Blockers
none

Verdict: 8 findings
