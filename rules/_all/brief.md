# Worker briefs

Every worker brief has these five parts, in this order (the first three are machine-checked by `quaqua worker prompt --check-brief`):

```
Goal: <what must be true at the end>
Hard constraints: <non-negotiables: what not to touch, hard limits; targeted tests only (`<cmd> <filter>`), foreground, no `pgrep -f` wait loops, `--slow` is the Lead's gate>
Current design (open to challenge): <the approach as it stands>
Unknowns: <what is not known yet + how to verify it>
Evidence: <what to return: command + result, files, decisions>
Time budget: <N> min
Cost budget: $<x>
```

- `Time budget` and `Cost budget` are the two caps; the time one is machine-read (`rules/_all/worker-timebox.md`), the cost one is recorded at acceptance in the project card.
- Mechanical task (≤2 files, no shared code path)? Skip the `Current design` paragraph and the plan file; Goal + Evidence is the whole contract.

- No pre-solve: a brief never fixes the cause or the solution — the worker finds it.
- `Current design` is a hypothesis, not an answer: deliver the goal within the constraints, not the design.
- `Unknowns` names what to check before building; `Evidence` is the proof the worker returns.
- Keep briefs short and technical. No mention of AI, and no representing anyone.

## Challenge
`## Challenge` (premise + evidence) in `runs/<name>.md` — the Lead (CoS on no-SLP work) resolves it before acceptance. A right, not a duty.

## Format (machine-read — get this exactly right)
- Headings are exactly `## Blockers` and `## Lessons` — never numbered (`## 5. Lessons` fails the watchdog as malformed, exit 6).
- `## Blockers` body: only real blockers. When there are none, the body must be exactly `none` — any sentence there is read as an unfinished-work blocker and the pipeline stops.
- `## Lessons` is the last section (≤2 lines or `none`).

`runs/<name>.md` ends with `## Lessons` — ≤2 lines of durable lessons worth a `rules/` entry, or `none`. CoS promotes 0–1 at cleanup. (Pipeline reviewers end with the `Verdict:` line their prompt requires instead.)

Publish atomically: write `runs/.<name>.tmp` then `mv` it to `runs/<name>.md` — the pipeline reads the file the moment it exists; a partial file is a corrupted report. Stale reports from earlier tasks are archived, never consumed.
- Lead/CoS acceptance conditions go in the review or fix brief, never appended to the plan file after plan-ready (BUILD ignores them) — 2026-10-14, runs/mu-lead.md
