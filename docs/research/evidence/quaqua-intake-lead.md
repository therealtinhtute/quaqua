# quaqua-intake-lead — r2 report

## Merged into quaqua/quaqua-intake-integ (tip c6a2c39)
| task | builder → reviewer | sha | failing-first | test cmd → result (Lead re-run) |
|---|---|---|---|---|
| capfix | tieude → review skipped (auto gate, 101 lines) | d64fb3a | old: `5 passed; 2 failed` (left 3/4, right 2) | `cargo test --locked -p quaqua cap` → 7 passed (worker_unit) |
| blkfix (1) | tieude → review skipped (auto gate, 38 lines) | 509a4db | `None…` assert red at worker_unit.rs:574; `!pipeline_finished("blocked",0)` red | `cargo test --locked -p quaqua pipeline` → 8 passed |
| escstop (2) | tieude → suhuynh, 2 rounds, r2 clean | c6a2c39 | tests red before stop.rs (build.md) | `cargo test --locked -p quaqua` → all pass except config_invalid_fixtures_are_rejected (pre-existing, see defects) |

- capfix root cause: Rust `PipelineState::save` (pipeline/mod.rs:259-289) never writes `task=`, so cap.rs:143 got `p_task=""`, the live builder went unattributed → 1 + 2 = 3. Fix: cap.rs derives task from the `pipeline-<task>.state` filename when `task=` is missing. Not `--resume`.
- blkfix: trailing `.`/`…` trimmed before the `none` match (non-empty guard; "None of the tests pass" stays a blocker); new `pipeline_finished(final_st, code)` = `code==0 && st!="blocked"`. Proven live: the blocked escstop pipeline resumed.
- escstop: new `quaqua worker stop <name>` (worker/stop.rs). Harness comes from `herdr agent get` → opencode `esc esc`, omp `esc`, never ctrl+c. **Lead decision B:** no claude mapping, so claude and unknown harnesses fail closed and send zero keys. Esc for claude is not verified.
- Test lines added/removed: capfix +90/−0, blkfix ≈+17/−0, escstop +186/−0.

## Cleanup (done first)
sude-blockedfix: pane closed (`herdr pane close wQ:p2`; this herdr has no `agent stop`), worktree removed, branch kept, uncommitted test diff saved to runs/sude-blockedfix.testdiff.patch (reused by blkfix). pipeline-blockedfix.state trashed after confirming pipeline_pid 16447 dead.

## Defects (r1) — found
2/4. **Not a code bug.** A zsh function `quaqua()` in anh iu's shell profile (in snapshot ~/.claude/shell-snapshots/snapshot-zsh-*.sh:7652) shadows ~/.cargo/bin/quaqua. In a herdr pane (HERDR_ENV=1) it runs `cd ~/quaqua; claude --model claude-opus-5-5 --effort low "$@"`, which explains the "Invalid API key" output and why `quaqua worker cleanup`/`merge`/`--check-brief` did nothing. Calling the binary by path works. AGENTS.md's `quaqua …` commands therefore break in any agent shell that loads the profile.
3. Pane placement: the pipeline grids into the Lead tab (pipeline/mod.rs:662 GRID_TAB default + :702 reconcile); every builder landed in wQ:t1 (p2–p7) despite GRID_TAB=quaqua-intake-workers being exported. Not root-caused yet.
- New: the installed ~/.cargo/bin/quaqua predates capfix, so pipelines still count 3 slots. escstop could only run after blkfix, not in parallel. Rebuilding/installing it is shared tooling → CoS call.
- New: `config_invalid_fixtures_are_rejected` (crates/quaqua/tests/config.rs:230) already fails on integ 509a4db, before escstop. Touches config.rs (spawneff territory) → quaqua-rs-lead.
- New: pipeline reviewer names `suhuynh-<task>r<N>` must be ≤20 chars, so the task slug must be ≤10. `blockedfix2` was rejected, renamed `blkfix`.
- New: an opencode web-search permission dialog blocked tieude-escstop at start (watchdog exit 5); I chose "Disable web search".

## Not done
- (3) targeted-tests: not started. Proposal below (CoS writes rules).
- (4) p0-fifth-worker: waiting on spawneff (sude-spawneff2 still live in quaqua-rs-lead's team).
- Per-step Lead watchdog: design proposal below, nothing built.

## Proposed rule text (CoS to write)
- rules/_all/worker-timebox.md: "Builders run only the targeted tests named in the brief (`cargo test --locked -p quaqua <filter>`), in the foreground; `bash tests/run.sh --slow` runs once, at the Lead's gate. Never wait on a process with a `pgrep -f` loop: `pgrep -f` matches its own and the caller's command lines (seen: the Lead's prompt text matched `blockedfix`)."
- rules/_all/brief.md, Hard constraints template line: "Targeted tests only (`<cmd> <filter>`), foreground, no pgrep -f wait loops; --slow is the Lead's gate."
- rules/_all/worker-interrupt.md: "Stop a busy worker with `quaqua worker stop <name>` (opencode esc esc, omp esc); claude workers have no verified sequence and are refused."
- AGENTS.md Protocol: "In agent shells call `~/.cargo/bin/quaqua` (or `command quaqua`); the interactive `quaqua` shell function launches Claude."

## Per-step Lead watchdog (proposal)
Each pipeline phase wait (plan, build, review rN, fix N) is a fresh `prompt-worker.sh` wait with its own brief budget and its own 80% nudge. The absolute cap (PROMPT_WORKER_MAX_SEC) is measured from that phase's prompt_ts, not from the first prompt. Fit with `--resume`: resume continues the current phase's allowance, as it does today. A new phase writes a new `runs/.<name>.budget` and resets `.grace`. That's a small change in pipeline/mod.rs wait_file (~l.831), where prompt_ts is already per phase. Evidence: r1 sude-blockedfix spent 78 min in one phase. Every r2 phase finished within 9 min.

## Risks
- Review was skipped on capfix and blkfix by the auto size gate; I reviewed both diffs myself and re-ran their tests.
- Integ isn't green on the full crate because of the pre-existing config test above.

## Lessons
- In agent shells, call tool binaries by absolute path: a user-profile shell function with the same name silently replaces them.
- A state file is a contract only if its writer and reader are tested together; a hand-written fixture hid the missing `task=`.

Verdict: done
