# pi harness — move the opencode kinds to pi

Source: review of #92, 2026-10-21 (quaqua @ c32174b, pi 1.3.0).
Status: **draft spec** · design only, nothing built until anh iu approves. [F] = verified in repo/CLI, [A] = assumption the PR must confirm first.

This is `harness-hygiene.md` H3. It needs H4a (harness registry) merged first.

Locked decisions (anh iu): `tieude` + `suhuynh` move to `harness = "pi"`, same model `opencode-go/deepseek-v4.1-flash`, effort `max` (`--thinking max`) · `herdr integration install pi` is done · suhuynh stays default reviewer (same model as tieude = accepted risk) · nothing built before approval.

## Facts

- `pi --version` → `1.3.0` [F]. `herdr agent start --kind` accepts `pi` [F]. `herdr integration status` → `pi: current (v9)` (`~/.pi/agent/extensions/herdr-agent-state.ts`) [F].
- `pi --list-models` lists `opencode-go  deepseek-v4.1-flash` [F]. `--thinking` takes `off, minimal, low, medium, high, xhigh, max` [F].
- Flags [F, `pi --help`]: `--model provider/id[:thinking]`, `--append-system-prompt <text|file>` (repeatable), `--session-dir <dir>`, `--tools <allowlist>`, `--no-tools`, `--no-extensions` (explicit `-e` still loads), `--no-approve` (ignore project-local files), `--no-context-files`. No tool-approval prompt flag exists (omp needs `--auto-approve`, `launch.rs:117`; pi has none).
- Session store [F]: `~/.pi/agent/sessions/--<abs path, "/"→"-">--/*.jsonl`, e.g. `$HOME/.wt/tieude-omp` → `--<home path dashed>-.wt-tieude-omp--` (`$HOME` kept, not stripped). omp's is `-<path after $HOME>` (`cost.rs:49-67`, `-.wt-sude-x`). **Dir naming differs**, so `omp_cost_in` (`cost.rs:101`) cannot read pi as is.
- Cost line [F]: pi assistant lines carry `message.usage.cost.total` (`{"input":33287,…,"cost":{…,"total":0.0036167}}`), the pointer `session_line_cost` reads first (`cost.rs:73-78`). The **line parser works unchanged**; only the directory differs.
- ~/.pi/agent holds personal extensions (`pi-subagents`, `tmux-subagents`, `memory`, MCP config) [F, `ls ~/.pi/agent`]. A worker that loads them can spawn its own subagents outside the grid.

## Sites that change (today's opencode/omp string checks)

| Site | Today | pi needs |
|---|---|---|
| `launch.rs:20`, `config.rs:490` | closed list `opencode, omp, claude` | add `pi` (via H4a registry) |
| `config.rs:208-219` `effort_allowed` | per-harness set | pi set = pi `--thinking` list |
| `launch.rs:97-151` `build_launch_cmd` | one arm per harness | new `pi` arm |
| `worker/stop.rs:21-25` | opencode `esc esc`, omp `esc`, else refuse | pi keys only after a smoke; until then `None` (refuse, fail closed by design) |
| `worker/prompt/workflow.rs:132,207`, `pipeline/mod.rs:1475,1508` | fallback idle for opencode/omp | pi has a herdr hook (v9) → not fallback, unless the smoke shows no state events |
| `cost.rs:44-47` `is_omp_worker` | harness `omp` or name `sude-/detu-` | pi reader of its own |
| `cost.rs:163` | `harness != "opencode" && !name.starts_with("tieude-")` → return | **bug for pi**: a `tieude-*` worker on harness `pi` falls through into the opencode reader (`cost.rs:167-220`) and may read a stale opencode session of the same name. The pi branch must run before this line. |
| `cost.rs:328-339` `session_store_mtime` | opencode/claude/omp arms | pi arm (same jsonl mtime as omp) |
| `usage.rs:344,383-384` | opencode-go account usage via `opencode auth export opencode` | unchanged if pi and opencode share the opencode-go key [A]; keeps the opencode CLI a runtime dependency |
| `auditor.rs:236,372-377`, `engine/activities.rs:667` | auditor picks claude/opencode on PATH; activities scan `.opencode-state` dbs | unchanged (no kind dependency) |
| `templates/workspace/team.toml:41-46,64-68` | `harness = "opencode"` | `harness = "pi"`, comments updated |

## Design

Launch (`build_launch_cmd`, `pi` arm):

