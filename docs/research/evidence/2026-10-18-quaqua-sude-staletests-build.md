# Run Report: Fix 5 Stale Test Suites & Close Pipeline Resume Wrinkles

## Summary
- Branch: `quaqua/sude-staletests` at commit `bd68969`
- All 5 stale test suites updated and passing cleanly with foreground targeted test runs under `target/debug/quaqua`.
- Both `pipeline --resume` wrinkles from `runs/pipeline-hooktab.md` resolved in `crates/quaqua/src/pipeline/mod.rs` and verified with new unit tests in `crates/quaqua/tests/pipeline_prompt.rs`.
- Full Rust suite passes: `cargo test --locked -p quaqua` (144 passed across 10 test binaries).

## Changes
- `tests/test_spawn_worker.sh`:
  - Updated `expect_reject_review` regex to match today's real error message: `error: --review requires a kind with can_review = true in team.toml (ni, suhuynh, truonglao), not $2`.
  - Scoped Claude read-only permissions loop to Claude kinds `ni` and `truonglao`.
  - Added dedicated test for `suhuynh` (`opencode` reviewer) verifying `--kind opencode` and `.srt.json` `"review": true`.
  - Updated Claude workspace-trust dialog test cases to use Claude kind `truonglao`.
- `tests/test_spawn_lead.sh`:
  - Updated happy-path assertion to reflect `strategy.lead_kind = "tieude"` (`opencode`), verifying `OPENCODE_CONFIG_CONTENT` passed in `workspace create` and `--standalone --auto` in `agent start`.
  - Replaced legacy `LEAD_MODEL`/`LEAD_EFFORT` override check with `LEAD_KIND=truonglao` override test.
  - Aligned herdr stub `agent get` response with `workspace create` pane/workspace tokens (`w7`, `%7`).
- `tests/test_pipeline.sh`:
  - Updated `reject "same harness"` test case to use `--builder tieude --reviewer suhuynh` (both `opencode`).
  - Updated `reject "non-reviewer kind"` pattern to assert `can_review = false in team.toml`.
  - Updated reviewer read-only spawn assertion to verify `--kind opencode` on default reviewer `suhuynh`.
- `crates/quaqua/src/pipeline/mod.rs`:
  - Added support for `--rounds <N>` (and `--rounds=<N>`) in `pipeline.sh --resume <task> <repo> [--rounds N]`; validates integer `>= 1` and raises `state.rounds` when `N > state.rounds`.
  - Updated USAGE header to reflect optional `[--rounds 2]`.
  - In `phase == "review"`, checks if reviewer report is already delivered on disk with `Verdict:` or `## Challenge`; bypasses `wait_file` to prevent watchdog exit 6 against a post-cleanup missing `srt.json`.
- `crates/quaqua/tests/pipeline_prompt.rs`:
  - Added `pipeline_resume_with_rounds_raises_cap`: verifies `--rounds 5` raises `state.rounds` from 2 to 5.
  - Added `pipeline_resume_at_phase_review_with_delivered_report_avoids_exit_6`: verifies resuming an existing reviewer report post-cleanup bypasses `wait_file` and avoids exit 6.

## Verification & Test Results

