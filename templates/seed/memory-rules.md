# CoS rules (always loaded)

Durable lessons from anh iu. One rule per bullet: rule — why (date).
Pane layout, herdr mechanics and the watchdog live in `scripts/` + `rules/_all/`.

<!-- Add rules here. Cap: 2200 chars (AGENTS.md '## Memory'). Over cap, merge
     or drop the oldest entry first — never raise the cap. -->

- Advisor workers (suhuynh) only advise, never edit code; their findings go to a build worker (2026-09-30).
- Clean up each worker (pane + worktree) right after its run is verified and its tests pass; keep the branch — branch delete/push/merge need approval (2026-10-05).
- Spawn checks — a `wait` timeout or `agent_not_found` is not failure: confirm with `herdr agent list` + `herdr pane read` before respawning (2026-10-01).
- Ownership — orchestrators work only in their own repo; this workspace is CoS-only. Check `git worktree list` before spawning; one file, one branch (2026-10-01).
- Gardener — a bug class found twice becomes a `rules/<slug>/` rule or test; a workaround used twice becomes a script (2026-10-06).
- Context pointers — prompts point to spec §, rules, runs, commits; never restate them. Keep verification output short (`| tail -3`).
- Paths in committed files: `~/` or repo-relative, never `/home/<user>/` (2026-10-08).
