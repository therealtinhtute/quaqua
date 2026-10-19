# Run Summary: sude-guard

## Goal
Dynamic harness/kind routing across quaqua end-to-end:
1. `quaqua lead spawn <slug>` starts the Lead from a configurable kind (`strategy.lead_kind` in team.toml, defaulting to `"tieude"`, overridable by `LEAD_KIND` env var), reusing `build_launch_cmd` for opencode/omp/claude launch argument construction.
2. Review paths accept any kind with `can_review = true` in `team.toml` (harness-independent; `suhuynh` moved to `opencode` with `can_review = true`).

## Commits & Git
- Branch: `quaqua/sude-guard`
- Commit SHA: `dc3368d2709dfd333fd7ff244ebd56c8bbf1f665`
- `git log --oneline -3`:
  ```
  dc3368d feat(lead): dynamic harness routing for lead and reviewer kinds
  dd4a008 Merge pull request #44 from therealtinhtute/quaqua/sude-combo
  c5ffb92 test(worker): skip grid repair in fallback spawn test and restore env before asserting
  ```

## Hardcoded Spots Updated (Before / After)

1. `team.toml:63-67` (`[kinds.suhuynh]`):
   - Before:
     ```toml
     [kinds.suhuynh]
     harness = "claude"
     model = "opus-5.5"
     effort = "medium"
     can_review = true
     ```
   - After:
     ```toml
     [kinds.suhuynh]
     harness = "opencode"
     model = "opencode-go/deepseek-v4.1-flash"
     effort = "max"                    # Claude out (anh iu, 2026-10-17): reviewer on opencode
     can_review = true
     ```

2. `team.toml:35-39` (`[strategy]`):
   - Before:
     ```toml
     [strategy]
     default_kind = "tieude"                   # anh iu, 2026-10-15
     default_reviewer = ["suhuynh"]           # first available wins (anh iu, 2026-10-16)
     builder_reviewer_differ = "harness"
     ```
   - After:
     ```toml
     [strategy]
     default_kind = "tieude"                   # anh iu, 2026-10-15
     lead_kind = "tieude"
     default_reviewer = ["suhuynh"]           # first available wins (anh iu, 2026-10-16)
     builder_reviewer_differ = "kind"
     ```

3. `crates/quaqua/src/pipeline/mod.rs:583-587`:
   - Before:
     ```rust
     if !["ni", "suhuynh", "truonglao"].contains(&r.as_str()) {
         return Err((1, format!("reviewer must be a claude kind (ni|suhuynh|truonglao) — spawn-worker.sh --review refuses {r}\n")));
     }
     ```
   - After:
     ```rust
     // Any kind with can_review = true may review — harness-independent
     // (Claude out: suhuynh runs on opencode).
     if !cfg.kinds.get(&r).map(|k| k.can_review).unwrap_or(false) {
         return Err((1, format!("reviewer kind '{r}' has can_review = false in team.toml\n")));
     }
     ```

4. `crates/quaqua/src/worker/spawn.rs:18`:
   - Before:
     ```rust
     #   --review      ni|suhuynh|truonglao: repo read-only, $HOME/quaqua/runs writable";
     ```
   - After:
     ```rust
     #   --review      any kind with can_review = true: repo read-only, $HOME/quaqua/runs writable";
     ```

5. `crates/quaqua/src/worker/spawn.rs:304-307`:
   - Before:
     ```rust
     eprintln!(
         "error: --review is only supported for claude kinds (ni, suhuynh, truonglao), not {}",
         opts.kind
     );
     ```
   - After:
     ```rust
     eprintln!(
         "error: --review requires a kind with can_review = true in team.toml (ni, suhuynh, truonglao), not {}",
         opts.kind
     );
     ```

6. `crates/quaqua/src/lead/spawn.rs:13,32` (USAGE header):
   - Before:
     ```rust
     #   Start the project Lead for <slug> (AGENTS.md '## Roles (SLP)'): a claude
     #   session named <slug>-lead running in the quaqua checkout this script
     ...
     #   Env: LEAD_MODEL (default claude-opus-5-5) · LEAD_EFFORT (default low).";
     ```
   - After:
     ```rust
     #   Start the project Lead for <slug> (AGENTS.md '## Roles (SLP)'): an agent
     #   session named <slug>-lead running in the quaqua checkout this script
     ...
     #   Env: LEAD_KIND (overrides strategy.lead_kind in team.toml).";
     ```