### 1. `tests/test_spawn_worker.sh`
```bash
QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_spawn_worker.sh
```
Output:
```
ok: no kind prefix -> exit 1, no git/herdr
ok: underscore -> exit 1, no git/herdr
ok: kind mismatch -> exit 1, no git/herdr
ok: uppercase -> exit 1, no git/herdr
ok: 21 chars -> exit 1, no git/herdr
ok: -lead suffix (reserved for project Leads) -> exit 1, no git/herdr
ok: task ending in -lead -> exit 1, no git/herdr
ok: accented name (C) -> exit 1, no git/herdr
ok: accented name (en_US.UTF-8) -> exit 1, no git/herdr
ok: 17 chars -> accepted, git+herdr called
ok: 20 chars (boundary) -> accepted, git+herdr called
ok: leadname suffix is not -lead -> accepted, git+herdr called
ok: tieude -> --kind opencode, model opencode-go/deepseek-v4.1-flash#max
ok: sude -> --kind omp, google-antigravity/gemini-3.8-flash high, rules appended
ok: detu -> --kind omp, google-antigravity/gemini-3.8-flash high, rules appended
ok: fallback -> omp failed, opencode (tieude) started, WARN + srt fallback recorded
ok: one-hop -> fallback start failure errors, no third attempt
ok: no fallback configured -> start failure errors, no fallback taken
ok: grid check -> called after a successful spawn, group recorded
ok: GRID_TAB -> the check is scoped to that tab, group recorded; timeout exits non-zero
ok: 2 unarranged workers + GRID_TAB -> both arranged into GRID_TAB with no manual reconcile
ok: GRID_SKIP=1 under tests/pipeline -> grid not checked
ok: bare GRID_SKIP=1 -> ignored with a warning, grid checked
ok: opencode with project MCP config -> no dialog wait
ok: opencode without project MCP config -> no dialog wait
ok: grid failure -> non-zero exit, spawn fails
ok: pane tokens -> grid_role=worker/grid_group/grid_owner on split, grid_ready=1 after agent start
ok: pane tokens -> grid_owner reflects QUAQUA_OWNER, lead owner -> <slug>-workers grid tab
ok: --review on tieude -> exit 1, no git/herdr
ok: --review on sude -> exit 1, no git/herdr
ok: --review ni -> dontAsk + Edit() rules (runs writable, worktree denied) + narrow Bash allows (cargo test, atomic mv), model/effort unchanged
ok: --review truonglao -> dontAsk + Edit() rules (runs writable, worktree denied) + narrow Bash allows (cargo test, atomic mv), model/effort unchanged
ok: --review suhuynh -> opencode start, review: true recorded in srt.json
ok: plain ni (no --review) -> ambient settings, no review mode/Bash rules
ok: claude trust dialog -> wait-output trust, Down+Enter before grid_ready, boundary untouched
ok: no trust dialog -> waited, nothing sent
ok: blocked claude start -> trust wait, idle wait, then grid_ready=1
ok: stuck trust dialog -> exit 1 with a clear error, pane closed
ok: --base -> rev-parse --verify, worktree add starts at quaqua/tieude-m12bt
ok: no --base -> worktree add <wt> only (HEAD), no rev-parse
ok: same-name retry -> kept branch reused, no -b
ok: another branch exists -> fresh -b still used
ok: same-name retry + --base -> branch reused, --base reported as ignored
ok: --base badref -> rejected before worktree/pane
ok: --base= empty -> exit 1, no git/herdr
ok: trailing --base w/o value -> exit 1, no git/herdr
ok: fresh clone -> runs/ created before the rules dump, spawn succeeds
ok: push block -> worktreeConfig + per-remote pushurl clear+block on spawn
ok: core.bare=true in common config -> spawn refused before any pane
ok: core.worktree in common config -> spawn refused before any pane
ok: zero remotes -> no pushurl written, spawn succeeds
ok: respawn -> surviving worktree reused, agent start retried
ok: foreign dir at worktree path -> refused before any agent start
ok: real git -> retry after cleanup reuses the kept branch (marker commit kept), spawn succeeds
ok: real git -> worktree push fails with a clear error, main pushurl unchanged
ok: real git -> inherited shared pushurl cleared, push refused, target empty
ok: real git -> core.bare/core.worktree refused, config untouched, repo usable
PASS
```

### 2. `tests/test_r3_shims.sh test_spawn_worker.sh`
```bash
QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_r3_shims.sh test_spawn_worker.sh
```
Output:
```
=== Running test_spawn_worker.sh ===
...
PASS
ALL TARGET SUITES PASSED
```

### 3. `tests/test_spawn_lead.sh`
```bash
QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_spawn_lead.sh
```
Output:
```
ok: herdr-grid -> Lead pane tagged as orchestrator (role=orchestrator, owner=coucou-island-lead), no grid_ready
ok: brief -> redirects reach the Lead through the CoS only
ok: happy path -> get/workspace/start/prompt order, claude args, rules file, owner updated
ok: duplicate -> refused after agent get, no workspace/start, INDEX untouched
ok: uppercase slug -> exit 1, no herdr/git
ok: underscore slug -> exit 1, no herdr/git
ok: --flag arg -> exit 1, no herdr/git
ok: single-dash slug -> exit 1, no herdr/git
ok: unknown slug -> exit 1, no herdr/git
ok: row but no card -> exit 1, no herdr/git
ok: lead name >32 chars (registered slug) -> exit 1, no herdr/git
ok: 32-char lead name (27-char slug) accepted
ok: 'mono' updates only its own row (mono-desk untouched)
ok: LEAD_KIND overrides strategy.lead_kind
ok: missing quaqua/<slug>-integ -> created from main before the workspace
ok: master-only repo -> integration branch created from master
ok: origin/HEAD set -> integration branch created from develop
ok: origin/HEAD without a local ref -> integration branch created from origin/develop
ok: origin/HEAD stale -> integration branch created from main
ok: origin/HEAD set -> integration branch created from it (precedence over main)
ok: no default branch in the project repo -> refused before side effects
ok: stale owner cell -> respawn allowed (agent registry is the truth)
ok: INDEX '~/repo' expands to a literal HOME (no patsub_replacement corruption)
ok: concurrent spawn vs cleanup on different rows -> both updates kept
PASS
```

### 4. `tests/test_r4_shims.sh test_spawn_lead.sh`
```bash
QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_r4_shims.sh test_spawn_lead.sh
```
Output:
```
=== Running test_spawn_lead.sh ===
...
PASS
ALL TARGET SUITES PASSED
```

