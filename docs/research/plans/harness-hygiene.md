# Harness & hygiene — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71); review 2026-10-21 (c32174b, pi added).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Not from pstack: findings from the 2026-10-21 workspace audit (drift, reviewer sandbox) and the six-concept brief (harness adapter, new harnesses). Runs in parallel with `pstack-integration.md`, except H1 merges before its P1.

Locked decisions: tieude + suhuynh move to **pi** (same model, `--thinking max`) · Codex deferred to H5 · reviewer read-only right after H1 · `harness/` starts as a registry (H4a) before pi; the full move (H4b) is optional · keep suhuynh as reviewer (accepted risk).

Order: H1 → H2 → H4a → H3 (pi) → H4b → H5. H2 stays right after H1 (its check is harness-independent); pi gets its reviewer flag in its own PR. Each one works on its own after merge.

---

## H1 — Fix contract drift

Goal: the rules a worker reads match what the engine enforces.
Scope:
- `templates/workspace/.claude/skills/herdr-orchestration/SKILL.md` ("Max 4" → follow `team.toml` `max_workers_per_orchestrator = 8`) [F]
- `templates/workspace/PEER.md` + `templates/workspace/rules/_all/brief.md`: merge the 6-field handback (Outcome/Snapshot/Scope/Verification/Unknown/Ownership) into the `runs/<name>.md` report format, one contract [F]
- `brief.md` says numbered headings fail; `watchdog.rs:22-49` accepts `## 5. Lessons` [F]. Fix the doc so it matches the code.
Acceptance:
- One handback format, referenced from both PEER.md and brief.md
- `grep -rn "Max 4"` under templates/ returns nothing
Verify: `cargo test -p quaqua watchdog`; `quaqua worker prompt --check-brief` on a sample brief
Forbidden: changing watchdog parsing logic

## H2 — Read-only reviewers on every harness

Goal: a review worker cannot change the worktree on any harness, or its verdict is void.
Current: only claude gets `deny Edit(worktree)` + `dontAsk` (`launch.rs:122-150`) [F]. opencode/omp/pi reviewers can write.
Scope: `crates/quaqua/src/worker/launch.rs`, review acceptance in `pipeline/` + `worker/prompt/`, `team.toml` comment ("claude read-only flags" is stale, `team.toml:46`) [F]
Design:
- Enforce by detection, on every harness: record worktree HEAD + `git status --porcelain` at review spawn; at report time, a changed HEAD or a dirty tree voids the verdict (treated like exit 6). This is the guarantee; the flags below only reduce attempts.
- Prevention per harness:
  - claude: keep today's settings.
  - omp: `--tools=<list>` / `--no-tools` exist (`omp --help`) [F] → drop `edit`/`write`, keep read tools + `bash`.
  - pi: `--tools <allowlist>` [F] → `read,grep,find,ls,bash`, added in H3 (pi does not exist as a harness yet).
  - opencode: permission deny for edit/write in `OPENCODE_CONFIG_CONTENT` [A: confirm opencode config key]; moot once no kind uses opencode.
- Tool allowlists are per tool, not per path: a reviewer that may run `bash` (cargo test, `mv` of its report) can still write the worktree. That is why detection, not the flag, is the check.
Acceptance: unit test per harness asserting the review launch args; repro test: a review whose worktree is dirty at report time is rejected (fails on old code)
Verify: `cargo test -p quaqua launch`, plus the targeted review-acceptance test

## H3 — pi harness (replaces Codex here)

Spec: `pi-harness.md` (phases P-pi0…P-pi4, each with its acceptance check).
Summary: `kind.harness = "pi"` for tieude + suhuynh, launch `--model opencode-go/deepseek-v4.1-flash --thinking max --append-system-prompt <rules> --session-dir <runs>/.pi-sessions/<name>`; cost reuses omp's line parser (`cost.rs:73`, same `message.usage.cost.total` pointer [F]) but not its directory naming (`cost.rs:49-67`) [F]; fixes the `tieude-` name fallback at `cost.rs:163` that would read opencode cost for a pi worker [F].
Depends on: H4a.

## H4a — harness registry (before pi)

Goal: the closed facts about a harness live in one place, so adding pi edits one table, not six string lists.
Today the same harness set is spelled out at `launch.rs:20` and `config.rs:490`; the "fallback idle" pair at `workflow.rs:132,207` and `pipeline/mod.rs:1475,1508`; effort sets at `config.rs:208-219`; stop keys at `stop.rs:21-25`; cost dispatch at `cost.rs:44-47,163,328-339` [F]. Name-prefix guesses (`sude-/detu-` at `cost.rs:46`, `tieude-` at `cost.rs:163`) stand in for a missing harness and break when a kind changes harness.
Scope: new `crates/quaqua/src/harness/mod.rs` with a `Harness` enum (`Claude, Opencode, Omp`) + `parse(&str)`, `effort_allowed`, `stop_keys`, `fallback_idle`; callers above switch to it. Launch args, auditor, usage stay where they are.
Design: enum, no trait. Pure move: no behavior change, including the name-prefix fallbacks (pi's PR fixes those with a repro test).
Acceptance: existing tests unchanged and green (`tests/worker_unit.rs:1430,1536` launch tests included); `rg -n '"(opencode|omp|claude)"' crates/quaqua/src --glob '!harness/**'` shows only launch-arg arms, auditor/usage CLI calls and test fixtures
Verify: `cargo test --locked -p quaqua launch config stop cost workflow`
Ownership: touches K1 (`workflow.rs`), K3 (`cost.rs`) and P1 (`pipeline/mod.rs`) in `engine/OWNERS.md` → one PR, reviewed by both leads.

## H4b — full `harness/` module (optional, after pi)

Goal: one file per harness: launch args, rules channel, read-only mode, effort/variants, cost reader, quirks (claude trust dialog `spawn.rs:434`).
Scope: `harness/{claude,opencode,omp,pi}.rs`; move the `build_launch_cmd` arms (`launch.rs:97-151`) and cost readers.
Acceptance: launch tests byte-identical; the `rg` above shows only test fixtures.
Skip if H4a + pi leave the remaining matches small; decide after pi lands.

## H5 — Codex harness (deferred)

Facts kept from the first draft: codex-cli 0.163.0 [F]; flags `-m`, `-c key=value`, `-s/--sandbox` [F]; `herdr integration status` → `codex: outdated (v8 < v10)` [F] → `herdr integration install codex` needs ní's OK (writes `~/.codex`).
Open: rules channel, session-log location, which kind would use it. Re-spec on H4a when a kind needs it.
