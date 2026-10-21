# Harness & hygiene — PR specs

Source: brainstorm 2026-10-21 (pstack @ 190d287 vs quaqua main d24be71).
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

Not from pstack: findings from the 2026-10-21 workspace audit (drift, reviewer sandbox) and the six-concept brief (harness adapter), plus anh iu's 2026-10-21 call to replace opencode with pi. Runs in parallel with `pstack-integration.md`, except H1 merges before its P1.

Locked decisions (2026-10-21): `harness/` module first, then pi replaces opencode, Codex later · `harness/` module is a pure move · keep suhuynh as reviewer (accepted risk; pi makes it read-only).

Order: H1 → H2 → H3 → H4 → H5 (H5 deferred). H2 starts after `quaqua/tieude-nits` (cost.rs) lands. Each one works on its own after merge.

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

## H2 — `harness/` module

Goal: one place per harness: launch args, rules channel, read-only mode, effort/variants, cost reader, quirks (claude trust dialog `spawn.rs:434`).
Scope: new `crates/quaqua/src/harness/{mod,claude,opencode,omp}.rs`; move the per-harness `match` sites [F, CoS 2026-10-21]: `launch.rs:20` (allowlist), `launch.rs:42,98`, `cost.rs:148,544,664,715,790`, `stop.rs:22`, `workflow.rs:132,207`, `auditor.rs:236,374`, `activities.rs:667`, `config.rs:209`, `usage.rs:344`.
Design: a `Harness` trait or enum dispatch. Pure move, no behavior change.
Ownership: `cost.rs` = worker-lead (K3); `launch.rs`, `config.rs`, `usage.rs`, `auditor.rs`, `worker/spawn.rs` have no row in `engine/OWNERS.md` → add rows for `harness/` and these files in this PR.
Acceptance: the existing launch-arg tests are byte-identical before and after; `rg "match .*harness"` outside harness/ → 0
Verify: `cargo test -p quaqua`

## H3 — pi harness replaces opencode

Goal: tieude and suhuynh run on pi instead of opencode, same model; the opencode adapter is deleted once no kind uses it.
Facts:
- pi 1.3.0: `pi --model opencode-go/deepseek-v4.1-flash --thinking max`; sessions under `~/.pi/agent/sessions`; `--tools <allowlist>` restricts tools [F, CoS 2026-10-21]
- herdr integration for pi is current (v9) [F, `herdr integration status`]
Scope: `harness/pi.rs` (new), delete `harness/opencode.rs`, `team.toml`/`models.md` (tieude, suhuynh → `harness = "pi"`)
Design:
- launch: build = full tools; review = `--tools` read-only allowlist (gives suhuynh a real read-only reviewer)
- rules channel: [A] how pi takes `runs/<name>.rules.md` (system-prompt flag vs file). Settle by test before coding.
- cost reader: parse `~/.pi/agent/sessions` [A: format]
- effort: map `max` → `--thinking max`; reject values pi doesn't accept
Acceptance: `herdr agent wait` sees idle/done for a pi pane; watchdog exit codes hold; a pi review attempting an edit is refused; `rg -i opencode crates/` → only history/tests that are intentionally kept
Verify: `cargo test -p quaqua`; one live 1-line build + one review in a scratch worktree

## H4 — Read-only reviewers on the remaining harnesses

Goal: no harness can review with write access.
Current: claude has `deny Edit(worktree)` + `dontAsk` [F]; pi gets it in H3; omp has none [F].
Scope: `harness/omp.rs`, `team.toml` comment ("claude read-only flags" is stale) [F]
Design: find omp's read-only/tool-deny flag [A]. If none exists, `can_review = true` on an omp kind is a config error, never a silent write-capable review.
Acceptance: unit test per harness asserting review launch args carry the restriction, or the config error for omp
Verify: `cargo test -p quaqua harness config`

## H5 — Codex harness (deferred)

Goal: `kind = codex` as one more adapter on the H2 seam. Not scheduled.
Facts: codex-cli 0.163.0 [F]; flags `-m`, `-c key=value`, `-s read-only|workspace-write` [F]; herdr integration outdated (v8 < v10) [F] → `herdr integration install codex` needs anh iu's OK (writes to `~/.codex`).
Open: rules channel (`-c` instructions key vs `AGENTS.md`), cost log location (`~/.codex/sessions`?) [A].
