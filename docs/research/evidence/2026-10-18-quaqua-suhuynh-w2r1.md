# REVIEW w2 — json.rs removal + state.rs normalization + worker cleanup fix (round 1)

Branch `quaqua/sude-w2` @ `dfff138`, diff vs `20cd4d2` (11 files, +624/−543).
Method: full diff read; every call site cross-checked against the deleted parser and base `cap.rs` semantics; compile + full suite run in the worktree; greps below run at `dfff138`.

## Findings

**F1 (low, trivial fix) — dead bindings leave 2 compiler warnings.**
`crates/quaqua/src/worker/prompt.rs:590` (`budget_file`) and `:595` (`autoext_file`) are no longer referenced: this diff replaced their readers/writers with `WorkerMetadata::load_budget` (:653), `save_budget` (:657, :665), `load_autoext` (:660), `save_autoext` (:898), `reset_prompt_state` (:666). Evidence: `cargo build --locked -p quaqua` →
`warning: unused variable: budget_file --> crates/quaqua/src/worker/prompt.rs:590:9`
`warning: unused variable: autoext_file --> crates/quaqua/src/worker/prompt.rs:595:9`
(both are the only warnings). Fix: delete the two lines.

**F2 (low, behavior surface) — `advisor-*.state` silently added to the taskforce state class.**
`crates/quaqua/src/state.rs:249-252` recognizes `advisor-` in addition to base's `taskforce-`/`dual-lens-`. No writer produces `advisor-*.state`: `run_advisor` (crates/quaqua/src/taskforce/mod.rs:117) writes only a `{name}.taskforce` marker, scripts/ writes no `.state` files, and repo-wide grep for `advisor-` hits only state.rs:252. Effect vs base: a stray `advisor-*.state` in runs/ now reserves orchestrator cap weight (`cap.rs:99-101`) where base ignored it. Brief says zero behavior change outside fix (c) — drop the prefix or state the intent; nothing pins it with a test.

**F3 (low, latent semantics drift) — `finished` handling is not equivalent to base cap.rs.**
Base gated counting on a literal line `finished=0` (base `20cd4d2:crates/quaqua/src/cap.rs:113`, removed by this diff). New: any state file except `finished=1|true` counts as unfinished — including files with no `finished=` line or `finished=false` (state.rs:272-273, :296-297; skip at cap.rs:99-101). All in-repo writers emit 1/0 (`pipeline/mod.rs:274`; `taskforce/mod.rs:189`, `:226`, `:231`, `:265`), so no live impact, but a foreign/legacy file now reserves cap weight where base skipped it. Also state.rs accepts `true` (:273, :297) while pipeline's own loader parses it as 0 (`pipeline/mod.rs:235`) — inconsistent semantics kept side by side, untested either way.

**F4 (nit, completeness) — normalization is incomplete: prompt.rs keeps its own layout + raw writes for 4 state files.**
state.rs defines `progress_path` (:66), `elapsed_path` (:70), `nudged_path` (:74), `submit_log_path` (:78), and `reset_prompt_state`/`cleanup_worker_files` (:186-195) delete exactly those names — but prompt.rs still builds the same names inline (:587-592) and writes them raw: `File::create(&submit_log)` (:694), `fs::write(&elapsed_file, …)` (:711), `fs::write(&progress_file, …)` (:862), `fs::write(&sentinel, "")` (:951). Two sources of truth for one on-disk layout; a rename on one side silently desyncs reset/cleanup. Names match byte-for-byte today, so follow-up material for scope (b), not a current bug.

## Verified claims

- `crate::json` fully gone: `grep -rn "parse_json\|crate::json\|mod json\|JsonValue" crates/ scripts/` → no matches (exit 1); `crates/quaqua/src/json.rs` absent; `pub mod json` removed (lib.rs).
- Suite: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **148 passed, 0 failed** (18 lib + 1 cli + 10 config + 8 gate + 50 grid + 5 init + 8 pipeline_prompt + 48 worker_unit). Base 147 = 128 integration + 19 lib confirmed by counting `#[test]` at base; delta = +2 new tests (worker_unit.rs:1936, :2010), −1 deleted json.rs unit test.
- Item (c): cleanup now deletes `.budget/.grace/.cost80/.cost100` plus `.progress/.elapsed/.nudged/.submit.log` (state.rs:182-195); test creates and asserts removal of all 14 per-worker files (worker_unit.rs:1936).
- Format compatibility: writers keep byte layout for `{name}.owner|grid|srt.json`, `.{name}.{budget,grace,autoext,cost80,cost100,provider_retry}`, `pipeline-*.state` key=value (state.rs:103-163, :243-300); readers unchanged — old/new interoperate.

## Residual risks (not findings)

- serde_json is stricter than the old hand parser (trailing garbage after the JSON value, some malformed escapes were tolerated before). Every call site falls back via `.ok()`/error path and the inputs are herdr CLI JSON, but this was verified against test stubs only, not live herdr output.
- `atomic_write` (state.rs:8-19) leaves a `.{file}.tmp.{pid}` file in runs/ if the process dies between write and rename.

Verdict: 4 findings
