---
name: pause-safely
when: Suspending in-flight work cleanly so it can be resumed, on an explicit pause or imminent context compaction.
---

## Orchestrator

You own a clean stop. Leave a durable checkpoint that a cold-start agent can resume from. This applies only on an explicit pause or before coordinator compaction; on "keep going", do not pause.

1. Stop at a safe boundary. Finish the current atomic step or back out cleanly. Start nothing new, and cancel or stand down any active workers.
2. Take no irreversible action to pause: no push, no merge to main, and no destructive branch deletion.
3. Make in-flight work durable. Commit uncommitted edits as one clear `wip:` commit on the current branch so no progress is lost. If the tree is broken or tests fail, state that in the commit message body.
4. Write the resume note off-context to `runs/<name>.resume.md` to protect the context window per principle-guard-the-context-window. Capture intent, current status, verified milestones, open items, key modified files, and the immediate next step.

## Worker
