# tieude-plugren — rename herdr plugin `herdr-grid` → `herdr-quaqua`

Branch `quaqua/tieude-plugren` @ `a511231e689c093d3db08cf67a7620a387af0c45` (base `main` @ `ae9a7ef`). Working tree clean; nothing pushed; no `herdr plugin *` command was run.

## Changes

`git show --stat -M a511231` — 13 files, +47/−32; the directory rename reads as:

```
 crates/quaqua/src/grid/hook.rs                         | 10 +++++-----
 crates/quaqua/src/herdr.rs                             |  2 +-
 crates/quaqua/src/init.rs                              |  4 ++--
 crates/quaqua/src/lead/spawn.rs                        |  2 +-
 crates/quaqua/tests/grid.rs                            | 16 +++++++++++++++-
 plugins/{herdr-grid => herdr-quaqua}/README.md         | 11 ++++++-----
 plugins/{herdr-grid => herdr-quaqua}/herdr-plugin.toml |  4 ++--
 plugins/{herdr-grid => herdr-quaqua}/plan.jq           |  0        <- R100
 tests/test_herdr_grid_plan.sh                          |  4 ++--
 tests/test_herdr_grid_rust.sh                          |  2 +-
 tests/test_init_workspace.sh                           | 12 ++++++------
 tests/test_spawn_lead.sh                               |  4 ++--
 tests/test_spawn_worker.sh                             |  8 ++++----
```

