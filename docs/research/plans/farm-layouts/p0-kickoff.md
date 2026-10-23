# Farm layouts — P0 kickoff brief

Paste into a fresh session at the engine repo root: `Read docs/research/plans/farm-layouts/p0-kickoff.md and execute it.`

Goal: ship P0 — identity + depth plumbing (design.md rollout table) plus the CoS marker and pane protection (plan.md addendum) — on branch `quaqua/farm-p0` from latest `main`.
Topology: solo — one phase, one owner area; P1+ go to a Lead.

## Read first
1. [`README.md`](README.md) — decisions D1/D3/D4.
2. [`plan.md`](plan.md) — the P0 addendum (hand-tested herdr 0.9.5 commands, sidebar chip).
3. [`design.md`](design.md) — the P0 row of the rollout table and its proving test.

## P0 must
1. Design P0: `lead spawn` writes `runs/<slug>-lead.owner`; `identity::resolve` is positive (fake pane tokens worker / orchestrator / cos / none → peer / lead / cos / unknown); the depth walk stops at a cycle. No behaviour change: identity is only computed and logged.
2. `quaqua cos tag [--pane <id>]` (default: current pane) sets the token, pane label and display name; idempotent, safe to re-run at every CoS session start (tokens may not survive a herdr restart). Pane id comes first; herdr rejects the `--opt=value` form.
3. Token key: the hand test used `role=cos`, D3 said `grid_role=cos`. Check what `grid reconcile` does with an unknown `grid_role` value before reusing that key; record the choice and the reason in the report.
4. Protection: `worker cleanup`, `grid reconcile` and the PreToolUse `gate/` refuse to close or move a pane carrying the CoS token, with a clear error. A manual close from the herdr UI cannot be blocked; document `claude --resume` as the recovery.
5. Ship the sidebar chip as an opt-in template plus a `cos tag` hint; never edit the user's herdr config silently.

## Hard constraints
- P0 only: no layout manifest, gate enforcement, mail or `farm_events` (P1+).
- No push, PR or merge to `main`; stop at a committed branch and the report.
- Do not change worker kind, model or effort (`team.toml`, `models.md`).
- Hermetic tests: stub herdr, use the test-isolation env (`env -i`); a stubbed git must pin the worktree HEAD.
- Delete with `trash`, not `rm`.

## Evidence
- `cargo test --workspace` green, with new tests for: resolve over the four token cases, the depth-walk cycle, `cos tag` idempotency, and the cleanup / reconcile / gate refusals.
- `actionlint` clean if workflows changed.
- Real herdr pane: `quaqua cos tag` run twice, then `worker cleanup` on that pane refused — paste the output.

Time budget: 180 min. Cost budget: $10.

## Report
`REPORT.md` at the repo root, uncommitted: token key decision, files changed, commands run with output, open risks, `## Lessons`, last line `Verdict: PASS|FAIL`.
