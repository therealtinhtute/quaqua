# Worker time budget & watchdog

- Briefs carry a time budget (state it as `Time budget: <N> min` inside the brief); CoS sets `PROMPT_WORKER_BUDGET_SEC` to match when launching.
- `prompt-worker.sh` watches the worker while it waits for `runs/<name>.md`. Exit codes: `0` run file printed · `2` hard cap `PROMPT_WORKER_MAX_SEC` (7200) · `3` stall — no pane/worktree change for `PROMPT_WORKER_STALL_SEC` (600), agent idle/done for `PROMPT_WORKER_IDLE_SEC` (120), or agent gone · `4` time budget `PROMPT_WORKER_BUDGET_SEC` (2700) exceeded · `5` blocked on a question/permission.
- At 80% of the budget the script sends **one** soft nudge (never again, never kills): wrap up, commit, write `runs/<name>.md` with a `## Blockers` section for what is left. When you see that nudge, do exactly that.
- On exit 3–5 CoS runs `herdr agent read <name> --source recent-unwrapped --lines 80` (or `--source visible` when `recent*` returns agent_not_idle), acts on what it sees, then `prompt-worker.sh --resume <name>` continues the wait. It never blind re-prompts. `blocked` can be stale — read the pane first; if it is working, just wait again.
- Long waits with no output are fine — the stall clock only measures *observable progress* (pane content or worktree diff). Keep working and it stays quiet.