- `herdr-plugin.toml`: `id` + `name` = `herdr-quaqua`; events/commands unchanged (`quaqua grid hook`).
- `README.md`: title, prose, state-dir path, env-var docs; documents the legacy env name and that old state is not auto-migrated.
- `grid/hook.rs`: state dirs → `herdr/plugins/herdr-quaqua` (XDG + HOME branch) and `.herdr-quaqua-state` (no-HOME fallback); lock error prefix `[herdr-quaqua]`; shadow env reads `HERDR_QUAQUA_SHADOW` first with a one-line `.or_else` fallback to legacy `HERDR_GRID_SHADOW` (both honoured).
- `--source "herdr-quaqua"`: `herdr.rs:116`, `lead/spawn.rs:292`, `init.rs:335` (+ doc comment `init.rs:304`); `grid_*` token names untouched. Confirmed write-only: no other reference to the source string anywhere (matches the brief's prior verification).
- Tests: `tests/grid.rs` oracle path → `plugins/herdr-quaqua/plan.jq`, and the shadow test extended (env-vs-file precedence; QUAQUA wins over legacy; legacy alone still disables shadow); plan paths in `tests/test_herdr_grid_rust.sh:11` and `tests/test_herdr_grid_plan.sh:38`; source-label greps in `test_spawn_lead.sh`, `test_spawn_worker.sh`, `test_init_workspace.sh`.

## Evidence

1. Full suite (exact brief command): `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → exit 0, **172 passed / 0 failed** (11 targets). CI runs exactly this command (`.github/workflows/ci.yml`), no fmt/clippy gate.
2. Shadow smoke: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked -p quaqua --test grid test_hook_shadow_mode` → `test test_hook_shadow_mode_file_precedence_and_empty_snapshot ... ok` (1 passed / 0 failed).
3. Shell suites, run against the branch binary (`QUAQUA_BIN=target/debug/quaqua`, the `tests/run.sh` convention, plus `env -u HERDR_BIN_PATH -u HERDR_SOCKET_PATH` for isolation):
   - `tests/test_herdr_grid_rust.sh` → `PASS: all test_herdr_grid_rust.sh cases passed` (debug binary supplied; its internal release build skipped via `QUAQUA_BIN`)
   - `tests/test_herdr_grid_plan.sh` → `ALL TESTS PASSED`
   - `tests/test_spawn_lead.sh` → PASS · `tests/test_spawn_worker.sh` → PASS · `tests/test_init_workspace.sh` → PASS (the three I edited for the source label)
   - Dead/skip suites still exit 0: `test_herdr_grid_hook.sh`, `test_herdr_grid_apply.sh`, `test_grid_workers.sh` (all print SKIP).
   - Trap encountered: without `QUAQUA_BIN`, these scripts run the mise-installed quaqua (old label) and `test_spawn_lead.sh` fails on the source grep — an environment staleness, not the branch. Re-run with `QUAQUA_BIN=target/debug/quaqua` and it passes.

4. Grep table for `herdr-grid` (matched lines, source tree, `target/` excluded):

| area | before (`ae9a7ef`) | after (`a511231`) | note |
|---|---|---|---|
| `plugins/` | 5 | **1** | the one hit is the README migration note naming the old state dir (`README.md:33`) — intentional, it tells the operator what to move |
| `crates/` | 11 | **2** | `grid.rs:64` fixtures path + `config.rs:3` pointer to the docs/research doc (both must keep the old name, see exceptions) |
| `rules/` | 0 | **0** | clean |
| `README.md`, `AGENTS.md` | 0 | **0** | clean |
| `tests/` | 37 | 22 | breakdown below |
| `team.toml` | 1 | 1 | pointer to the docs/research doc filename |
| `docs/research/*` | 8 | 8 | exempt (history, untouched) |
| `tests/fixtures/herdr-grid/*` | 4 | 4 | exempt per brief (fixtures NOT renamed — proposal below) |

`tests/` after, by file: `test_herdr_grid_hook.sh` 9 + `test_herdr_grid_apply.sh` 7 (dead suites that SKIP in R6 cutover — left untouched as suspected-deletion files per the brief), `test_herdr_grid_rust.sh` 3 (2× fixtures path + 1× mktemp prefix `test-herdr-grid`, cosmetic), `test_prompt_worker.sh` 1 + `test_herdr_grid_plan.sh` 1 (fixtures paths), `test_config.sh` 1 (docs/research pointer). Outside the worktree: `~/quaqua/memory/` has 11 historical hits (exempt).

Exceptions, explicit: (a) `tests/fixtures/herdr-grid/` keeps its name by instruction — a follow-up rename must touch `grid.rs:64`, `test_herdr_grid_rust.sh:4,10`, `test_herdr_grid_plan.sh:39`, `test_prompt_worker.sh:342` (+ the dead suites); (b) `docs/research/herdr-grid-rust-migration-plan.md` / `-edgecases.md` are history, so the three pointers to that filename (`config.rs:3`, `team.toml:2`, `test_config.sh:91`) stay correct as-is; (c) the README migration note reuses the old name deliberately.

## Live re-register (operator, after merge — not run here)

Finding: `~/.config/herdr/plugins.json` has one enabled registration, `plugin_id: "herdr-grid"`, whose source is a github-installation checkout `…/github-installations/herdr-grid-1156f788bb72/<…>/checkout/plugins/herdr-grid/` and whose events run `["bash","hook.sh"]` (v1.0.0, pre-R6). It never tracked the in-repo manifest (which already ran `quaqua grid hook` before this rename), so the live daemon is still driving the old bash hook — its state dir `~/.local/state/herdr/plugins/herdr-grid/` holds a fresh `last_snapshot.json` + 430 KB `shadow.log` (mtime 2026-10-18 20:19, i.e. live events during this session).

Suggested sequence once this branch is merged (from `~/quaqua`):

```
herdr plugin uninstall herdr-grid
herdr plugin link ~/quaqua/plugins/herdr-quaqua
herdr plugin enable herdr-quaqua   # verify exact enable syntax with `herdr plugin --help`
```

State migration (do it, or the renamed hook starts with no snapshot baseline and shadow mode resets to ON because the `shadow` flag file is left behind):

```
mv ~/.local/state/herdr/plugins/herdr-grid ~/.local/state/herdr/plugins/herdr-quaqua
```

## Risks / notes

- State-dir default changed with no auto-migration; the legacy env fallback (`HERDR_GRID_SHADOW`) covers env-based shadow setup, not the `shadow` file in the old state dir.
- `tests/fixtures/herdr-grid` rename is proposed (not done, per brief) — see exceptions list for the exact follow-up touch points.
- Cosmetic leftovers: mktemp prefix in `test_herdr_grid_rust.sh:18`; dead-suite refs left to the worker that deletes those files.
- `cargo test` run had no warnings for the touched crates; commit is not pushed.

## Blockers

none

## Lessons

- Renaming a plugin in-repo leaves the live registration and its state dir under the old name — check `~/.config/herdr/plugins.json`, not just grep.
- Shell tests in this repo run the mise-installed quaqua unless `QUAQUA_BIN=target/debug/quaqua` is set; always set it when testing a branch.
