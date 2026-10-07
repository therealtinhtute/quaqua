# Setup prompt: SLP orchestration with Herdr

Set up a multi-agent orchestration workspace on this machine using the SLP model (Supervisor, Lead, Peer), with Herdr as the terminal workspace where the agents run. Read the references first, then propose a setup plan and ask me before installing anything.

## References
- SLP article: https://vhlam.com/article/agent-orchestration-multi-agent-slp
- Herdr: run `herdr --help` once it's installed; find its plugins under the GitHub topic `herdr-plugin`.
- pi-setup (pi agent config): https://github.com/therealtinhtute/pi-setup

## Architecture
- **Lead** (one main session):
  - Turns my vision and priorities into concrete tasks.
  - Assigns them to workers. It talks to them as me: short, technical, no mention of AI or of representing anyone.
  - Never implements itself.
  - Verifies the work before reporting to me.
- **Peers** (worker agents, each in its own Herdr pane):
  - Implement or review exactly one scoped task.
  - Implementation and review go to different agents.
- **Supervisor** (read-only, scheduled every ~2 days):
  - Looks at the Lead's activity and the workspace docs.
  - Flags drift and docs that have gone stale against the real code.
  - Writes a report. Decisions go to me; it never applies changes.

## Invariants
- Roles define responsibility and authority, not personality.
- One scope has one owner. No two workers on the same files, and the Lead doesn't patch a scope it delegated.
- Every brief has 3 parts: **Goal**, **Hard constraints**, and **Current design** (open to challenge). Never pre-solve the task.
- Peers may push back with evidence (a repo or a test). Pushback is a right, not a duty. The Lead then changes the plan, keeps it with a stated reason, or escalates to me.
- Every redirect, whether from me or from the Supervisor, flows through the Lead.
- Keep it simple: no heavy process, no issue tracking, no redundant docs. Work directly on the code.

## Done when
- The Lead can spawn a worker in Herdr, brief it, and read back its result.
- The Supervisor produces one report.
- You report back in 5 lines or fewer.
