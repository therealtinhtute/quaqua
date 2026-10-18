# Review: task w1 — round 2 (`quaqua/sude-w1` @ 20cd4d2, base e69c1eb)

Read-only review of `git diff e69c1eb14d8ef18d3290291f8d2138c739fda632` (11 files, +116/−119 — exactly the brief's allowed file list, no scope creep). No files outside `~/quaqua/runs` touched. Tip under review: `20cd4d2` (fix round on top of build commit `cfbda41`).

## Round-1 findings — status

**1. [was Medium] `is_pid_alive` semantic divergence / missing Challenge — resolved via the mandated Challenge path; no open code defect.**
- The three originals were indeed not byte-identical (verified against base):
  - base `cap.rs:27-35`: no trim, `parse::<i32>` + `pid > 0`, `.status()`;
  - base `grid/hook.rs:45-54`: trim, no integer validation, `.output()`;
  - base `worker/prompt.rs:387`: inline, no trim/validation, `.status()`.
- `runs/sude-w1.fix1.md` §Challenge (lines 9–22) now records the divergence, the value flips on `"0"`/`"-1"` (verified on this host: `/bin/kill -0 0` → exit 0, `/bin/kill -0 -1` → exit 0), the stderr-visibility delta (`.status()` → `.output()`), and the hardening rationale. Its key claim — no reachable drift for valid inputs — holds: each call site's input source was read; all are machine-generated positive decimal pids (`cap.rs:154,223` from state-file lines; `grid/hook.rs:74` from the lock pid file written by `LockGuard` via `process::id()`; `worker/prompt.rs:376` from trimmed state-file lines).
- Disposition (accept hardened semantics vs restore per-site behavior behind the re-exports) is a Lead acceptance decision, properly teed up by the Challenge — not a defect remaining in the diff.

**2. [was Low] `workspace.rs` stray `///` doc attached to `use` — fixed.** `20cd4d2` removes former lines 6–8; module doc is now `workspace.rs:1-4` only, and the function carries its own doc at `workspace.rs:31-32` (one home per fact).

**3. [was Low] `cap.rs` import order + double blank line — fixed.** `cap.rs:4-7` now groups `crate::` imports before `std::path::Path`; single blank line before `contains_prefix_digit` (`cap.rs:25-26`). `cargo fmt --check` no longer flags those lines; the remaining cap.rs diffs (`:9-11,:27,:57,:101,:169`) sit on untouched lines and pre-date the change (same as base, shifted).

## Fresh verification (round 2, re-run by me)

1. Scope: `git diff --stat e69c1eb..HEAD` → the 11 allowed files exactly.
2. Moves:
   - `find_quaqua_root`: base `lead/spawn.rs` body vs new `workspace.rs:33` body extracted and diffed → empty (byte-identical).
   - `now_sec`: 3 copies semantically identical (`.unwrap_or(0)` vs `.unwrap_or_default().as_secs()` both yield 0 on failure); unified at `util.rs:5-18`.
3. Greps: `strip_tab_suffix` → 0 matches repo-wide; `fn now_sec` / `fn is_pid_alive` / `fn find_quaqua_root` → 1 each (`util.rs:5`, `util.rs:21`, `workspace.rs:33`); raw `kill -0` → only `util.rs:25`.
4. No orphaned imports after the refactor (`Command::` still ×3 in `worker/prompt.rs`; `PathBuf` ×2 in `lead/spawn.rs`; `cap.rs` `Path` used; `grid/reconcile.rs` has no stale `SystemTime`/`UNIX_EPOCH`).
5. Suites (foreground, hermetic `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin`, `--locked`):
   - `cargo test` → **147 passed / 0 failed** (10 suites; lib 19 incl. `util::tests` ×3) — ≥144/0 met;
   - `--test config --test worker_unit --test pipeline_prompt --test grid` → **114 passed / 0 failed** (10/46/8/50) — identical to builder counts.
   - Re-export paths still exercised: `tests/grid.rs:440-447` imports `grid::reconcile::now_sec` (passes).

## Findings

None open (0). Observations, non-blocking:
- `runs/sude-w1.build.md:56` ("preserved line-by-line") and `sude-w1.plan.md` §2.2 are superseded by the `fix1.md` Challenge; historical run reports left unedited as evidence snapshots.
- Challenge cites `cap.rs:155,224`; current lines are `cap.rs:154,223` (shift) — cosmetic.

## Risks

- The wave's only behavior delta is the documented `is_pid_alive` hardening, reachable only on non-positive/whitespace-padded pid strings that no in-repo caller produces; it is on record (`sude-w1.fix1.md` §Challenge) for the Lead's acceptance call.

Verdict: clean
