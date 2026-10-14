# SLP Pattern (Nested Orchestration)

## Roles

- Principal: defines priorities and approves release.
- CoS: owns intake, design, brief, build, verify, integrate, release approval process, learning.
- Lead: owns project integration branch, dependencies, peer lifecycle, reports to CoS.
- Peer: owns one task in isolated worktree and may challenge assumptions with evidence.

## Rules

- Use SLP only for project-size work; small work stays single-worker supervised.
- CoS may recruit Lead; Lead may spawn up to 4 peers.
- Builder/reviewer harnesses should differ.
- Every worker owns one task and one branch.
- Challenge evidence must be written in `runs/<name>.md`.
- Recovery happens before merge.
- CoS verifies output/tests and re-runs gates before reporting done.
- Push, merge to main, or public release requires Principal approval.

## Branch model

- Main remains canonical.
- Lead owns `quaqua/<slug>-integ`.
- Peer agents create `quaqua/<peer-name>`.
- Integration lands only after CoS verification and Lead review.
