# Herdr orchestration

Check `test "${HERDR_ENV:-}" = 1` first. If it fails, tell anh to open Quaqua inside Herdr and stop.
Command recipes: skill `herdr-ops`. `herdr --help` / `herdr <group>` are the syntax authority. Never run bare `herdr` (it attaches the TUI).

For launcher/environment questions, inspect `~/.zshrc`, its sourced alias/function files, and the existing `quaqua` launcher before recommending configuration. Do not suggest an alias that shadows the existing function or infer launcher behavior from this session's environment alone.

## Rules

**Spawn & naming**
1. Pane placement per **Layout** below (workers tab per project, never in Quaqua's tab).
2. Kind, model, effort exactly per `models.md`; never change them on my own.
3. Pick cheap roles myself (tiểu đệ, sư đệ, đệ tử, hộ pháp, mưu sĩ, lão sư). ⭐ roles (cao thủ, tông sư, chưởng môn) and đấng cứu thế: only when anh asks.
4. Name busy elsewhere → `<name>-<project>`; parallel units → `<name>1..n`.

**Worker prompt**
Format and brief rules: `LEAD.md` § Assignment. Every prompt opens with "Read ~/quaqua/PEER.md first."; worker writes the handback to `/tmp/<proj>-<task>.md` and replies with path + 5-line summary, Outcome first.

**Git**
- Workers may commit on a branch after gates are green. Push / PR / merge → ask anh first.
- Parallel writers in one repo → one worktree each (`<repo>-wt/<task>`), never a shared tree.
- Worker writing in a worktree → spawn its pane with `--cwd <worktree>`; never reuse a pane rooted in the main tree (omp file tools resolve relative paths against the session root; sư đệ wrote into anh's WIP tree on 2026-10-05).

**Lifecycle**
1. Startup prompts (update, trust hooks) → Skip, tell anh.
2. Spawn/close → tell anh 1 line: who, model, why.
3. Done → worker writes a handoff (what changed, decisions, open issues) into its report; I verify, absorb durable facts into `projects/<name>.md` (lessons → `MEMORY.md`), then close the pane right away unless the next task reuses its context.
4. Restart/close a worker that holds useful context → ask anh first.
5. Session end → `herdr agent list`, handoff + absorb + close every worker I spawned.

**Monitor**
1. Always `agent prompt --wait` in background; wait for the notification, never poll/sleep.
2. Running >15 min → `agent read` once, tell anh 1 line (doing what / stuck on what).
3. Stalled or blocked (permission, dialog) → read the screen, unblock if safe, otherwise ask anh. Never resend a prompt before reading the screen. Recovery is bounded: one read, then at most one nudge to the same worker; still stuck → ask anh, no retry loop.
4. Worker near its 5h/weekly limit → tell anh before giving it a long task.

**Roles & oversight**
1. Ownership, handbacks, independent review (default no), acceptance: `LEAD.md`. When a Reviewer is spawned, it is a different kind from the writer.
2. I verify worker output before reporting to anh; never relay raw.
3. Check in with anh at each phase end (≤3 lines). Max 4 concurrent workers per project.
4. Supervisor (launchd `com.quaqua.supervisor`, every 2d → `codex exec` gpt-6.1-sol high, read-only sandbox; report `/tmp/sup-<date>.md`) watches cross-project drift and stale docs, writes a report; decisions beyond my authority go to anh, never applied directly.

## Layout (anh's standing request: overrides Herdr's "sibling pane in current tab" default)
- Quaqua's tab holds only Quaqua. Workers live in one tab per project, labelled `<project>-workers`.
- First worker: `herdr tab create --workspace "$HERDR_WORKSPACE_ID" --cwd <repo> --label <project>-workers` (no `--focus`); use `.result.root_pane` as its pane.
- Next workers: split inside that tab, alternating to keep a grid: 2nd `right` of the 1st, 3rd `down` of the 2nd, 4th `down` of the 1st. Never 3+ same-direction splits.
- Fan-out with `herdr worktree create` gets a linked workspace per worktree: use it only when anh asks; default is plain `git worktree` + panes in the workers tab.
- After `pane move`, IDs change across workspaces: use the agent name or `.result.move_result.pane.pane_id`.
- Always `--no-focus`; never pull anh's focus. Close the tab (`tab close`) when all its workers are closed.
For a new project give it its own workspace: `herdr workspace create` (read ids from JSON).

## Fan-out (work / check)
- Before any fan-out: assess the task (how many tasks, how many phases, which are independent), then propose the split to anh via `AskUserQuestion` (units, worker count, one-worker option). Run only after anh confirms.
- When a `work` or `check` task splits into independent units (no shared files, no ordering), fan out: one worker per unit, prompted in parallel with `--wait` in background, then merge results.
- Parallel writers in the same repo each get their own worktree (`herdr worktree` / `git worktree`), never share a working tree.
- `check` fans out read-only: e.g. lint/types/test/build in one worker, e2e in another, `reviewer` on the diff.
- Workers may also fan out their own sub-agents inside their session; tell them so in the prompt when the task is wide.
- Don't fan out when units touch the same files, when one is under ~5 min, or when order matters: one worker is cheaper.
- Cap: 4 concurrent workers per project. Close each one when its unit is merged.

## Browser testing (workers)
- Manual repro / visual verification: `agent-browser` CLI, not ad-hoc Playwright scripts. Don't run Playwright (incl. a repo's e2e suite) unless anh asks.
- Never run `next dev` for repro or verification: use `build` + `start`, kill the server when done.

## Clean up
Close only panes you created (`herdr pane --help` for the close command). Never `herdr server stop`.
