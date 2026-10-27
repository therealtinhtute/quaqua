# Harness & hygiene — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71); review 2026-10-21 by truonglao (c32174b, pi added); anh iu's answers 2026-10-21 applied.
Status: **mostly done** · H1 merged #95 · H2 merged #96 · H3 pi done `64ddc49`, Codex dropped from the roster `1c3d9ae` · H4 merged #99 · H5 (optional) not checked against this spec. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Not from pstack: findings from the 2026-10-21 workspace audit (drift, reviewer sandbox) and the six-concept brief (harness adapter, new harnesses). Runs in parallel with `pstack-integration.md`, except H1 merges before its P1.

Locked decisions (anh iu, 2026-10-21): tieude + suhuynh move to **pi** (same model, `--thinking max`), isolated with `--no-extensions` + the herdr extension only · **Codex in parallel with pi** on the same registry · opencode stays a fallback harness, removed only after ≥2 weeks of stable pi · reviewer read-only = **detection first** (verdict void if HEAD or `git status --porcelain` changed during review), tool flags are prevention only · `harness/` starts as a registry (H2); the full move (H5) is optional · keep suhuynh as reviewer (accepted risk).

Order: H1 → H2 → H3 (H3a pi ∥ H3b Codex) → H4 → H5 (optional). H4 may land earlier, any time after H1. Each one works on its own after merge.

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

## H2 — harness registry (before pi and Codex)

Goal: the closed facts about a harness live in one place, so adding pi and Codex edits one table, not six string lists.
Today the same harness set is spelled out at `launch.rs:20` and `config.rs:490`; the "fallback idle" pair at `workflow.rs:132,207` and `pipeline/mod.rs:1475,1508`; effort sets at `config.rs:208-219`; stop keys at `stop.rs:21-25`; cost dispatch at `cost.rs:44-47,163,328-339` [F]. Name-prefix guesses (`sude-/detu-` at `cost.rs:46`, `tieude-` at `cost.rs:163`) stand in for a missing harness and break when a kind changes harness.
Scope: new `crates/quaqua/src/harness/mod.rs` with a `Harness` enum (`Claude, Opencode, Omp`) + `parse(&str)`, `effort_allowed`, `stop_keys`, `fallback_idle`; callers above switch to it. Launch args, auditor, usage stay where they are.
Design: enum, no trait. Pure move: no behavior change, including the name-prefix fallbacks (H3's pi PR fixes those with a repro test).
Acceptance: existing tests unchanged and green (`tests/worker_unit.rs:1430,1536` launch tests included); `rg -n '"(opencode|omp|claude)"' crates/quaqua/src --glob '!harness/**'` shows only launch-arg arms, auditor/usage CLI calls and test fixtures
Verify: `cargo test --locked -p quaqua launch config stop cost workflow`
Ownership: touches K1 (`workflow.rs`), K3 (`cost.rs`) and P1 (`pipeline/mod.rs`) in `engine/OWNERS.md` → one PR, reviewed by both leads.

## H3 — pi + Codex adapters (in parallel, both on H2)

Depends on: H2. pi and Codex are two independent PRs on the same registry; neither waits for the other.

### H3a — pi (tieude + suhuynh)
Spec: `pi-harness.md` (phases P-pi0…P-pi4, each with its acceptance check).
Summary: `kind.harness = "pi"` for tieude + suhuynh, launch `--model opencode-go/deepseek-v4.1-flash --thinking max --append-system-prompt <rules> --session-dir <runs>/.pi-sessions/<name> --no-extensions -e <herdr ext>` (isolated: only the herdr extension loads, no personal pi-subagents/tmux-subagents/memory); cost reuses omp's line parser (`cost.rs:73`, same `message.usage.cost.total` pointer [F]) but not its directory naming (`cost.rs:49-67`) [F]; fixes the `tieude-` name fallback at `cost.rs:163` that would read opencode cost for a pi worker [F].
opencode stays a supported fallback harness; removal (P-pi4) only after ≥2 weeks of stable pi, because `usage.rs:344,383` still shells to `opencode auth export`.

### H3b — Codex
Facts: codex-cli 0.163.0 [F]; flags `-m <model>`, `-c key=value`, `-s/--sandbox read-only|workspace-write` [F]; `herdr integration status` → `codex: outdated (v8 < v10)` [F] → `herdr integration install codex` needs anh iu's explicit OK (writes `~/.codex`) before the live check.
Scope: `Codex` variant in the H2 registry (allowlist, effort set, stop keys, fallback idle), a `build_launch_cmd` arm, a cost reader, a `team.toml` example kind.
Design: build = `-s workspace-write`, review = `-s read-only` (prevention; H4 is the check).
Open [A], settle by a CLI smoke (like P-pi0) before coding: how Codex loads `runs/<name>.rules.md` (`-c` instructions key vs `AGENTS.md` in the worktree); where session logs live for cost (`~/.codex/sessions`?), else cost shows "n/a", not 0; which kind uses Codex.
Acceptance: unit test asserting exact Codex launch args; live 1-line build + review in a scratch worktree: `herdr agent wait` sees idle/done, watchdog exits 0, `cost=` readable or "n/a"
Verify: `cargo test --locked -p quaqua launch config cost`

## H4 — Read-only reviewers by detection (every harness)

Goal: a review worker cannot change the worktree on any harness, or its verdict is void.
Can land any time after H1 (the detection check is harness-independent); listed after H3 so its per-harness flags cover pi and Codex.
Current: only claude gets `deny Edit(worktree)` + `dontAsk` (`launch.rs:122-150`) [F]. opencode/omp/pi reviewers can write.
Scope: `crates/quaqua/src/worker/launch.rs`, review acceptance in `pipeline/` + `worker/prompt/`, `team.toml` comment ("claude read-only flags" is stale, `team.toml:46`) [F]
Design:
- Enforce by detection, on every harness: record worktree HEAD + `git status --porcelain` at review spawn; at report time, a changed HEAD or a dirty tree voids the verdict (treated like exit 6). This is the guarantee; the flags below only reduce attempts.
- Prevention per harness:
  - claude: keep today's settings.
  - omp: `--tools=<list>` / `--no-tools` exist (`omp --help`) [F] → drop `edit`/`write`, keep read tools + `bash`.
  - pi: `--tools <allowlist>` [F] → `read,grep,find,ls,bash`, added in H3 (pi does not exist as a harness yet).
  - opencode: permission deny for edit/write in `OPENCODE_CONFIG_CONTENT` [A: confirm opencode config key]; still needed, opencode stays a fallback harness.
  - Codex: `-s read-only`, added in H3b.
- Tool allowlists are per tool, not per path: a reviewer that may run `bash` (cargo test, `mv` of its report) can still write the worktree. That is why detection, not the flag, is the check.
Acceptance: unit test per harness asserting the review launch args; repro test: a review whose worktree is dirty at report time is rejected (fails on old code)
Verify: `cargo test -p quaqua launch`, plus the targeted review-acceptance test

## H5 — full `harness/` module (optional, after H3)

Goal: one file per harness: launch args, rules channel, read-only mode, effort/variants, cost reader, quirks (claude trust dialog `spawn.rs:434`).
Scope: `harness/{claude,opencode,omp,pi,codex}.rs`; move the `build_launch_cmd` arms (`launch.rs:97-151`) and cost readers.
Acceptance: launch tests byte-identical; the `rg` in H2 shows only test fixtures.
Skip if H2 + H3 leave the remaining matches small; decide after pi lands.