### 5. `tests/test_pipeline.sh`
```bash
QUAQUA_BIN="$(pwd)/target/debug/quaqua" bash tests/test_pipeline.sh
```
Output:
```
ok: no args -> exit 1 (^# Usage:)
ok: bad task chars -> exit 1 (^# Usage:)
ok: missing --brief -> exit 1 (brief.*required)
ok: missing brief file -> exit 1 (brief.*required)
ok: brief lacks headers -> exit 1 (Goal.*Hard constraints.*Current design)
ok: same harness -> exit 1 (different harnesses)
ok: same kind -> exit 1 (different harnesses)
ok: non-reviewer kind -> exit 1 (can_review = false in team.toml)
ok: bad --review -> exit 1 (auto|always|skip)
ok: --rounds 0 -> exit 1 (integer >= 1)
ok: builder name >20 -> exit 1 (exceeds 20 chars)
ok: reviewer name >20 -> exit 1 (exceeds 20 chars)
ok: resume w/o state -> exit 1 (no pipeline state)
ok: plan-ready -> exit 5, plan parked, state + telemetry written
ok: duplicate slug -> exit 1, --resume hinted
ok: clean -> review ran on builder branch (read-only), cleaned, telemetry clean
ok: auto gate under threshold -> review skipped, no reviewer spawned
ok: rounds exhausted -> blocked + Blockers, 2 fresh reviewers, telemetry
ok: disputed -> Challenge copied into pipeline file, stops before round 2
ok: watchdog exit 2 -> propagated, pipeline blocked, telemetry written
ok: missing Verdict -> watchdog exit 6, pipeline blocked, no cleanup
ok: --base -> builder worktree starts at quaqua/integ
ok: builder spawn -> in-process grid check after the spawn, converged
ok: resume -> in-process grid checks for the build + review phases
ok: GRID_TAB overrides the workers tab label
ok: mis-arranged owner population repaired into GRID_TAB before the prompt
ok: without GRID_TAB the repair labels the created tab <repo>-workers
ok: grid failure pauses pre-prompt; resume after arranging continues
ok: reviewer-spawn grid failure pauses pre-prompt; resume completes review
ok: state carries lead/lead_pane; --resume refreshes pipeline_pid
ok: blocked plan report -> blocked, evidence copied, no build prompt
ok: Outcome: BLOCKED build (nudge-style report) -> blocked, no reviewer
ok: blocked fix report -> blocked, no next review round
ok: Blockers 'none'/'None' -> not blocked, plan advances to plan-ready
ok: zero commits after build -> resumable blocked, no reviewer
ok: explicit 'Outcome: no changes' -> clean, review skipped
ok: dirty worktree (tracked + untracked) -> blocked, evidence logged
ok: fix without a new commit -> resumable blocked (no-op fix is a Challenge)
ok: reviewer Challenge + clean verdict -> disputed, copied, reviewer cleaned
ok: reviewer Challenge + findings verdict -> still disputed
ok: phase budget clamped to pipeline remainder (20s of 2700s)
ok: file landing past the total budget -> watchdog 4 + blocked, never clean
ok: stale report archived before the fresh prompt
ok: resume archives a report older than the recorded prompt time
ok: resume consumes a fresh post-prompt report without archiving or re-prompting
ok: telemetry -> every stop line is valid jsonl with all lock §2 fields
ok: real git — reviewer spawned on the exact committed tip (gate SHA == base)
ok: real git — dirty worktree blocked (untracked file kept off the branch)
ok: real git — zero commits after build is blocked
ok: real git — declared no-op finishes clean, no reviewer spawned
ok: failed agent start -> resume reuses worktree, retries start, prompts plan
ok: reviewer spawn failure -> resume reuses worktree, reviews, finishes clean
PASS
```

### 6. `cargo test --locked -p quaqua`
```bash
cargo test --locked -p quaqua
```
Output:
```
test result: ok. 16 passed; 0 failed (src/lib.rs)
test result: ok. 0 passed; 0 failed (src/main.rs)
test result: ok. 1 passed; 0 failed (tests/cli.rs)
test result: ok. 10 passed; 0 failed (tests/config.rs)
test result: ok. 8 passed; 0 failed (tests/gate.rs)
test result: ok. 50 passed; 0 failed (tests/grid.rs)
test result: ok. 5 passed; 0 failed (tests/init.rs)
test result: ok. 8 passed; 0 failed (tests/pipeline_prompt.rs)
test result: ok. 4 passed; 0 failed (tests/worker_cap.rs)
test result: ok. 42 passed; 0 failed (tests/worker_unit.rs)
Total: 144 passed; 0 failed; 0 ignored
```

## Risks
- None identified. All changes strictly preserve existing error semantics and contract invariants while updating stale assertions to match current team configuration and harness routing.

## Lessons
- none
