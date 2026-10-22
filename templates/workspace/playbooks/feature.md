---
name: feature
when: New or changed behavior, built from a named data shape.
---

## Orchestrator

1. Define the desired behavior, interfaces, and data contracts in the brief before implementation begins.
2. Establish the throughput checkpoint: resolve blocking first steps, partition independent workstreams, separate shared mutable state, and define the smallest safe decomposition.
3. Require tests that validate external behavior and boundary invariants.
4. Re-run verify checks on the branch before integrating or merging.

## Worker

1. Clarify the core domain model, state transitions, and boundary interfaces before writing business logic.
2. Design mutating operations and lifecycle steps to converge to the correct state across retries and restarts, citing principle-make-operations-idempotent.
3. Write tests that assert what users and callers observe through the contract, citing principle-test-behavior-not-implementation.
4. Implement the feature code, verifying that new and existing targeted test suites pass.
5. Check every Evidence line, then write `Verified: <level> <sha>`, citing principle-prove-it-works.
