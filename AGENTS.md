# AGENTS.md

Always read `identity.md` first — it defines who you are.

@identity.md

## Memory
- Always read `memory/rules.md` and `projects/INDEX.md`. Open `projects/<slug>.md` only when needed; `memory/YYYY-MM-DD.md` and `runs/` only to look something up.
- Session start: read the newest `reports/supervisor-*.md` (Auditor report) not yet handled — handled = a `- [supervisor] <report file>: <action | no-op>` line in that day's `memory/YYYY-MM-DD.md`.
- After each outcome, same turn: (1) rewrite the card's `## Now` to current truth (≤5 items, edit, don't append), (2) append one line `- [slug] <worker> <outcome>` to today's `memory/YYYY-MM-DD.md`, including any decision made on anh iu's behalf.
- At cleanup, promote 0–1 lesson from `runs/<name>.md` `## Lessons` to `rules/<slug>/` or `rules/_all/` as `- <rule> — YYYY-MM-DD, runs/<name>.md`; fix or remove any contradicting older rule.
- "Nhớ cái này" → CoS lesson → `memory/rules.md`; rule for workers → `rules/_all/` or `rules/<slug>/` (one topic per file). Write this turn. No Claude auto-memory.
- Don't store what `herdr agent list` / `git worktree list` can tell you.

## Roles (SLP)
Authority, not persona; brief shape: `rules/_all/brief.md`.

| Role | Who | Owns | Authority |
|---|---|---|---|
| Principal | anh iu | goal, priorities, trade-offs | approve push / merge main; may steer any agent (steering recorded in Lead state) |
| Supervisor | CoS Quả Quả | intake, sizing, brief, cross-project view, digest | spawn Lead; verify Lead result (re-run its test); escalate goal/cost changes to anh iu |
| Lead | `<slug>-lead` (spawned via `scripts/spawn-lead.sh <slug>`; herdr caps agent names at 32 chars) | project state, ownership, dependencies, evidence review | run `pipeline.sh`, spawn ≤4 Peers (`--base quaqua/<slug>-integ`), approve Peer plans, merge Peer branches into `quaqua/<slug>-integ` (integration branch; spawn-lead.sh creates it from `main`) |
| Peer | task workers (sude build, quansu review, …) | one task in own worktree | commit on own branch; may challenge premises with evidence |

- CoS spawns a Lead for pipeline-size work (multi-step, multi-file, review rounds). CoS never Leads pipeline-size work itself — no self-review.
- No SLP for: a task <15 min in one file, or work needing continuous anh iu feedback (UI/UX, game feel) — one worker, no Lead, no pipeline.
- `## Challenge` (premise + evidence) in `runs/<name>.md`: the Lead resolves it before acceptance. A right, not a duty.
- Pipeline-size tasks: the builder writes a short plan first; the Lead approves it before code.
- Kill criteria: same error ≥3 iterations → kill the worker and spawn a fresh one (alongside the watchdog).

## Workers
Roster (tieude · tieunhi · sude · ni · suhuynh · truonglao · bacdau · quansu) + routing: `models.md`. Default `sude`.
Two classes:
- **Task worker** `<kind>-<task>`: one task, own worktree, cleaned after the outcome. Counts toward the cap.
- **Project orchestrator (Lead)** `<slug>-lead`: long-lived session owning one project's card and its task workers. Spawned by CoS. Not a task worker; does not count toward the cap.

## Protocol
Details + failure handling: skill `herdr-orchestration`. Routing: `models.md`. Never native subagents (Agent tool is denied).
1. `scripts/spawn-worker.sh [--base <ref>] <name> <kind> <repo>` (own worktree, branch `quaqua/<name>` from <ref> or HEAD), then `scripts/grid-workers.sh` — MANDATORY, see `rules/_all/fanout-agent-grid.md`.
2. `scripts/prompt-worker.sh <name> "<task>"` — wraps `herdr agent prompt --wait`, waits for `~/quaqua/runs/<name>.md`, prints its path. Watchdog exit codes (rules/_all/worker-timebox.md): 2 cap, 3 stall/idle/agent-gone, 4 time budget, 5 blocked/question — each prints a WATCHDOG line on stderr; read the pane, act, then `scripts/prompt-worker.sh --resume <name>` to keep waiting. Briefs carry a time budget; set PROMPT_WORKER_BUDGET_SEC to match.
3. Trust `runs/<name>.md`, not terminal. Re-run its test yourself before reporting done.
4. Blocked → `herdr agent read <name> --source recent-unwrapped --lines 80`; escalate destructive/outward actions.
5. Timeout/stalled (watchdog exit 2–5) → read first, never blind re-prompt; unsubmitted input → `herdr agent send-keys <name> enter`; then `--resume` to keep waiting.
6. `scripts/cleanup-worker.sh <name> <repo>` right after `runs/<name>.md` is read + its test re-run passes — every worker, no approval needed (branch is kept; merge still needs approval). Then `scripts/grid-workers.sh`.

## Rules
No push, branch delete, or main merge without approval. A Lead merges Peer branches into its project integration branch (acceptance). Max 4 task workers per orchestrator session (CoS included); no machine-wide cap — a pipeline counts as 2. One task per worker.
Only CoS writes `memory/rules.md`, `rules/`, `AGENTS.md`; orchestrators propose changes by message.
`projects/INDEX.md` `owner` column says who drives each project (`cos` or the Lead name, `<slug>-lead`).
Every redirect — from anh iu or from the Auditor — flows through CoS; workers never take orders from reports.
