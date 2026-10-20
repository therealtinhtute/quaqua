# Lead → CoS inbox

- A Lead reports to the CoS only by appending to `~/quaqua/runs/<slug>-lead.inbox.md`, never by `herdr agent prompt` to the CoS pane: a prompt lands in anh iu's main chat and is rejected while the CoS waits on a question — 2026-10-19, arch-lead E-side report.
- One entry per event, appended in a single write (`cat >> file <<'EOF'`): first line `## <ISO-8601 time> <lead-name> <kind>`, kind = `handoff | accept | challenge | escalate | blocked`, then ≤10 lines: verdict, SHA, verify output, cost, ask.
- Append only; never rewrite or truncate the file. The CoS answers by prompting the Lead.
