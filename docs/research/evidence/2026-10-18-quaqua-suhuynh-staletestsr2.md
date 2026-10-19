# Review — task `staletests` (pipeline REVIEW round 2 of 2)

Diff under review: `git diff de2094995a2640e7d53cea75c1ecea3b2c98b1a5` on `quaqua/sude-staletests` tip `a708fcd` → 5 files, +183/−37:
`crates/quaqua/src/pipeline/mod.rs`, `crates/quaqua/tests/pipeline_prompt.rs`, `tests/test_pipeline.sh`, `tests/test_spawn_lead.sh`, `tests/test_spawn_worker.sh`.
`git diff --name-status`: 5 × M, no added files; every touched path is on the brief's allowed list; `test_r3_shims.sh`/`test_r4_shims.sh` unchanged because they delegate.

## Findings

None.

Round 1's single finding (the wrinkle-(ii) regression test could not fail on the buggy code) is resolved — proof below.

### Round-1 finding 1 verified closed — the test is red-capable

- On tip: `cargo test --locked -p quaqua --test pipeline_prompt` → 8 passed, incl. `pipeline_resume_at_phase_review_with_delivered_report_avoids_exit_6`.
- Variant build (throwaway copy of `crates/quaqua` + `team.toml` in the session temp dir) with only the bypass reverted (`crates/quaqua/src/pipeline/mod.rs:1267-1269` made unconditional): the same test **FAILS** — `left: Some(6), right: Some(0)` at `crates/quaqua/tests/pipeline_prompt.rs:539`.
- The test cannot pass for an unrelated reason: it neutralizes the grid hook via `GRID_SKIP=1 QUAQUA_TEST=1` (`pipeline_prompt.rs:531-533`, honored at `mod.rs:719-722`); the fake `prompt-worker.sh` exits 6 only on `--resume` (`pipeline_prompt.rs:519-526`) and the wait path propagates that as pipeline exit 6 (`mod.rs:908-921`). On the fixed path the fake is never invoked, and `last_findings=2` (`:545-549`), `rounds_run=1` (`:550-554`) plus the `- review round 1: Verdict: 2 findings` line (`:555-559`) prove the delivered report was consumed.
- Round-1 notes 1 and 3 are also fixed: `tests/test_spawn_lead.sh:6` now names LEAD_KIND; `mod.rs:1264` matches the `verdict:` prefix case-insensitively.

### Wrinkle (i) red-capability (verified from the diff)

`pipeline_resume_with_rounds_raises_cap` (`crates/quaqua/tests/pipeline_prompt.rs:473-493`) runs `--resume pfx7 <repo> --rounds 5`. The base revision accepted only exactly 3 args (`if args.len() != 3` in diff hunk `@@ -441,18 +441,50 @@`), so on old code the resume exits 1 and `state.rounds` stays 2 — the assert at `:488-492` fails. The raise-only merge (`mod.rs:507-511`) implements "resuming with `--rounds N` should raise the cap"; the raised cap is consumed unchanged at `mod.rs:1349`.

## Verification runs (foreground, tip a708fcd)

| Command | Result |
|---|---|
| `cargo test --locked -p quaqua --test pipeline_prompt` | 8 passed / 0 failed |
| `cargo test --locked -p quaqua` | 144 passed / 0 failed (10 targets) |
| `QUAQUA_BIN=$PWD/target/debug/quaqua bash tests/test_spawn_worker.sh` | PASS |
| `QUAQUA_BIN=$PWD/target/debug/quaqua bash tests/test_spawn_lead.sh` | PASS |
| `QUAQUA_BIN=$PWD/target/debug/quaqua bash tests/test_pipeline.sh` | PASS |
| `bash tests/test_r3_shims.sh test_spawn_worker.sh` | PASS · ALL TARGET SUITES PASSED |
| `bash tests/test_r4_shims.sh test_spawn_lead.sh` | PASS · ALL TARGET SUITES PASSED |
| Variant build with the bypass reverted → same Rust test | FAILED, exit 6 (proves the test guards the fix) |

Assertions match today's real behavior, none weakened:
- `--review` refusal text == `crates/quaqua/src/worker/spawn.rs:349-353`; `--review` on `suhuynh` really starts opencode (`spawn.rs:201-215`) and records `"review":true` in srt.json (`spawn.rs:664-674`).
- tieude/suhuynh is same-harness today (both `harness = "opencode"` in `team.toml`); non-reviewer `sude` matches `pipeline/mod.rs:624`; `LEAD_KIND=truonglao` claude args match `spawn.rs:226-254` + team.toml (`opus-5.5` → `claude-opus-5-5`, effort high).
- The stale reviewer expectation is replaced by the real launch (`test_pipeline.sh:213`); the claude read-only profile is still asserted for claude kinds (`test_spawn_worker.sh:408-436`).

## Notes (not counted as findings)

1. Default reviewer `suhuynh` is opencode and gets no enforced read-only boundary — `spawn.rs:201-215` ignores the `review` flag for opencode; only claude kinds get `dontAsk` + Edit rules (`spawn.rs:239-251`). Pre-existing since dc3368d; this diff correctly pins today's behavior, but `tests/test_pipeline.sh:208,223` still say "read-only" in comment/echo. Lead decision, not a defect of this diff.
2. Cosmetic stale text: `tests/test_spawn_lead.sh:131` echo says "claude args" while the happy path asserts opencode; `tests/test_r4_shims.sh:47` generated shim header still shows `pipeline.sh --resume <task> <repo>` without `[--rounds 2]` (the real USAGE, `mod.rs:16`, was updated).
3. Resume-path `--rounds` error branches (`--rounds 0`, `--rounds=abc`, trailing `--rounds`) have no targeted test; the fresh path has one (`test_pipeline.sh` "reject --rounds 0"). Minor coverage gap.
4. `mod.rs:1264` predicate is case-broader than the verdict parser (`mod.rs:1300-1306`): a report whose only verdict line is e.g. `VERDICT: …` is consumed then reported blocked (`:1321-1323`, exit 0, Lead decides) instead of re-waited — safe, no route back to exit 6.

## Blockers / limits

- `bash tests/run.sh --slow` (25/0/3 acceptance gate) and the full r3/r4 umbrellas were not run — the former is the Lead's gate; the previously-stale r3/r4 delegations were run targeted, matching the builder's evidence.
- Verification built `target/` in the review worktree and a throwaway variant copy in the session temp dir (build artifacts only; no repo, source, or report file outside `~/quaqua/runs` was modified).

Verdict: clean