```
herdr agent start <name> --kind pi --pane <p> -- \
  --model opencode-go/deepseek-v4.1-flash --thinking max \
  --append-system-prompt <runs>/<name>.rules.md \
  --session-dir <runs>/.pi-sessions/<name> \
  --no-extensions -e ~/.pi/agent/extensions/herdr-agent-state.ts \
  --no-approve
review: + --tools read,grep,find,ls,bash   (see H2; not a sandbox)
```

- Rules channel: `--append-system-prompt <file>` — same shape as omp (`launch.rs:118-119`), help says it takes "text or file contents" [F]; that pi reads the file (not the literal path) is [A] → P-pi1 smoke.
- `--session-dir` under `runs/` mirrors opencode's per-name `XDG_STATE_HOME` (`launch.rs:107-108`): one dir per worker, no cwd-slug guessing, no collision with ní's own pi sessions. Cost = sum `cost.total` over that dir (reuse the `omp_cost_in` loop with a dir argument). Cleanup archives the dir like the report.
- Isolation: `--no-extensions` + explicit `-e` for the herdr state extension, so personal subagent/memory extensions don't load and herdr still sees state [A: `-e` path keeps idle/done events].
- Effort: kind `effort = "max"` → `--thinking max`; validation by the pi set. Whether `deepseek-v4.1-flash` accepts `max` through pi (opencode allowed `low|high|max`, `config.rs:225`) is [A].
- `resolve_launch` keeps its fail-closed checks (`launch.rs:19-40`); the opencode variant allowlist (`launch.rs:42-63`) does not apply to pi.

## Phases

Each phase lands on its own; each has one acceptance check.

| Phase | Change | Acceptance |
|---|---|---|
| P-pi0 CLI smoke (no code, ≤30 min pilot) | Plain shell, scratch dir, no herdr: `pi -p --no-extensions --no-approve --model opencode-go/deepseek-v4.1-flash --thinking max --append-system-prompt <file with sentinel> --session-dir <tmp> "print the sentinel"` | output contains the sentinel (file read, not the path); a jsonl under `<tmp>` with `message.usage.cost.total` and `thinkingLevel` = `max` |
| P-pi1 launch + config | `pi` in the H4a registry: allowlist, effort set, `build_launch_cmd` arm; stop keys `None` | unit test in `tests/worker_unit.rs` next to `test_launch_args_carry_exact_model_and_effort` (:1430) asserting the exact pi args; `quaqua config validate` accepts a pi kind, rejects `effort = "auto"`. Live: `QUAQUA_TEAM_TOML=<scratch toml with a pi kind> quaqua worker spawn` (the gate blocks a direct `herdr agent start`) → `herdr agent wait` sees idle then done, no trust prompt; record which of `esc` / `esc esc` stops a busy turn, then set stop keys in a 1-line follow-up |
| P-pi2 cost + mtime | pi reader on `--session-dir`; pi branch before `cost.rs:163`; `session_store_mtime` pi arm | fixture jsonl (copied shape of a real line) sums correctly; repro test: `tieude-x` with harness `pi` and a planted opencode db returns the pi cost, fails on old code |
| P-pi3 switch kinds | `team.toml` tieude + suhuynh → pi; fallbacks unchanged (`tieude → detu`) | one live 1-line task on tieude and one review on suhuynh in a scratch worktree: report lands, watchdog exits 0, `cost=` readable |
| P-pi4 (optional, later) | drop opencode as a kind harness | only after ≥2 weeks of pi runs and a decision on `usage.rs` (it still shells to `opencode`) |

## Unknowns

- Does `--append-system-prompt <path>` load the file contents? (P-pi0)
- pi interrupt keys (`stop.rs`). omp, a pi fork, uses one `esc` (`stop.rs:23`); not proof for pi. (P-pi1 live)
- Does `--no-extensions -e <herdr ext>` keep herdr idle/done events? If not, drop `--no-extensions` and add pi to the fallback-idle list. (P-pi1 live)
- Is `max` thinking real for `deepseek-v4.1-flash` via pi, or silently clamped? (P-pi0: check `thinking_level_change` line in the session jsonl)
- Do pi and opencode bill the same opencode-go key, so `usage.rs` stays correct? (`~/.pi/agent/auth.json` vs `opencode auth export`; read-only compare of key ids, not values)
- Does a fresh worktree trigger a pi trust prompt (`~/.pi/agent/trust.json` exists) despite `--no-approve`? Like claude's trust dialog (`spawn.rs:434`). (P-pi1 live)
