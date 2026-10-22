# Worker briefs

Every worker brief starts with these two headers, then five parts, in this order (the headers are the leading lines, up to the first blank line; the three `Goal:`/`Hard constraints:`/`Current design` lines are machine-checked by `quaqua worker prompt --check-brief`):

```
Playbook: <name from `quaqua playbook list`, or `none, <reason>` for a trivial one-command unit>
Context: <files / PR pointers; when this unit depends on another, that unit's report pasted in full>
Goal: <what must be true at the end>
Hard constraints: <non-negotiables: owned and forbidden paths, hard limits; targeted tests only (`<cmd> <filter>`), foreground, no `pgrep -f` wait loops, `--slow` is the Lead's gate>
Current design (open to challenge): <the approach as it stands>
Unknowns: <what is not known yet + how to verify it>
Evidence: <checkable lines: the exact verify command (one filter per run) and its real output, files, decisions>
Time budget: <N> min
Cost budget: $<x>
```

- `Playbook:` and `Context:` warn when missing, on a `none` without a reason, or on an unfilled `<…>` placeholder; with the workspace's `playbooks/` and `principles/` index present, a playbook name that index cannot resolve is rejected (an absent or incomplete index only warns). No playbook fits, or the work is large and cross-cutting → `figure-it-out`, never a blank.
- `worker prompt` appends the playbook's `## Worker` steps and each cited leaf to the submitted prompt, and pins the name + step ids in `runs/.<name>.playbook` once the prompt has landed; a missing pin means no check. The `## Orchestrator` section is the coordinator's and never reaches the worker.
- `Context:` is a relay: an upstream unit's report pasted in full is exempt from brief sizing. Only the header block counts — a `Playbook:` line inside pasted text is prose, not this brief's header.

- `Time budget` and `Cost budget` are the two caps; the time one is machine-read (`rules/_all/worker-timebox.md`), the cost one is recorded at acceptance in the project card.
- Mechanical task (≤2 files, no shared code path)? Skip the `Current design` paragraph and the plan file; Goal + Evidence is the whole contract.

- No pre-solve: a brief never fixes the cause or the solution — the worker finds it.
- `Current design` is a hypothesis, not an answer: deliver the goal within the constraints, not the design.
- `Unknowns` names what to check before building; `Evidence` is the proof the worker returns.
- Keep briefs short and technical. No mention of AI, and no representing anyone.

## Challenge
`## Challenge` (premise + evidence) in `runs/<name>.md` — the Lead (CoS on no-SLP work) resolves it before acceptance. A right, not a duty.

## Report format (machine-read — get this exactly right)

Write the report to the path the brief gives (`runs/<name>.md`). One format, six fields first,
then the machine-read headings:

```
Outcome:         COMPLETE | REOPEN_REQUEST | DEPENDENCY_REQUEST | BLOCKED
Snapshot:        SHA + branch + worktree path (omit if nothing was written)
Scope:           files changed / read, concrete paths
Verification:    commands run + REAL output, and what you deliberately skipped
Unknown / risk:  assumptions you rely on, decisions that need the Human
Ownership:       scope released, or still held and why

## Blockers
none

## Lessons
none
```

- `Outcome:` starts its line at column 1, no list marker: the pipeline reads it to tell an unfinished phase (`BLOCKED`/`PARTIAL`/`INCOMPLETE`/`FAILED`) from a finished one, and a builder that changed nothing says `Outcome: no changes`.
- `## Blockers` body: write `none` when there are none. Any other non-empty line is read as an unfinished-work blocker and the pipeline stops.
- `## Lessons` may be numbered (`## 5. Lessons` passes) but must be an `##` heading (`###` fails). The watchdog checks that the heading exists, not where it sits; keep it last so the machine-read tail stays one block.
- Pipeline reviewers end with exactly one `Verdict: clean` or `Verdict: <N> findings` line instead of `## Lessons`.
- Brief names a playbook? The submitted prompt ends with a pre-filled `## Playbook` block — copy it into the report and mark every step id `1: done` or `1: skip: <reason>`.
- Put any extra section before `## Blockers`, never after `## Lessons`, so the machine-read tail stays one block.

`runs/<name>.md` ends with `## Lessons` — ≤2 lines of durable lessons worth a `rules/` entry, or `none`. CoS promotes 0–1 at cleanup.

Publish atomically: write `runs/.<name>.tmp` then `mv` it to `runs/<name>.md` — the pipeline reads the file the moment it exists; a partial file is a corrupted report. Stale reports from earlier tasks are archived, never consumed.
- Lead/CoS acceptance conditions go in the review or fix brief, never appended to the plan file after plan-ready (BUILD ignores them) — 2026-10-14, runs/mu-lead.md
