# Recovery

Always look first: `herdr agent get <name>` then `herdr agent read <name> --source recent-unwrapped --lines 80`.

| Signal | Meaning | Do |
|--------|---------|----|
| `blocked` | approval/question UI on screen | Safe in-worktree choice → `herdr agent send-keys <name> <key>`. Destructive/outward (push, delete, network publish) → ask anh iu. |
| `timeout` / `agent_prompt_stalled` | prompt may or may not have landed | Never re-prompt blind. Text sitting unsubmitted → `send-keys <name> enter`. Still working → wait again. |
| `unknown` (codex) | herdr can't tell turn end | Check whether `runs/<name>.md` exists; if yes, treat as done. |
| `agent_not_ready` at start | startup dialog (e.g. codex "Trust this folder") | `herdr pane read <pane> --source visible`, answer, then `herdr agent wait <name> --until idle`. |
| `runs/<name>.md` missing but agent idle | worker skipped the handoff | Prompt once: "Write the summary file now; reply only the path." |

Long answer not visible in read → ask the worker to write it to a file and reply with the path.
