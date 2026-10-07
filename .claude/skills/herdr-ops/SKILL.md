---
name: herdr-ops
description: Herdr command recipes for spawning, prompting, reading, unblocking and closing worker agents. Use before any herdr pane/agent/tab command. Rules live in herdr.md.
---

# Herdr ops

## Spawn a worker
```bash
herdr pane split --current --direction right --cwd <project-path> --no-focus   # read .result.pane.pane_id
herdr agent start <name> --kind <kind> --pane <pane_id> -- <launch args from models.md>                     # name: [a-z][a-z0-9_-]{0,31}
```


## Drive it
```bash
herdr agent prompt <name> "<task, written as anh>" --wait --timeout 900000
herdr agent read <name> --source recent-unwrapped --lines 150
herdr agent wait <name> --until blocked --timeout 900000
herdr agent list
```
- `prompt --wait` can return `idle` before the reply renders: run `agent wait <name>` again before `agent read`.
- Close a worker pane: `herdr pane close <pane_id>`.
- On `timeout` / `agent_prompt_stalled`: `agent get` + `agent read` before anything. Never resend blindly.
- On `blocked`: read the UI, decide per AGENTS.md (Core), reply with `agent prompt` or `agent send-keys`.
- `blocked` after a provider/stream error (`agent prompt` returns `agent_blocked`): `herdr pane send-text <pane_id> "<msg>"` then `herdr pane send-keys <pane_id> Enter`.
- Long output not visible: ask the worker to write it to a temp .md and reply with the path.


## Installed (2026-09-28)
- Integrations: claude, codex (session restore only; state via screen), omp, opencode (lifecycle authority).
- Plugin `persiyanov.reviewr` @v0.39.0: diff review sidebar; `herdr plugin action invoke` with `toggle`/`open`/`close`. Auto-opens on worktree create.
- Community plugins: GitHub topic `herdr-plugin`. Review manifest + scripts, pin `--ref` before installing.
