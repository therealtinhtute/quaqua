---
name: figure-it-out
when: Large migrations, multi-part changes, or work when no narrower playbook fits.
---

## Orchestrator

1. Define done as a falsifiable predicate on the real artifact, and choose a rigor level proportional to blast radius and reversibility.
2. Decompose the goal into atomic, independently landable units sequenced riskiest-unknown-first.
3. Establish the verification check and capture baseline measurements before work begins.
4. If two or more hypotheses in a class fail the same gate, stop patching and cite principle-attack-the-premise to re-examine the core assumption.
5. Audit each landed unit against its predicate and verify the end-to-end outcome before accepting.

## Worker

1. Frame the unit outcome as a falsifiable done predicate and establish baseline measurements before modifying code.
2. Decompose and order execution into small, verifiable units that prove themselves sequentially, citing principle-sequence-verifiable-units.
3. When multiple attempts fail the same gate, stop and test the shared premise directly, citing principle-attack-the-premise.
4. Run each unit as a hypothesis experiment: state the hypothesis, make the smallest change, measure against the predicate, and keep if verified or revert if refuted. Record verdict as VERIFIED, NOT VERIFIED, or INCONCLUSIVE.
5. Check every Evidence line, then write `Verified: <level> <sha>`, citing principle-prove-it-works.