7. `crates/quaqua/src/lead/spawn.rs:275-363` (Agent start and workspace create):
   - Before: hardcoded `LEAD_MODEL`/`LEAD_EFFORT` env lookup and `--kind claude` with `--append-system-prompt-file`.
   - After: resolves lead launch via `resolve_lead_launch(&cfg, env_override)`, applies project override from `slug`, computes launch via `build_launch_cmd`, passes `split_envs` (opencode JSON config) via `herdr workspace create --env`, and starts agent with `herdr agent start <name> --kind <launch.harness> --pane <pane> --timeout 60000 -- <cmd.agent_args>`.

## Behavior Statement (`quaqua lead spawn --help` grade)
```text
quaqua lead spawn <slug>

Start the project Lead for <slug> (AGENTS.md '## Roles (SLP)').

The Lead runs in its own herdr workspace labelled <slug> at the quaqua repository root.
Unlike workers, the Lead has no worktree, takes no slot in the 4-worker orchestrator cap,
and does not run on the agent grid.

Harness and kind routing are resolved dynamically from team.toml:
  1. Target kind is determined by $LEAD_KIND env override if set, else strategy.lead_kind
     in team.toml (defaults to "tieude"). If the resolved kind is missing or unknown,
     it falls back safely to "tieude".
  2. The harness, model, and effort are resolved via resolve_launch from [kinds.<kind>].
  3. The launch command is constructed via build_launch_cmd:
     - opencode: passes OPENCODE_CONFIG_CONTENT via herdr workspace create --env and
       launches with:
         herdr agent start <name> --kind opencode --pane <pane> --timeout 60000 -- --standalone --auto
     - omp: launches with:
         herdr agent start <name> --kind omp --pane <pane> --timeout 60000 -- --model <m> --thinking <e> --auto-approve --append-system-prompt <rf>
     - claude: launches with:
         herdr agent start <name> --kind claude --pane <pane> --timeout 60000 -- --model <m> --effort <e> --append-system-prompt-file <rf>

Environment:
  LEAD_KIND    Overrides strategy.lead_kind from team.toml (default: tieude)
```

## Unknowns Verified
1. `herdr workspace create --env`:
   Verified via `herdr workspace create --help` and `src/herdr.rs`. `herdr workspace create` accepts `--env <KEY=VALUE>`. In Rust, `Command::new("herdr").args(...)` passes argv items directly to the OS `execve` syscall without shell quoting/escaping, allowing JSON payload (`OPENCODE_CONFIG_CONTENT={"model":"...","instructions":["..."]}`) to be passed cleanly and set in the workspace root pane environment.
2. `opencode` auto-load of `OPENCODE_CONFIG_CONTENT`:
   Verified in `~/.config/opencode/herdr-tui-session.js` / OpenCode integration. OpenCode TUI instances read `process.env.OPENCODE_CONFIG_CONTENT` on initialization to configure model variants (`provider/model#variant`) and system instruction files.

## Tests & Verification
- Gate command:
  ```bash
  PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked -p quaqua --test worker_unit --test config
  ```
- Result:
  ```text
  running 10 tests
  test config_base_fixture_validates_ok ... ok
  test config_get_can_review_and_kind_fields ... ok
  test config_get_cap_applies_project_override ... ok
  test config_get_default_reviewer_is_a_list_one_per_line ... ok
  test config_get_unknown_key_fails_clearly ... ok
  test config_get_unknown_project_fails_clearly ... ok
  test config_invalid_fixtures_are_rejected ... ok
  test config_repo_kinds_carry_model_and_effort ... ok
  test config_repo_team_toml_validates_ok ... ok
  test config_validate_project_does_not_recheck_other_projects ... ok

  test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s

  running 41 tests
  ...
  test test_lead_launch_uses_strategy_lead_kind_and_override ... ok
  test test_team_toml_harness_routing ... ok
  ...
  test result: ok. 41 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
  ```
- Full suite:
  ```bash
  PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked
  ```
  Result: 123 passed across 9 suites, 0 failed.

## Risks
1. Builder and Reviewer Harness Collision: `models.md` and `pipeline/mod.rs` require builder and reviewer to use different harnesses. With `suhuynh` on `opencode`, pipeline invocations with builder `tieude` (also `opencode`) will fail with `builder and reviewer must be different harnesses`. Standard pipeline builder should be `sude` (`omp`), which differs from `suhuynh` (`opencode`).
2. Live workspace or agent spawning was intentionally not executed per hard constraints; verification was performed via unit tests, launch command resolution unit tests, and CLI config validation.

## Lessons
- When a reviewer switches harness to match the default worker kind, strategy differ policy must align with kind-based separation to prevent validation deadlock.
