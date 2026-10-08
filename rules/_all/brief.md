# Worker briefs

Every worker brief has these five parts, in this order (the first three are machine-checked by `scripts/prompt-worker.sh`):

```
Goal: <what must be true at the end>
Hard constraints: <non-negotiables: what not to touch, hard limits>
Current design (open to challenge): <the approach as it stands>
Unknowns: <what is not known yet + how to verify it>
Evidence: <what to return: command + result, files, decisions>
```

- No pre-solve: a brief never fixes the cause or the solution — the worker finds it.
- `Current design` is a hypothesis, not an answer: deliver the goal within the constraints, not the design.
- `Unknowns` names what to check before building; `Evidence` is the proof the worker returns.
- Keep briefs short and technical. No mention of AI, and no representing anyone.

## Challenge
`## Challenge` (premise + evidence) in `runs/<name>.md` — the Lead (CoS on no-SLP work) resolves it before acceptance. A right, not a duty.

`runs/<name>.md` ends with `## Lessons` — ≤2 lines of durable lessons worth a `rules/` entry, or `none`. CoS promotes 0–1 at cleanup.
