# Run Summary: suhuynh-bashcleanr1 — REVIEW round 1/2

Scope: diff `a708fcd` → `a35a54d` (branch `quaqua/sude-bashclean`, 36 files, +112/−399) vs brief `bashclean-brief.md`. Read-only review; every claim below cites the diff or a re-checked source.

## Findings (defects in the diff)

None. The change meets the brief's three testable axes:

1. **Correctness** — deleted set is exactly the 9 wrappers + `scripts/lib/common.sh` (diff `--name-status`: `scripts/lead/*.sh`, `scripts/worker/*.sh`, `scripts/workspace/init-workspace.sh`, `scripts/lib/common.sh`); common.sh was unused post-change (only src doc comments + the migration-plan doc name it, both constraint-frozen — see O1). `scripts/lib/common.sh` deletion is within the brief's conditional allowance.
2. **No dangling callers** — `git grep -E 'scripts/(worker|lead|workspace|lib)/'` after the diff matches only (a) `crates/quaqua/src/*.rs` doc comments/messages, (b) `scripts/hooks/pre-commit:27`, `scripts/hooks/herdr-gate.sh:34`, `scripts/auditor/auditor.sh:153`, (c) `docs/research/herdr-grid-rust-migration-plan.md` — all in the brief's explicitly-frozen sets except the research doc, which is pinned as historical by `tests/test_config.sh:98-104` (rule→key table rows; updating it would break that test). `tests/test_docs.sh:62` excludes exactly these frozen prefixes from the dangling-ref scan; the new scan pass is verified live (`ok: every scripts/*.sh reference resolves`).
3. **Runtime safety of deletion** — the binary's five `find_engine_script` call sites all have in-process fallbacks, so no caller needs the deleted scripts: `crates/quaqua/src/pipeline/mod.rs:836→859` and `:883→896` (prompt), `:1014→1030` and `:1206→1231` (spawn), `:1275+` (cleanup). `quaqua init` root fallback confirmed at `crates/quaqua/src/init.rs:53-75`.

Supporting verifications:

- `crates/quaqua/src/lead/spawn.rs` change is the single prompt-text hunk only (line 409-412) — the brief's allowed src exception; no other src edit.
- `tests/run.sh:31,33` slow_cover now points at real paths (`crates/quaqua/src/worker/prompt.rs`, `crates/quaqua/src/pipeline/`) — both exist; `tests/run.sh:66-67` still exports PATH+`QUAQUA_BIN`, so `${QUAQUA_BIN:-quaqua}` in all rewired suites resolves under the runner.
- `tests/test_docs.sh:69` new target `crates/quaqua/src/pipeline/mod.rs` contains `Verdict: clean` (mod.rs:1257) ✓.
- `crates/quaqua/tests/gate.rs:67-77` allow-cases updated to the `quaqua` spellings (was the deleted script paths) ✓.
- `QUAQUA_ROOT` added in `tests/test_spawn_lead.sh:79,276` and `tests/test_cleanup_lead.sh:58,165` is honored by `find_quaqua_root` (`crates/quaqua/src/lead/spawn.rs:35`; `lead/cleanup.rs:43` uses the same resolver) — the right replacement for the wrapper-exported `QUAQUA_SCRIPT_DIR` ✓.
- Shim suites retired as permitted ("retire kèm lý do"): `tests/test_r3_shims.sh`, `test_r4_shims.sh`, `test_r5_shims.sh` are now self-skip stubs naming replacements; reasons per suite are in the builder report §5; the unique binary-gate coverage moved to `crates/quaqua/tests/gate.rs` (updated) and direct suites. No suite left red; self-skip is counted as SKIP by `tests/run.sh:89-91`.
- `tests/test_pipeline.sh:313-334` fabricates a one-line `spawn-worker.sh` in a sandbox shim root to keep exercising the binary's legacy shell branch; it no longer recreates repo wrappers — sandbox-only fixture, no scope creep. `tests/test_grid_workers.sh:93` unconditional SKIP is required because the file it probed is deleted (it self-skipped at baseline too; dead code below predates this diff — O2).

## Observations (no fix required under this brief)

- O1 — Stale user-facing refs remain in constraint-frozen files: `scripts/hooks/herdr-gate.sh:34` (fallback message lists the 5 deleted scripts), `scripts/hooks/pre-commit:27`, `scripts/auditor/auditor.sh:153`, and src messages `crates/quaqua/src/pipeline/mod.rs:1037,1238` (`pipeline.sh --resume` hints). The brief froze these files ("KHÔNG đụng"), so the builder correctly left them; worth a follow-up task for the Lead since gh-34 is what a user sees when the binary is missing.
- O2 — `tests/test_grid_workers.sh:16-22,93` now skips unconditionally with ~1,000 lines of unreachable code below; pre-existing shape, candidates for a cleanup task.
- O3 — `tests/test_init_workspace.sh:85,248,271,300,310` changed `cd /` → `cd "$R"`: the "foreign cwd" property of the old bash entrypoint is no longer exercised (the binary resolves root from cwd/`QUAQUA_SCRIPT_DIR`). Justified by design, but a real semantic difference vs the deleted script.
- O4 — Builder report inventory nits (outside diff scope): per-file diffstat rows in `runs/sude-bashclean.build.md:39-48` don't match `git show --numstat a35a54d` for 6 test files (e.g. r4 91 vs 106, spawn_lead 14 vs 16); totals match (36 files, +112/−399). The per-suite list there also omits `test_taskforce_cap.sh`, so the case count sums to 27/28.
- O5 — Stale comments left behind: `tests/run.sh:25-27` ("drives prompt-worker.sh… pipeline.sh"), `tests/test_spawn_worker.sh:2` ("spawn-worker.sh shim").

## Residual risk (not verified by this reviewer)

- `env -u QUAQUA_OWNER -u GRID_TAB bash tests/run.sh --slow` was **not re-run** here (time budget); the Lead's gate must run it before acceptance. Expected vs baseline 23/2/3: ≈ 20 pass / 2 fail (auditor, out of scope) / 6 skip (r3,r4,r5 now self-skip; plus grid_workers, herdr_grid_apply, herdr_grid_hook as at baseline). If the count lands at 19 pass, `test_taskforce_cap.sh` (unlisted in the report, O4) is the case to inspect.

## Blockers
none

Verdict: clean
