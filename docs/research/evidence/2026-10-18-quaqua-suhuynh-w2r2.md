# REVIEW w2 — round 2: verify round-1 fixes on `22e6a76` (json.rs removal + state.rs normalization + cleanup fix)

Branch `quaqua/sude-w2` @ `22e6a76`, diff vs `20cd4d2` (11 files, +687/−570), reviewed as instructed: full diff read at the branch tip; every round-1 finding re-checked against the fix commit; parity of `cap.rs`/`state.rs` against base semantics; full suite re-run in this worktree; greps at `22e6a76`.

## Findings

None. All four round-1 findings are fixed and verified; the fix commit introduced no new defect found by this review.

## Round-1 findings — resolution verified

**F1 (dead bindings / warnings) — fixed.** The fix commit deleted the 9 unused path bindings; `prompt.rs:586-591` now binds only `out`, `submit_log` (used at `:682`), `srt` (used at `:958-959`). Independent evidence: full-suite compile at `22e6a76` produced **zero warnings** (log scanned, none).

**F2 (`advisor-` prefix) — fixed, matches writers.** `state.rs:277-279` recognizes only `pipeline-`/`taskforce-`/`dual-lens-` (`.state`) — exact base parity. Writer check holds: `taskforce/mod.rs:186` is the only `taskforce-*.state` writer (`kind=dual-lens` at `:189`, `finished=1` at `:226,:231,:265`), and `run_advisor` (`taskforce/mod.rs:117-164`) writes no state file. Pinned by test `worker_unit.rs:2073-2076` (`advisor-test.state` → `None`).

**F3 (`finished` parity) — fixed.** `state.rs:285` derives `is_running` from the literal line `finished=0` (`any(|l| l.trim() == "finished=0")`), `:288-291`/`:313-316` set `finished: !is_running`; `cap.rs:99-102` skips finished states. That reproduces the base gate exactly (file without `finished=0` counts as finished). Pinned by test `worker_unit.rs:2061-2065`.

**F4 (raw writes / duplicated layout in prompt.rs) — fixed for the flagged files.** All four watchdog state files now go through `WorkerMetadata` read/write pairs: `prompt.rs:640,641,645,648,653,654` (elapsed/budget/autoext), `:699` (elapsed), `:729-730` (nudged/grace), `:739,847` (progress), `:936` (nudged), plus `:590-591,766,771` (cost sentinels), `:791,794` (provider_retry), `:865,:883` (grace/autoext). Same writers in `spawn.rs:497,498,500,675`, `worker/cleanup.rs:155-157,163-166,177`, `lead/cleanup.rs:92-94`. Names in `state.rs:210-226` match byte-for-byte.

## Verified claims (evidence)

- **json.rs fully removed.** File absent; `pub mod json` gone from `lib.rs`; `grep -rn "parse_json|crate::json|mod json|JsonValue" crates/quaqua/src crates/quaqua/tests scripts` → 0 matches; no md/doc references.
- **Suite (re-run by me at `22e6a76`):** `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **148 passed, 0 failed** (18 lib + 1 cli + 10 config + 8 gate + 50 grid + 5 init + 8 pipeline_prompt + 48 worker_unit). ≥147 required. Baseline 147 → +2 new tests (`worker_unit.rs:1936`, `:2010`), −1 deleted `json.rs` unit test.
- **Scope:** diff touches exactly the 10 allowed files + `tests/worker_unit.rs`. No scope creep.
- **Format compatibility:** all paths unchanged vs base inline names (`state.rs:26-80`); payload bytes identical (`{value}\n`; srt raw JSON; owner/grid `{x}\n`). State files are only *read* (`state.rs:274-331`), never rewritten — key=value layout untouched, old readers unaffected.
- **Item (c):** `worker/cleanup.rs:177` → `state.rs:210-226` deletes `.budget`, `.grace`, `.cost80`, `.cost100` (the brief's four) plus `.progress`, `.elapsed`, `.nudged`, `.submit.log`, on top of the base set; test `worker_unit.rs:1936-2008` creates and asserts removal of all 14 per-worker files.
- **cap.rs parity:** line-by-line vs base — owner default `cap.rs:77`, aliveness `:110-119`/`:155-173`, weights `:83-92`/`:145-153`, standalone accounting `:198-202`. Pre-existing cap tests pass (`test_cap_pipeline_state_task_line_wins_over_filename`, `test_cap_pipeline_rust_state_without_task_line`, `test_cap_pipeline_rust_state_review_round`, `test_cap_pipeline_slot_reservation_mock`, `test_cap_pipeline_double_count_repro`, `test_taskforce_dual_lens_cap_reservation`).

## Residual risks (not findings)

- serde_json is stricter than the old hand parser (trailing garbage after the value); every call site falls back (`.ok()` or string fallback, e.g. `cap.rs:60-72`, `lead/cleanup.rs:53-61`, `worker/prompt.rs:444-458`), inputs are herdr CLI JSON. Round-1 note, unchanged.
- `atomic_write` (`state.rs:13`) can leave a `.{file}.tmp.{pid}` file if the process dies between write and rename; `cleanup_worker_files` does not sweep `.tmp.*` (negligible window; not in the brief).
- Two pre-existing inline paths untouched by this diff: `prompt.rs:464` (`{name}.srt.json` read) and `:667` (`{name}.grid` read) — both existed at base `20cd4d2` (base lines 463/690); F4 covered the four watchdog files, which are now centralized.
- `load_owner`/`load_grid` normalize empty files to defaults (`state.rs:82-88`, `:94-99`) where the old inline reads kept `""`; reachable only with hand-corrupted/empty metadata files and strictly safer.

## Integration note (context, not a finding)

The stated base `quaqua/clean-integ` has since advanced from `20cd4d2` to `6048a28` (t1: bash-wrapper removal + merge). File-level overlap with this diff is only `crates/quaqua/src/lead/spawn.rs`, in disjoint hunks (t1 edits the Lead prompt text ~`:364`; w2 edits JSON parsing ~`:123`,`:269`) — merge expected clean, but the Lead should merge against the current `clean-integ` tip and re-run the suite there.

Verdict: clean
