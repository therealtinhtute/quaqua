# Review: task w1 — round 1 (`quaqua/sude-w1` @ cfbda41, base e69c1eb)

Reviewed the diff `git diff e69c1eb14d8ef18d3290291f8d2138c739fda632` (11 files, +119/−117) read-only; no files outside `~/quaqua/runs` touched (worktree clean, branch `quaqua/suhuynh-w1r1`).

## Verification performed

1. Scope — `git diff --stat e69c1eb..cfbda41` contains exactly the 11 allowed files (util.rs new, lib.rs:19 `pub mod util;`, workspace.rs, grid/plan.rs, grid/reconcile.rs, grid/hook.rs, cap.rs, worker/prompt.rs, pipeline/mod.rs, lead/spawn.rs, merge/mod.rs). No scope creep.
2. Dead code — `git grep strip_tab_suffix HEAD` → 0 matches repo-wide; the plan.rs deletion left clean formatting.
3. Consolidation greps — `fn now_sec` → 1 (util.rs:5); `fn is_pid_alive` → 1 (util.rs:21); `fn find_quaqua_root` → 1 (workspace.rs:37); raw `kill -0` → only util.rs:25. The 3 `now_sec` copies were semantically identical (`.unwrap_or(0)` vs `.unwrap_or_default().as_secs()` both yield 0) — that move is safe.
4. `find_quaqua_root` move — extracted base lead/spawn.rs:34-79 vs new workspace.rs:37-82 → byte-identical body (diff empty). Callers resolve: merge/mod.rs:6,85; pipeline/mod.rs:101; lead/cleanup.rs:4,43 via the lead/spawn.rs:34 re-export; lead/spawn.rs:85.
5. Tests re-run by me (foreground, hermetic env):
   - `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **147 passed / 0 failed** (10 suites: 19 lib incl. `util::tests` ×3, 1 cli, 10 config, 8 gate, 50 grid, 5 init, 8 pipeline_prompt, 46 worker_unit, 0 doc-tests). ≥144/0 met.
   - `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked -p quaqua --test config --test worker_unit --test pipeline_prompt --test grid` → **114 passed / 0 failed**.
   - Builder's counts reproduced exactly.
6. `cargo fmt --check` → not clean; only the two deltas in finding 3 are attributable to this diff (other fmt diffs sit on untouched lines and pre-date the change; tests/run.sh has no fmt gate).
7. Missing tests — none blocking: `tests/grid.rs:441` exercises the `grid::reconcile::now_sec` re-export, the 3 new `util::tests` cover the consolidated functions; `grid::hook::is_pid_alive` keeps no direct test exactly as before.

## Findings

### 1. [Medium] `is_pid_alive` consolidation is not behavior-preserving; the mandated `## Challenge` is missing and the "zero behavior change" claims are false

The three originals were **not** byte-identical (the brief's audit premise does not hold):
- base `crates/quaqua/src/cap.rs:27-35` — no trim, `parse::<i32>()` + `pid > 0`, `.status()` (stderr visible to the caller);
- base `crates/quaqua/src/grid/hook.rs:45-54` — trim only, **no** integer validation, `.output()` (stderr hidden);
- base `crates/quaqua/src/worker/prompt.rs:387` — inline, no trim/validation, `.status()`.

The merged `crates/quaqua/src/util.rs:21-30` combines trim (hook) + `pid > 0` (cap) + `.output()` (hook), so it diverges on inputs where the originals disagreed:
- `"0"`: old hook / old prompt-inline ran `kill -0 0` → success → `true` (verified on this host: `/bin/kill -0 0` → exit 0); new → `false`.
- `"-1"`: same flip (verified `/bin/kill -0 -1` → exit 0 here); old returned `true`, new `false`.
- whitespace-padded numeric pid: old cap → `parse::<i32>` fails (Rust does not trim) → `false`; new → trims → may return `true`.
- stderr: old `cap.rs:155,224` and `worker/prompt.rs:376` used `.status()`, leaking `kill` error text for dead pids; new `.output()` suppresses it (an output change the brief's "giữ nguyên error message" forbids).

The brief (line 9) explicitly required this exact case to be surfaced as `## Challenge` with evidence ("bản copy khác nhau → ghi `## Challenge`... đừng đoán"); neither `sude-w1.plan.md` nor `sude-w1.build.md` contains one (`## Challenge` → 0/0), yet plan §2.2 quotes the differences before asserting "zero behavior change", and `sude-w1.build.md:56` claims "logic ... preserved line-by-line". The new tests `util.rs:42-48` pin the **new** semantics (`!is_pid_alive("0")`, `!is_pid_alive("-1")`) — assertions that fail against the old hook/prompt semantics, self-contradicting the zero-drift claim.

Practical reachability is near zero: all in-repo call sites (`cap.rs:155,224`, `grid/hook.rs:74`, `worker/prompt.rs:376`) receive machine-generated positive decimal pids (pane tokens, pipeline state, lock files), and the unified function is arguably safer.

Resolution (lead's call): either explicitly accept the hardened unification — write the Challenge and correct the plan/report claims — or restore per-site semantics behind the re-exports. As shipped, the "zero behavior change" constraint is unmet and the report's claims are inaccurate.

### 2. [Low] `workspace.rs:6-8` — module note became a doc comment on a private `use`

The "not to be confused with `find_quaqua_root`" note was moved from the module docs (`//!`) to `///` lines that attach to `use std::path::{Path, PathBuf};` (`workspace.rs:10`) instead of documenting the module, and duplicates the function's own doc (`workspace.rs:35-36`) — two homes for one fact (Change Policy §Docs). Fix: delete `workspace.rs:6-8` (the fn doc already covers discovery) or restore the `//!` form.

### 3. [Low, style-only] `cap.rs` — import order and a double blank line introduced by the diff

- `cap.rs:6-7`: `use crate::util::is_pid_alive;` placed after `use std::path::Path;`; repo-local order (cf. merge/mod.rs:3-6) and `cargo fmt --check` want the `crate::` group first.
- `cap.rs:25-26`: two consecutive blank lines left by the function removal (`cargo fmt --check` wants one).

The file was already fmt-dirty at base on untouched lines (e.g. `cap.rs:9-11`, 58, 102) and there is no fmt gate in `tests/run.sh`, hence style-only.

## Risks / notes

- Only finding 1 carries any behavioral risk, and its inputs are unreachable from the in-repo call sites; the real defect is the missing Challenge and the false zero-drift claims.
- Everything else in the wave verified sound: scope exact, dead code gone, `now_sec`/`find_quaqua_root` moves behavior-preserving, suites 147/0 and 114/0 reproduced.
- No `## Challenge` section in this review — findings only (pipeline §1 contract).

Verdict: 3 findings
