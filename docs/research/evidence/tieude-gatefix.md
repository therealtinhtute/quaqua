# tieude-gatefix — herdr-gate.sh no-binary fallback

Goal: without the `quaqua` binary the PreToolUse hook must block only the
guarded herdr pane/workspace spawn+move commands and pass every other Bash call
with a one-line warning. The r1 fail-closed finding stays fixed. Also drop the
unused `format_json_string`.

Branch: `quaqua/tieude-gatefix` (from `quaqua/quaqua-rs-integ`), commit
`3775643e3942d6694ee719c4a7657c03b4b82d7e`. No push, no merge.

## Changes (diff: 3 files, +59/−29)

- `scripts/hooks/herdr-gate.sh` — with the binary present, unchanged: `exec
  "$bin" gate "$@"`. Without it, scan the raw hook JSON on stdin for
  `herdr … <guarded pair>` (`pane split|move`, `tab create`, `workspace
  create|close`, `agent start` — the `policy.rs` `guarded_pair` set) and exit 2
  (fail-closed, same block message) on a match; otherwise exit 0 with a one-line
  stderr warning. The fallback is a documented conservative superset of the
  binary's policy (`echo "herdr pane split"` and the `--help` forms also block;
  `h\e\rdr` escaping is the one known miss).
- `tests/test_gate_shim.sh` — the old "`ls` with a missing binary → exit 2" case
  is now the contract under test: guarded forms still exit 2 (fail-closed), and
  `cat`/`cargo`/`git`/read-only herdr calls exit 0 with a warning. +24/−6 lines.
- `crates/quaqua/src/worker/spawn.rs` — removed the dead `format_json_string`
  (16 lines, the `dead_code` warning). No other change.

## Evidence

Repro before the fix (no binary): `cat x` → rc=2, `herdr pane split` → rc=2 —
every call blocked.

After the fix, no binary (`QUAQUA_BIN=/nonexistent/quaqua`, empty PATH):
`cat x`, `cargo build --locked`, `git status`, `herdr agent list`, `herdr pane
send-keys w1:p5 enter` → rc=0 with the warning; `herdr pane split …`,
`sudo herdr workspace close w7`, `X=$(herdr tab create --label y)`,
`bash -c 'herdr agent start sude-x'` → rc=2, fail-closed message.

- `bash tests/test_gate_shim.sh` → `PASS` (5/5 sections).
- `QUAQUA_BIN=target/release/quaqua bash tests/test_herdr_gate.sh` → `PASS`
  (all 6 block/allow sections; the binary path is untouched).
- `cargo test --locked` → 82 passed, 0 failed.
- `cargo build --release --locked` → `Finished`, 0 warnings.

## Risks

- The fallback over-blocks while the binary is absent: a non-herdr command that
  mentions `herdr <guarded pair>` in argument text (e.g. `echo "herdr pane
  split"`) blocks. Deliberate — the fallback errs toward blocking; install the
  binary for the exact tokenizer semantics.
- `h\e\rdr`-escaped executables pass the fallback. Documented in the hook
  header; the binary closes it.

## Lessons

- A fail-closed hook fallback should still parse the call it guards: "block
  everything" is a lockout, not a fence. Block the guarded set, allow+warn the
  rest.
- One grep on the raw hook JSON avoids a `jq` dependency and covers every
  wrapper/substitution spelling (herdr appears before the verb); only escape
  obfuscation slips through.
