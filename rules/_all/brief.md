# Worker briefs — the task contract

A brief is a contract: fixed before the build and graded by commands, not by anyone's word. `scripts/worker/prompt-worker.sh --check-brief` and `scripts/lead/pipeline.sh --brief` reject a brief that lacks Goal, Hard constraints, Current design, Owned scope, or at least one acceptance line.

```
Task: <slug> · Source: projects/<slug>.md <item>
Disposition: Engineer | Architect | Reviewer | Scout
Goal: <what must be true at the end>
Acceptance:
  - A1: `<command>` → <expected result>
Owned scope: <paths the worker may write> · Read-only: <paths>
Hard constraints: <non-negotiables: what not to touch, hard limits>
Current design (open to challenge): <the approach as it stands>
Unknowns: <what is not known yet + how to verify it>
Time budget: <N> min
```

- Acceptance is the Lead's (CoS on no-SLP work) to write, before the build. Each A-id runs from the worktree root with no prior `cd`, alias, or env var. A worker may challenge an A-id (`## Challenge`) but never edits or loosens it.
- Owned scope: a glob (`src/*.rs`), a directory (`src/`), or a file, comma separated. The pipeline blocks a change outside it.
- No pre-solve: a brief never fixes the cause or the solution — the worker finds it. `Current design` is a hypothesis, not an answer.
- Keep briefs short and technical. No mention of AI, and no representing anyone.

## Handback and review

The worker's `runs/<name>.md` is the PEER.md handback: Outcome, Snapshot, Scope, Acceptance (each A-id verbatim, real output, pass|fail), Verification, Unknown / risk, Ownership. Pipeline reviewers write `## Spec` and `## Standards` findings as `- [blocking|nit] <path:line> — <claim> — <evidence>` and end with `Verdict: clean` (zero blocking) or `Verdict: <N> findings (<B> blocking)`; only blocking findings start a fix round.

Before any `clean`, the pipeline re-runs every A-id on the frozen tip and checks Owned scope, writing `## Acceptance` into `runs/pipeline-<task>.md`. The Lead still re-checks before merging; the Auditor grades that section.

## Challenge
`## Challenge` (premise + evidence) in `runs/<name>.md` — the Lead (CoS on no-SLP work) resolves it before acceptance. A right, not a duty.

`runs/<name>.md` ends with `## Lessons` — ≤2 lines of durable lessons worth a `rules/` entry, or `none`. CoS promotes 0–1 at cleanup. (Pipeline reviewers end with the `Verdict:` line their prompt requires instead.)

Publish atomically: write `runs/.<name>.tmp` then `mv` it to `runs/<name>.md` — the pipeline reads the file the moment it exists; a partial file is a corrupted report. Stale reports from earlier tasks are archived, never consumed.
