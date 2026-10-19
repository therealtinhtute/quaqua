# Run Report: sude-pollpre

## Summary
- Embedded `pre-commit` hook template (`PRE_COMMIT_TEMPLATE`) directly into `crates/quaqua/src/init.rs`.
- `quaqua init` creates `<root>/.git/quaqua-hooks/`, writes `pre-commit` (+exec 0o755), and sets `core.hooksPath` pointing to `<root>/.git/quaqua-hooks`.
- Added idempotent refresh: if hook content differs from embedded template, refreshes and logs `hooks: refreshed <path>`; if identical, keeps quiet.
- Added legacy migration: if `core.hooksPath` was set to `<root>/scripts/hooks`, migrates to `<root>/.git/quaqua-hooks` and logs `hooks: migrated core.hooksPath from '<have>' to <want>`. If already set to target, leaves unchanged and silent. Foreign values still trigger warning.
- Removed tracked `scripts/hooks/pre-commit` from repository while keeping `scripts/hooks/herdr-gate.sh`.
- Updated `tests/test_pre_commit.sh` to initialize a sandbox repo via `quaqua init`, generating the hook and verifying all 9 existing portable-path assertions + added case 10 for legacy migration and idempotent refresh.
- Updated `tests/test_init_workspace.sh` and added `crates/quaqua/tests/init.rs:init_wires_embedded_hook_and_migrates_legacy` unit/integration test.

## Commit
- Commit SHA: `72e54b1` (`72e54b1b7e168eb0d56b42b9954ded1ecdfe678c`)
- Diff stat (`git show 72e54b1 --stat`):
```
 crates/quaqua/src/init.rs    | 155 +++++++++++++++++++++++++++++++++++++++++--
 crates/quaqua/tests/init.rs  |  92 +++++++++++++++++++++++++
 scripts/hooks/pre-commit     |  65 ------------------
 tests/test_init_workspace.sh |  23 ++++---
 tests/test_pre_commit.sh     |  63 +++++++++++++++---
 5 files changed, 306 insertions(+), 92 deletions(-)
```

## Evidence

### 1. `git ls-files scripts/hooks/`
```
scripts/hooks/herdr-gate.sh
```

### 2. `test_pre_commit.sh`
Command:
```bash
env -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin QUAQUA_BIN=$(pwd)/target/debug/quaqua bash tests/test_pre_commit.sh
```
Output:
```
ok: sandbox repo initialized (core.hooksPath -> .git/quaqua-hooks)
ok: added /home/<user>/ path -> rejected, file + line + policy named
ok: real git commit through core.hooksPath -> blocked
ok: added /Users/<user>/ path -> rejected
ok: tests/** fixture with a machine path -> allowed
ok: rules/_all/portable-paths.md with the pattern -> allowed
ok: allowlist is exact (tests-helper.sh still checked)
ok: ~/, $HOME, $(dirname "$0"), placeholders, /etc -> allowed
ok: existing violations are not re-judged; clean edits still commit
ok: pure rename allowed; rename + added machine path rejected under the new name
ok: binary diff -> allowed (no added lines)
ok: clean index -> allowed; unborn-branch first commit with a machine path -> blocked
ok: hook source is free of literal machine paths and is executable
ok: legacy core.hooksPath migrated and hook refresh idempotent
PASS
```

### 3. Demo Sandbox Init + Commit Gate
```
--- quaqua init stdout ---
created memory/
created projects/
created raw/
created runs/
created reports/
created memory/rules.md
created projects/INDEX.md
created projects/_template.md
created USER.md
hooks: core.hooksPath=/private/var/folders/00/1jsbl9d94hl8k0bh1qnlyb5h0000gn/T/tmphr1uqn79/demo-repo/.git/quaqua-hooks
workspace ready in /private/var/folders/00/1jsbl9d94hl8k0bh1qnlyb5h0000gn/T/tmphr1uqn79/demo-repo
cargo install --path crates/quaqua

--- core.hooksPath ---
/private/var/folders/00/1jsbl9d94hl8k0bh1qnlyb5h0000gn/T/tmphr1uqn79/demo-repo/.git/quaqua-hooks

--- git commit output ---
exit code: 1
script.sh: cat /home/<user>/secret.txt
pre-commit: machine-specific absolute path in the added lines above.
Use a repo-relative path, ~/, $HOME or $(dirname "$0") instead
— rules/_all/portable-paths.md
```

### 4. Test Suites
- `cargo test --locked -p quaqua --test init` → PASS (6/6 tests)
- `bash tests/test_init_workspace.sh` → PASS

## Risks
- Existing workspaces must run `quaqua init` to migrate `core.hooksPath` from `scripts/hooks` to `.git/quaqua-hooks`.

## Blockers
none

## Lessons
- Generating hooks from binary templates into `.git/` decouples repository tracking from personal workspace hooks and enables atomic binary updates across workspaces.
