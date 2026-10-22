---
name: bug-fix
when: A reported defect to reproduce, root-cause, and fix with runtime evidence.
---

## Orchestrator

1. Frame the defect, reproduction conditions, and verify commands in the brief.
2. If two or more previous fixes in this class failed the same gate, stop patching and cite principle-attack-the-premise to test the underlying assumption first.
3. Keep the worker scoped to reproduction, root-cause isolation, and the minimal fix.
4. Review the failing-then-passing evidence on the target branch before approving or merging.

## Worker

1. Reproduce the bug directly on the target surface with runtime evidence. Capture the failing observation before making changes. If two or more fixes sharing a premise have failed, stop and apply principle-attack-the-premise.
2. Binary-search and isolate the mechanism to find the root cause, citing principle-fix-root-causes. Trace the symptom back to the first point of failure rather than patching downstream effects.
3. Write an automated reproduction test asserting observable behavior rather than internal implementation details, citing principle-test-behavior-not-implementation.
4. Implement the minimal fix addressing the root cause. Re-run the reproduction test and targeted test suite to confirm they pass cleanly.
5. Check every Evidence line, then write `Verified: <level> <sha>`, citing principle-prove-it-works.
