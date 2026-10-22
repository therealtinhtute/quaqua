# pstack integration — PR specs

Source: review session 2026-10-21 (pstack @ 190d287, plugin 0.15.29, vs quaqua main d24be71). Revised 2026-10-21 against main 309f698 (H1 + H2 merged; H3a + H4 in flight). v3 adds the router, figure-it-out, session lifecycle, and the orchestrate rules found on a second read. v3.1 (2026-10-21, main dd01e68, H3a + H4 merged): P1 split into P1a/P1b, coordinator-never-edits-code rule, merge warns without a verdict at head, fixed Verify lines, and an evidence audit (§ Audit). v3.2 (2026-10-22): non-Claude second pass on § Audit (`runs/suhuynh-pstackfit.md`), counts redone with recorded filters, exit 6 starts as a warning, 24 principles as cited leaves, 18 playbooks in P2a–d, the SLP team mapping, P3 first.
Status: **draft spec** (chưa giao worker) · spec only, nothing implemented. [F] = verified in repo/CLI or the pstack source, [A] = assumption the PR must confirm first.

Scope: only ideas ported from [pstack](https://github.com/cursor/plugins/tree/main/pstack): its skills, principles and playbooks. Nothing from other skill sets. Non-pstack work (drift, read-only reviewers, pi, `harness/`, Codex) lives in `harness-hygiene.md` + `pi-harness.md`.

Locked decisions: workflow-first, not agent-first: the playbook is picked before the agent (P1b skill steps), and the harness is an adapter (`HARNESS.md`) · port ideas, not the pstack plugin · router = playbook data + `quaqua playbook list|show` + a `/poteto-mode` skill that is only the front door · Playbook header + Orchestrator/Worker split; the `## Playbook` report check warns through the pilot, then a missing section is exit 6 (§ P1a) · port all 18 playbooks before eval · principles = 24 leaf files, loaded only when a playbook step cites them · verdict = journal event, no TSV · brief gains Scope, Acceptance, Verify, Context; they warn, and become required only when the P2c eval shows a missing header caused a real failure · coordinator = CoS, or the Lead inside an SLP team (§ SLP team) · no orch.ts/store.ts, no standing-orders file, no swarm.

| PR | From pstack |
|---|---|
| P3 Verdict in journal | orch verdict ledger keyed by SHA, worker self-report vs verifier override, verifier on another model family, prove-it-works levels |
| P1a Router engine + brief contract | poteto-mode (playbook index, steps as todos, `done` / `skip: <reason>`, cited principles), orchestrate brief template (SCOPE, CONTEXT, ACCEPTANCE, VERIFY), `docs/harness/claude-code.md` (one mapping per harness, `Skipped in <harness>` line) |
| P1b Router skill + coordinator rules | poteto-mode as the one front door, orchestrate coordinator rules |
| P2 Port 18 playbooks + 24 principles | poteto-mode playbooks + figure-it-out + `principle-*` skills |
| P5 CoS queue | orchestrate: completions are queue events, drain in batches |

P4 is folded into P1a: both change the brief headers in `brief_headers_ok` (`worker/prompt/mod.rs`), so two PRs would conflict on one function.

## Order and lanes

Prerequisites: H1 (one handback format) [F merged #95], H2 (harness registry) [F merged #96], H4 (review baseline: worktree HEAD + porcelain recorded at review spawn, `worker/prompt/readonly.rs:1-3`) [F merged #99].

| Lane | PR | Starts from | Waits for |
|---|---|---|---|
| A | P3 | main | nothing |
| B | P1a | main | P3 merged |
| C | P5 | main | P3 merged |
| B | P1b | main | P1a merged (the skill calls `quaqua playbook`) |
| D | P2a/b/c/d | main | P1a merged (the parser loads the playbooks); not P1b |

P3 goes first: it has the strongest evidence and the smallest surface, and every playbook's last Worker step writes `Verified: <level> <sha>` with P3's levels, so P1a and P2 use the real format from the start. P1a and P5 then run in parallel; both touch `status.rs` or the journal through P3's code, and the second to merge rebases. P1b touches templates only.

Risk to test first: the `## Playbook` check assumes workers reliably write the section with every step id. P1a ships it as a warning. Pilot it on one real pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker, then turn a missing section into exit 6 (§ P1a, Pilot). Report parsing has already produced false blocks: notes under `## Blockers` were read as real blockers (CoS log 2026-10-16 truonglao-intake, which cost a respawn; 2026-10-18 T1 bashclean [A: the 10-18 log is not on this machine]), and the 2026-10-22 blind-check advisor was flagged `malformed-report` for a missing `Verdict:` line although its report was complete. This is pstack's "pilot one unit through the whole path before fan-out" applied to P1a itself.

## SLP team

Every rule below that says "coordinator" applies to the CoS on no-SLP work and to the Lead (`<slug>-lead`) inside an SLP team (`templates/workspace/AGENTS.md` § Roles). anh iu stays Principal and talks only to the CoS.

| Concept | No SLP (CoS → worker) | SLP team (CoS → Lead → ≤4 Peers) |
|---|---|---|
| Route `/poteto-mode` | CoS picks the playbook per unit | CoS picks once for the Lead brief (`multi-phase-plan` or `figure-it-out` for project-size work); the Lead routes each Peer unit |
| Brief headers | CoS writes them | CoS writes the Lead brief; the Lead writes each Peer brief. `Context:` relay = an upstream Peer report pasted in full |
| `## Orchestrator` | CoS reads it | the Lead reads it |
| `## Worker` + `## Playbook` | worker | Peer |
| Verdict (P3) | per branch, before `quaqua merge` to main | per Peer branch before the Lead merges it into `quaqua/<slug>-integ`, and on `<slug>-integ` before the Lead reports done |
| `quaqua watch` + drain (P5) | CoS watches workers | the Lead watches its Peers; the CoS watches the Lead and `runs/<slug>-lead.inbox.md` |
| Never edits code or config | CoS | the Lead too: a conflict on `<slug>-integ` is rework for a Peer |
| Attack the premise | CoS | the Lead, who sees a fix class repeat on `<slug>-integ` first |

---

## P3 — Verdict in the journal

Goal: separate agent-done → verified → ship-ready, with verified tied to a SHA.
Builds on H4 [F merged #99]: H4 records the worktree HEAD + `git status --porcelain` at review spawn and voids a verdict whose tree changed (`worker/prompt/readonly.rs:1-3`). P3 reuses that recorded SHA as the verdict's `sha`; it does not compute its own. One place decides "stale".
Scope: `engine/journal_sqlite.rs` (new event kind, no schema change: `events.kind` + `payload_json`) [F], `pipeline/mod.rs` (Verdict parse, :2662 at 0fc0ad8) [F], `status.rs`, `merge/mod.rs`, the reviewer-kind check in config
Design (pstack orchestrate § Verification, § Roles; `principle-prove-it-works`) [F]:
- event `verdict {branch, sha, level, evidence, verifier, source: self|verifier}`; levels `unit-test-verified | type-check-only | review-clean | inconclusive | verifier-blocked | verifier-failed`
- a builder may self-report; a reviewer verdict on the same branch + SHA overrides it. `quaqua status` shows the strongest source for the current SHA. A self-report alone is never shown as verified.
- the verifier runs on another model family (pstack orchestrate § Roles). Today `strategy.builder_reviewer_differ` compares harness only and `kind` is not enforced (`rules/_all/config-dynamic.md`) [F]; a verdict from a reviewer of the builder's model family is recorded as `source: self`, never `verifier`.
- `inconclusive` and `verifier-blocked` are never a pass; `verifier-failed` calls for a fix unit, not a re-review
- CI green is evidence inside a verdict, not a verdict
- report line `Verified: <level>  <sha>`
- `quaqua status` shows the per-branch level; stale if head SHA ≠ verdict SHA (turns "new commit voids review" from prose into a check)
- ship-ready stays anh iu's approval; the engine only refuses to call a stale SHA "verified"
- `quaqua merge` prints the branch head's strongest verdict and warns when no verdict matches the head SHA, for merges to main and Lead merges into `<slug>-integ`. Today its check passes when no check command is set (`merge/mod.rs:157-165`) [F]. Evidence: PR #42 merged at `b12f74f` while the last CoS test run was at `a7de35b` (its handoff comment says so).
Acceptance: tests for record/read verdict, stale on new commit, status output, reviewer overrides self-report on the same SHA, same-family reviewer recorded as `self`, `inconclusive` not shown as verified, a verdict H4 voided is never recorded as verified, `quaqua merge` warns on a head SHA with no verdict
Verify: `for f in journal status pipeline merge; do cargo test --locked -p quaqua "$f"; done`

## P1a — Router engine + brief contract

Goal: the engine lists and shows playbooks, appends the worker's steps and cited principles to the brief it submits, and checks the report against the steps; briefs gain the headers a stranger needs to execute them.
Scope: `worker/prompt/mod.rs` (`brief_headers_ok`, :41; submit path), `worker/prompt/watchdog.rs` (`validate_report_content`, :57-75) and its callers (`workflow.rs:467`, `mod.rs:956`, `cost.rs:483,488,511`), `rules/_all/brief.md`, new `templates/workspace/playbooks/` (fixture playbook + `HARNESS.md`; real playbooks in P2), new `templates/workspace/principles/` (fixture leaf; real leaves in P2a), new `playbook` subcommand. Line numbers are from d24be71: re-locate them on current main.

Design, router (anh iu: data + CLI; the skill is P1b):
- Each playbook file starts with frontmatter `name:` + `when:` (one sentence, the routing trigger). The index is generated from these files, never hand-written, so it cannot drift from `playbooks/`. pstack writes its index by hand in `poteto-mode/SKILL.md` § Playbooks [F]; this is the one place quaqua departs from it.
- `quaqua playbook list` prints `<name> — <when>` per file. `quaqua playbook show <name>` prints the `## Orchestrator` section for the coordinator. Reuses the P1a parser.
- No playbook fits, or the work is large and cross-cutting → `figure-it-out` (ported in P2a), never a blank. pstack routes there for "no bundled playbook fits" [F]. `Playbook: none, <reason>` stays only for trivial one-command units.

Design, playbook contract:
- Brief header `Playbook: <name>`. `--check-brief` rejects unknown names; a missing header warns, as does `none` without a reason.
- Playbook file has `## Orchestrator` (coordinator) and `## Worker`; Worker steps are a numbered list, the number is the step id.
- Injection point is **`worker prompt`, not spawn**: spawn writes `runs/<name>.rules.md` before any brief exists (`spawn.rs:321-354`; `SpawnOpts` has no brief, `spawn.rs:43-49`) and each harness reads that file once at launch (`launch.rs:105,118-119,145-146`) [F]. `worker prompt` appends the `## Worker` section to the brief it submits. Never via `rules/_all` (`playbooks/` and `principles/` stay outside it; spawn's concat isn't recursive [F]).
- Cited principles: a Worker step names a principle as `principle-<name>`; `worker prompt` appends each cited leaf from `principles/<name>.md` once, after the steps. A leaf no step cites is never sent. pstack loads a principle only when the router names it ("Cite only principles whose leaf SKILL.md you read", `poteto-mode/SKILL.md`; every leaf has `disable-model-invocation: true`) [F]. An unknown principle name fails the P1a parser test.
- `worker prompt` persists the playbook name in `runs/.<name>.playbook` (like `.budget`), so `--resume` validates without the brief.
- Pre-filled block: `worker prompt` appends a `## Playbook` block with one line per step id (`1: `, `2: ` …) and tells the worker to copy it into its report and fill each line. The worker never invents the format.
- Report check, builder only (not reviewer: `expect_verdict = false`): the parser accepts `1: done`, `1. done`, `1) done` and `- 1: done`, and the same for `skip: <reason>`. The brief has a Playbook and the `## Playbook` section is missing → warning during the pilot, exit 6 (resumable, like today's Lessons check) after it. A missing step id or a `skip:` without a reason → always a warning plus a journal event `playbook_gap {name, missing, unreasoned}`, never exit 6.
- `validate_report_content` gains the expected step ids (`None` = no playbook = today's behavior); all callers pass it.

Pilot, before a missing section becomes exit 6:
- One pi worker (`opencode-go/deepseek-v4.1-flash`) and one claude worker, each running at least 5 real units on a fixture playbook in warn mode.
- Count per report: section present, `playbook_gap` events, and gaps that hid real unfinished work (the coordinator checks each gap against the diff).
- Exit 6 turns on only when the section is present in ≥ 9 of 10 reports on both kinds. Otherwise it stays a warning and the plan's Pre-mortem applies.

Design, harness skip mapping (from pstack `docs/harness/claude-code.md`: one mapping file per harness, and an unsupported step becomes one `Skipped in Claude Code: <step>. <reason>.` line, then the next step, no stop, no ask) [F]:
- A playbook step the worker's harness cannot do is reported as `<n>: skip: unsupported on <harness>, <reason>`; it is a valid skip.
- One short table per harness in `templates/workspace/playbooks/HARNESS.md`: step concepts (read-only check, run targeted test, open PR, headless UI) → the command or flag for claude / omp / opencode / pi. Row keys come from the H2 registry's harness names, so a new harness without a row shows up in a test. "open PR" replaces pstack's opening-a-pr playbook: quaqua workers do not open PRs, the coordinator does.
- `worker prompt` appends only the worker's own harness rows next to the `## Worker` steps.

Design, brief headers (pstack's 9-field brief: GOAL, SCOPE, CONTEXT, ACCEPTANCE, VERIFY, TIMEBOX, FORBIDDEN, REPORT, STANDING; quaqua already has Goal, Hard constraints ≈ FORBIDDEN, Evidence ≈ REPORT, `.budget` ≈ TIMEBOX) [F]:
- New headers, each warning, not failing. pstack refuses to spawn on a missing field; quaqua does not until the P2c eval shows a missing header caused a real failure.
  - `Scope:` owned / forbidden paths.
  - `Acceptance:` checkable lines, outcomes not a solution, so it keeps the no-pre-solve rule.
  - `Verify:` the exact command(s), plus known gotchas. `cargo test` takes one test filter per run (`runs/ni-h3a.md`), so a Verify line with several filters is one run per filter.
  - `Context:` file / PR pointers; when this unit depends on another, that unit's report pasted in full. Pasted text reaches every harness, so a dependency is a context relay, not just ordering.
- Size the brief to the unit: a one-command unit collapses to one paragraph that still names goal, scope, the verify command and the report shape. The context relay is exempt: an upstream report is pasted in full even into a one-paragraph brief.
- Later (not this PR): `herdr-gate.sh` blocks writes outside Scope.

Acceptance: tests for unknown playbook rejected, Worker-only text in the submitted prompt (no Orchestrator text), only cited principle leaves appended and each once, only the worker's own harness rows appended, pre-filled `## Playbook` block in the prompt, all four step-line formats parsed, missing section → warning in pilot mode and 6 after, missing step / `skip:` without reason → warning + `playbook_gap` event (never 6), `skip: unsupported on <harness>` accepted, `--resume` keeps the check, report without Playbook brief unchanged, brief missing Playbook / Scope / Acceptance / Verify / Context warns but passes, `playbook list` built from the fixture frontmatter, `playbook show` prints only the Orchestrator section, every H2 harness has a row in `HARNESS.md`
Verify: `for f in prompt watchdog playbook; do cargo test --locked -p quaqua "$f"; done`
Forbidden: auto-classifying the playbook in the engine (the coordinator picks; the engine only lists and checks)

## P1b — Router skill + coordinator rules (templates only)

Goal: one front door that routes every new task to a playbook before an agent is picked, and the coordinator rules the CoS and Leads follow; no engine change.
Scope: new `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, `templates/workspace/AGENTS.md`, `crates/quaqua/tests/init.rs` (the skill lands in a fresh workspace).

Design, skill `templates/workspace/.claude/skills/poteto-mode/SKILL.md`, the only routing entry:
- `/poteto-mode <goal>` from anh iu, or the coordinator on any new task. Steps: run `quaqua playbook list`, pick one name, run `quaqua playbook show <name>`, write the brief (all P1a headers), spawn per `models.md`.
- It is a front door, not a fence. Claude Code's `allowed-tools` pre-approves tools and does not restrict them, and the CoS runs with bypassed permissions, so nothing stops a routing turn from editing. Whether the coordinator routes is measured, not enforced: the P2c eval reports how briefs spread across playbooks.
- Never invent a playbook name; never route inside a worker. A worker that loads it stops: its playbook is already fixed in `runs/.<name>.playbook` and it follows only the `## Worker` steps in its prompt.
- pstack's other skills (how, architect, swarm, interrogate…) are not bundled; each playbook names the quaqua equivalent (P2 rules).
- `templates/workspace/AGENTS.md`: one line, "Before every brief, route through `/poteto-mode`."

Coordinator rules in `templates/workspace/AGENTS.md`, from pstack `orchestrate.md` and the principle leaves [F]. Each applies to the CoS and to a Lead (§ SLP team):
- Coordinator never edits code or config: it writes briefs, reads reports, verifies and merges. Edits to `crates/`, `tests/`, `rules/`, `AGENTS.md`, `team.toml` and `models.md` are a worker unit, except a Lesson promotion at cleanup. A conflicted merge or rebase is a worker unit, which `quaqua merge` already hands back as rework (exit 4, `merge/mod.rs:288-294`) [F]. Evidence: the CoS resolved the W3 merge conflict itself, which left 2 suites failing and leaked a real herdr spawn (CoS log 2026-10-18 [A: not on this machine]); a stray `git checkout origin/main~4` in the engine checkout (CoS log 2026-10-20); the CoS edited AGENTS.md/rules mid-brainstorm and spawned ahead (`memory/2026-10-20.md`) [F]. Later (not this PR): `herdr-gate.sh` denies coordinator edits under `crates/` and `tests/`.
- Attack the premise (`principle-attack-the-premise`): when a second fix in one class fails the same gate, stop patching. Write the premise down, and open a `figure-it-out` unit that tests it before any further fix. Evidence: grid layout, 19 fix commits, the self-anchor loop 3–4 times in one day.
- Encode lessons in structure (`principle-encode-lessons-in-structure`): when promoting a Lesson at cleanup, first try a test, hook, generator or parser change; write it to `rules/` only when the lesson needs judgment, and then with one example of the failure.
- Retry by mode: cap-hit / oom → respawn with smaller scope; network drop → retry as-is; tool error → retry on a different model; unknown → retry once. Two retries per unit, then abandon the unit and replan. This counts respawns of a unit; the existing kill criteria (same error ≥3 iterations inside one session, `AGENTS.md` § Roles) still decide when to stop a looping session.
- Stop line: when more spawning would produce garbage tree-wide (bad upstream output, broken acceptance, dead infra), write a stop line, let in-flight work finish, fix the cause, clear it.
- Budget: at about 70% of the wall-clock or cost budget, stop spawning and land what is verified. New line; `worker-timebox.md` has no 70% rule today.
- Pilot and liveness already live in `rules/_all/worker-timebox.md` (:11 pilots get 30 min, :9 never blind re-prompt, read the pane) [F]. AGENTS.md links there and adds only "on a new unit shape, pilot one unit through brief → worker → verify → merge before any fan-out", which binds a Lead fanning out to Peers too; it never restates a rule, because a restated rule drifts (`runs/tieude-h1drift.md`).
- Rollup: at each wave boundary, account for every spawned worker (arrived, respawned, or scope absorbed); never silently redo a missing worker's job.
- Resume vs fresh: unchanged from `AGENTS.md` § Roles: an interrupted worker resumes (`quaqua worker prompt --resume`); a fresh spawn only when the goal changed or the worktree is unrecoverable. A fresh spawn gets consolidated scope: original brief + later directives + prior report (the `.fixN` practice). pstack's "fresh subagents by default" (`poteto-mode/SKILL.md` § Subagents) is not adopted: resuming saved $0.14 + 3h33m on spawneff (2026-10-17).
- Never block on the human (`principle-never-block-on-the-human`), adapted: inside a unit, decide a reversible call, act, and report it. Irreversible actions (merge to main, push, branch delete, `herdr integration install`) and anh iu's goal, cost and priority calls stay questions for anh iu.

Acceptance: a fresh `quaqua init` workspace has `.claude/skills/poteto-mode/SKILL.md`; AGENTS.md has the route line and the coordinator rules, each stating it binds the CoS and a Lead; pilot and liveness appear as a link to `worker-timebox.md`, not as copied text
Verify: `cargo test --locked -p quaqua --test init`

## P2 — Port 18 playbooks + 24 principles

Goal: 18 playbooks in `templates/workspace/playbooks/` and 24 principle leaves in `templates/workspace/principles/`, all landed before eval is used:
- 11 of pstack's 23 files in `skills/poteto-mode/playbooks/` [F]: bug-fix, feature, refactoring, perf-issue, investigation, prototype, hillclimb, runtime-forensics, trace-forensics, visual-parity, eval.
- figure-it-out, from `skills/figure-it-out/SKILL.md` [F]: the router's target when nothing fits. Frame done as a falsifiable predicate, pick a rigor level, split into independently landable units riskiest-first, build the check before the work, run each unit as hypothesis → smallest change → measure → keep or revert, verdict `VERIFIED | NOT VERIFIED | INCONCLUSIVE` (inconclusive is not a pass). Ported in full; it does not hand planning to any other skill.
- session-pickup, from `playbooks/session-pickup.md` [F]: resume a prior worker or coordinator session from its report, branch and journal; the prior trail is authoritative, do not redo completed work; then route the rest to the matching playbook.
- pause-safely, from `playbooks/pause-safely.md` [F]: stop at a safe boundary, no irreversible action, one `wip:` commit, a resume note off-context (`runs/<name>.resume.md`). Triggered on an explicit pause or before a coordinator compaction (`principle-guard-the-context-window`), never on "keep going".
- multi-phase-plan, from `playbooks/multi-phase-plan.md` [F]: the plan is a checklist an owner runs box by box and the operator audits from the evidence; Orchestrator-only, no implementation. The router's target for project-size work and the Lead brief.
- worktree-cleanup, from `playbooks/worktree-cleanup.md` [F]: audit from `git worktree list`, never hand-typed paths; classify by merge state, uncommitted work and age; the bucket is advice, pinned or active work wins; remove through `quaqua worker cleanup`. Drop the iOS simulator half. Evidence: 3 live + 2 prunable worktrees at P2 M0 (`memory/2026-10-21.md`).
- shipping, from `playbooks/shipping.md` [F]: verify each PR independently, land only the contiguous verified run from the root. The Orchestrator side of P3; landing on main stays anh iu's approval. Evidence: PR #42.
- babysit, from `playbooks/babysit.md` [F]: declare a mode, clear one PR at a time, stop where the human's call begins. Pairs with P5 `watch`. Evidence: llmhub team idle ~14h (`memory/2026-10-20.md`).

Principles, from pstack `skills/principle-*/SKILL.md` [F]: all 24 leaves, one file each in `templates/workspace/principles/<name>.md`. Each keeps its rule, its Why and its When; drop TypeScript-only and Cursor references. A leaf is never concatenated into `rules/_all`; it reaches a worker only when a Worker step cites it (§ P1a). Where each one is cited:

| Principle | Cited by |
|---|---|
| attack-the-premise | bug-fix, figure-it-out; coordinator rule (P1b) |
| encode-lessons-in-structure | coordinator rule (P1b), eval |
| prove-it-works | the last Worker step of every playbook |
| sequence-verifiable-units | figure-it-out, multi-phase-plan, shipping |
| separate-before-serializing-shared-state | multi-phase-plan (splitting Peers), worktree-cleanup |
| make-operations-idempotent | feature, refactoring |
| fix-root-causes | bug-fix, runtime-forensics |
| build-the-lever | feature, refactoring, worktree-cleanup |
| guard-the-context-window | investigation, pause-safely |
| explain-the-number | perf-issue, hillclimb, eval |
| laziness-protocol, subtract-before-you-add, migrate-callers-then-delete-legacy-apis, minimize-reader-load | refactoring |
| redesign-from-first-principles, foundational-thinking, model-the-domain, boundary-discipline, type-system-discipline | feature |
| outcome-oriented-execution | multi-phase-plan |
| test-behavior-not-implementation | bug-fix, feature |
| exhaust-the-design-space, experience-first | prototype, visual-parity |
| never-block-on-the-human | coordinator rule (P1b), adapted to reversible work only |

pstack playbooks have no Orchestrator/Worker split: one `### Title`, a lead paragraph, numbered steps that mix lead and subagent work (`bug-fix.md`: "You own this task… Delegate investigation and the fix to subagents") [F]. Each port is an authored split, not a copy. session-pickup, pause-safely and multi-phase-plan may be Orchestrator-only (empty `## Worker`). Local source for the brief: a pstack checkout at 190d287 (`git clone https://github.com/cursor/plugins` → `pstack/skills/`).
Rules for each port:
- source only pstack: drop Cursor, Task-tool, `/loop` and every reference to a skill outside pstack
- arena → N workers of different kinds + judge; architect → taskforce `dual-lens`; interrogate → reviewer on a different model
- visual-parity runs headless only (`no-gui-driving.md`) [F]
- eval reads `runs/` reports + telemetry, not `~/.cursor/projects/`; it reports how briefs spread across playbooks (every name, plus `none` and `figure-it-out`), the `playbook_gap` events, and briefs with a missing header that ended in a failed or respawned unit
- the last `## Worker` step of every playbook (except Orchestrator-only ones) is "check every Acceptance line, then write `Verified: <level> <sha>`" with the P3 levels, citing `principle-prove-it-works`
- each port carries frontmatter `name:` + `when:`; the `when:` line is adapted from the playbook's one-line entry in pstack's `poteto-mode/SKILL.md` § Playbooks
Split into 4 PRs, so each review stays small:
- P2a — code + fallback + principles: bug-fix, feature, refactoring, perf-issue, figure-it-out, multi-phase-plan, and all 24 principle leaves
- P2b — explore + lifecycle: investigation, prototype, hillclimb, session-pickup, pause-safely
- P2c — forensics + eval: runtime-forensics, trace-forensics, visual-parity, eval
- P2d — landing + hygiene: shipping, babysit, worktree-cleanup
P2b, P2c and P2d cite principle leaves, so they merge after P2a; they touch disjoint files and can be built in parallel.
Acceptance: each file passes the P1a parser; `quaqua playbook list` shows all 18; every cited `principle-<name>` resolves to a leaf; every non-empty `## Worker` ends with the `Verified:` step; `grep -riE "cursor|Task tool|/loop|brainstorm|to-plan"` in playbooks/ and principles/ returns nothing
Verify: P1a test fixture loads every playbook and principle in the directory

## P5 — CoS queue via `quaqua watch`

Goal: worker completions/failures/blockers land in one queue; the coordinator drains it at boundaries instead of waiting on each agent.
Already on main [F]: `quaqua status` with `DONE-NOT-CLEANED` / `IDLE-UNPROMPTED` / `ORPHAN` (`status.rs:50-52`) and `--metrics` (#93); the per-worker watchdog blocks on `herdr agent wait` (`herdr.rs` `agent_wait`, #90). Missing [F]: no `watch` subcommand exists.
One design, not two: `quaqua watch` is P1 of `team-event-driven-monitor.md` (:19, :26; #47, draft) [F]. P5 builds it once and adds only pstack's drain; the same PR marks that plan's P1 as superseded by P5.
herdr contract (from the evtprobe run, `~/quaqua/raw/2026-10-17-tieude-evtprobe.md`, workspace-private, 8/8 transitions delivered, latency ≤1.6s):
- subscriptions are per pane, no wildcard, no snapshot on connect, one subscribe per connection → pin the pane set, read state once at connect, reconnect and re-sync when topology changes (`pane.agent_detected`)
- `idle` at the end of a turn with a background shell is a real idle → a status event is never proof of completion; `runs/<name>.md` stays the source of truth
Design:
- `quaqua watch <names…> [--timeout]` reuses `Herdr::agent_wait` per name (one thread each) [A: `herdr agent wait` takes one name]. On `idle | done | blocked` or pane exit it checks `runs/<name>.md`, appends a journal event `worker_signal {name, status, report: present|missing}`, prints one line and exits. The coordinator runs it in the background and is woken instead of sleep-polling. A Lead watches its Peers; the CoS watches Leads.
- Drain discipline (pstack orchestrate § Queue and drain) [F]: a signal is recorded and the coordinator returns to what it was doing; it never reviews a diff inside a drain (a review is its own reviewer unit). The coordinator drains at four points: after a critical section (writing a brief, a merge decision), at a wave boundary, on a `watch` wake, and before reporting to anh iu.
- `quaqua status --drain` lists undrained signals, marks them drained, and ends with two lines: counts per state, and what changed since the last drain.
Acceptance: replayed event fixture → correct one-line output and journal event; idle with no report → `report: missing`, not done; socket drop → reconnect + re-sync; `--drain` output ends with the two lines
Verify: `for f in watch status; do cargo test --locked -p quaqua "$f"; done`; one live run with 2 workers
Forbidden: a daemon or scheduler beyond `watch`
Later (own PR, after `watch` has run live): gates. A question for anh iu is a journal event `gate {question, options, default}`, shown in the drain's last line and batched into the next report; irreversible actions are always gates with no default. The one incident (a Lead prompt lost while the CoS waited) is already covered by `rules/_all/lead-inbox.md`.

## Out of scope

Failures the second pass found that no pstack concept here covers. Listed so they are not lost; each needs its own plan.
- Cost overruns: llmhub `$74.55` against a `$30` cap, sude `$71.3` (`memory/2026-10-20.md`); nothing enforces `Cost budget`.
- Team-level idle: llmhub idle ~14h (`memory/2026-10-20.md`); liveness is per agent. babysit (P2d) helps only while a coordinator is running.
- herdr `done` ≠ success: an API 400 ends `working → done`.
- Merge gates that run suites separately hide cross-module failures: arch W2 107/3 combined vs 31/31 pipeline alone (`memory/2026-10-20.md`).
- Lead inbox timestamps are not wall clock (`memory/2026-10-20.md` retro).

## Audit — does quaqua have the failure each concept fixes?

Method (2026-10-22): pstack @ 190d287 read from source; quaqua evidence from `~/quaqua/runs` and `memory/`, `git log --no-merges origin/docs/pstack-plan-v3` in the engine, `rules/_all/` and the code. Counts are files or commits matching the filter in § Filters, not incidents and not cost: a phrase scan misses incidents written in other words and counts files that only mention the phrase. Two passes: one Claude pass (v3.1), and a non-Claude pass (`runs/suhuynh-pstackfit.md`, opencode deepseek-v4.1-flash), which was not fully blind: a session restart showed it the table before it wrote its verdicts, so only its counts are independent. v3.1 counts that no filter reproduces are marked `unverified`.

| Concept | Count (filter) | Verdict | Where it lands / evidence |
|---|---|---|---|
| Router skill | n/a: there is no router yet to fail | Keep, as front door only | P1b; measured by the P2c eval, not enforced |
| Steps as todos, `skip:` | 1 false-done report (F1); v3.1's ~10 unverified | Keep, check warns through the pilot | P1a; exit 6 for a missing section only after the pilot |
| figure-it-out | 0 | Keep as fallback, ported in full | P2a |
| Principles (24) | 19 grid fix commits (C3), 54 reports on drift / two copies (F2) | Keep, as cited leaves | P2a leaves; P1b coordinator rules for attack-the-premise, encode-lessons, never-block |
| 9-field brief, refuse to spawn | 1 unclear-brief report (F3); v3.1's ~7 unverified | Adapt, warnings; refuse-to-spawn dropped | P1a: 4 headers; required only on eval evidence |
| Context relay | 7 reports on an unreadable brief path (F4); the two v3.1 citations (`runs/suhuynh-r0r1.md`, `runs/tieude-r0.fix1.md`) are absent | Keep | P1a `Context:` |
| Standing orders | handled | Exists | spawn concatenates `rules/_all` into every `runs/<name>.rules.md` (`worker/spawn.rs:322-355`) |
| Coordinator never edits code or config | 2 found of 3 cited (`memory/2026-10-20.md` ×2; 10-18 log not on this machine) | Keep, new | P1b |
| Queue and drain | 0 found; v3.1's "5 prompts in one CoS session" unverified (no transcript) | Keep, gates later | P5 (`team-event-driven-monitor.md:7`) |
| SHA verdict, verifier overrides | PR #42; stale-verdict count unverified | Keep, new, first | P3 + merge warning |
| Verifier on another model family | config compares harness only (`rules/_all/config-dynamic.md`) | Adapt | P3: same-family reviewer recorded as `self` |
| Liveness | 6 reports on false exits / stalls (F5); v3.1's 13 unverified | Exists | `worker-timebox.md:9`; event watchdog (f65c479) |
| Retry by mode, stop line | ~4 (78-min stall, lead cap hit twice, hermpin cancelled by anh iu), unverified | Adapt, prose rule | P1b; pipeline already caps rounds (`pipeline/mod.rs:2082`) |
| Pilot | 1 (srt pilot) | Exists | `worker-timebox.md:11`; P1b links it |
| 70% budget | — | Adapt, new line | P1b |
| Fresh over resumed | spawneff resume saved $0.14 + 3h33m | Not adopted | `AGENTS.md` § Roles resume rule stays; P1b names consolidated scope for a fresh spawn |
| Gates, irreversible pause | 1 (Lead prompt lost while the CoS waited) | Later | P5 follow-up; `lead-inbox.md` |
| Session pickup, pause safely | 0 | Keep | P2b, from pstack |
| multi-phase-plan | 35 reports mention phases (F6) | Keep | P2a; the Lead brief's playbook |
| worktree-cleanup | 2 reports on stale / prunable worktrees (F7) | Keep | P2d |
| shipping | PR #42 | Keep | P2d, Orchestrator side of P3 |
| babysit | 5 reports on long idle (F8) | Keep | P2d, with P5 |
| autonomous-run | conflicts with anh iu's goal/cost calls | Not ported | figure-it-out already defines done first |
| autopilot-full | 0; removes anh iu's merge gate | Not ported | — |
| autopilot-stack | 0 stacked-PR reports | Not ported | SLP `<slug>-integ` plays this role |
| opening-a-pr | workers do not open PRs | Not ported | one `HARNESS.md` row (P1a) |
| Harness adapter | 16 harness fix commits (C4) | Keep | P1a `HARNESS.md`, built from the existing per-harness rules (`opencode-no-bg-wait.md`, `herdr-plugin-smoke.md`) |
| Model per role | 3 fallback fix commits (C5) | Exists | `launch.rs:19-71` fails closed per kind |
| Durable orch store | handled for the journal | Exists for the journal; units / ledger tree not ported | event journal `runs/quaqua.db` |

Filters (run 2026-10-22; R = `~/quaqua/runs/*.md` minus `.rules.md`, `.brief.md`, `.resume.md` = 362 reports, 274 with `## Lessons`; engine = `git log --no-merges origin/docs/pstack-plan-v3`, 194 commits, 81 matching `^(fix|hotfix|bugfix)`):
- F1 `grep -liE 'claimed done|reported done but|false done|not actually done|marked done but'` R
- F2 `grep -liE 'two copies|drift(ed)?\b|duplicated rule|stale copy'` R
- F3 `grep -liE 'brief (was )?(unclear|ambiguous)|ambiguous brief|unclear brief'` R
- F4 `grep -liE 'could not read the brief|brief path'` R
- F5 `grep -liE 'false exit|exited early|premature exit|false stall'` R
- F6 `grep -liE 'multi-phase|phase [0-9]'` R
- F7 `grep -liE 'prunable|stale worktree'` R
- F8 `grep -liE 'idle ~?[0-9]+h|idle for'` R
- C3 `git log --no-merges -iE --grep='^fix.*(grid|reconcile|anchor)'`
- C4 `git log --no-merges -iE --grep='^fix.*(opencode|omp|pi\b|claude|harness|launch)'`
- C5 `git log --no-merges -iE --grep='^fix.*fallback'`

Where the two passes still disagree (not resolved here): the second pass would port 5–6 playbooks, not 18, and one principle, not 24; it rates P5 drain as deferrable; it rates figure-it-out as overlapping existing planning. The decisions above were taken by anh iu with both passes in hand.

Counterfactual, the costliest failures with P1a–P5 in place:
- Grid layout (19 fix commits, the self-anchor loop 3–4 times in one day): the P1b attack-the-premise rule stops patching at the second failed fix in the class and opens a figure-it-out unit.
- W2 cutover (7 review findings, a second review round): no P-step catches it; it was caught by review, which quaqua already has.
- PR #42 merged without tests at its head SHA: P3's merge warning catches it.
- Coordinator resolves a conflict and breaks main: P1b's rule names it; only the later gate hook would stop it.
- `## Blockers` false block → respawn: no P-step; P1a adds parse surface of the same kind, which is why its check starts as a warning, parses four line formats, and is pre-filled.

Pre-mortem: in six months the `## Playbook` exit 6 and the four new headers have made more false blocks and resumes than the false-done reports they caught. Watch the ratio of exit-6 resumes to `playbook_gap` events where a missing step hid real unfinished work; if resumes win, turn exit 6 back into a warning.
