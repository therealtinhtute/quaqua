# tieude-t1fix — spawn_lead test fragment list matches new lead prompt text

## Goal
`tests/test_spawn_lead.sh` happy-path fragment list matches the new lead spawn prompt; suite passes.

## Changes
- `tests/test_spawn_lead.sh:111` — fragment `'pipeline.sh'` → `'quaqua pipeline'` (new prompt text: "Pipeline-size work runs through quaqua pipeline." — `crates/quaqua/src/lead/spawn.rs:412`).
- No other fragment in the list was stale: `'Roles (SLP)'`, `'projects/coucou-island.md'`, `'not counted toward the 4-worker cap'`, `'quaqua/coucou-island-integ'`, `'reaches you through the CoS'` all still match the new prompt — verified by the full suite run below.

## Evidence
- Commit `694f42546a50b086b74031daf17fa4c8f21ede29` on `quaqua/tieude-t1fix` (base `quaqua/sude-bashclean` = a35a54d).
- `git diff --stat HEAD~1 HEAD` → `tests/test_spawn_lead.sh | 2 +-` (1 insertion, 1 deletion).
- Repro before fix: `FAIL: happy: brief missing 'pipeline.sh'` (exit 1).
- Build:
  `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo build --locked -p quaqua` → `Finished dev profile` (exit 0).
- Test:
  `env -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin QUAQUA_BIN=$(pwd)/target/debug/quaqua bash tests/test_spawn_lead.sh` → all checks ok, `PASS`, exit 0 (run post-commit on 694f425).

## Risks
- None known. The fragment `'quaqua pipeline'` is a literal substring of the prompt line; if the lead prompt wording changes again the test will fail loudly (intended pin).
- Only `tests/test_spawn_lead.sh` touched; no push, no merge, no branch delete.

## Blockers
none

## Lessons
- Prompt-text assertions should grep a stable phrase, not a script filename, or removing a wrapper breaks the pin for the wrong reason.
