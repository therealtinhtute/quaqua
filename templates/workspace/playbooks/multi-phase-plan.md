---
name: multi-phase-plan
when: Work that spans phases or stacked PRs; the plan is a checklist an owner runs box by box.
---

## Orchestrator

You own the plan, not the code. The plan is a checklist an owner runs box by box and the coordinator audits from the evidence. The plan is the deliverable. Do not implement.

1. When the change is one or two files with an obvious approach, skip the plan. State so and stop.
2. Settle open questions by prototype before writing. Keep the branch, the SHA, and the recorded observations. Ask the human only about a product or preference call that no run can settle, offering concrete options per principle-never-block-on-the-human.
3. Explore in focused subagents, routing bulk investigation to subagents and keeping summaries in the main thread per principle-guard-the-context-window. Each returns file pointers, conventions, test commands, and entry points, not inlined dumps.
4. Copy the skeleton below into the plan file and fill every placeholder. Unless a path is named, write the file under `docs/plans/active/{slug}.md`. Keep every heading and every sub-block in the order shown. One section per unit or PR. One unit is one change with its own evidence, structured per principle-sequence-verifiable-units.
5. Write with short declarative sentences, concise and audience-aware. The body is a how-to checklist. Appendices hold explanation and reference. Each heading states the task or the finding. No long dashes. No mid-sentence colons.
6. Validate the plan structure per principle-encode-lessons-in-structure, ensuring every checklist box has concrete, verifiable evidence and explicit dependencies.
7. Verification rule: tests alone are not sufficient verification. A unit is verified only when its unit, live, and perf criteria are all checked per principle-prove-it-works. That sentence is the verification rule; every verification block opens with it.
8. Hand back: post the plan path and summary, then stop. Execution starts on explicit confirmation under the coordinator or project Lead (`<slug>-lead`).

```markdown
# <Program> plan

<Under ten lines. What changes, for whom, the rule the program enforces, and the units in order.>

## How to read this

One box is one unit of work. Every box names the evidence that checks it. A nested box is a sub-step of the box above it. Check a box only when its evidence exists: a file, a log line, an artifact, a test run, or a SHA. The body is a how-to. The appendices explain and record.

Tests alone are not sufficient verification. A unit is verified only when its unit, live, and perf boxes are all checked.

## Program checklist

### Arm the program

- [ ] State the protocol and this plan to the coordinator, then stop. Start execution only on explicit go.
- [ ] Read referenced specifications and playbooks from the repository at program start. Re-read them at every milestone.
- [ ] Maintain the audit cadence. Audit the operation against the plan and fix drift in each cycle. Probe every active worker and judge progress by side effects only. Stand down a stuck worker and dispatch its replacement. Then report a short status message only when the audit found a tracked change that no earlier status reported.
- [ ] On a hold or stand-down order, send every worker a stop order at once.

### Spawn owners

- [ ] Spawn one worker per unit via `quaqua worker spawn <name> <kind> <repo>` per `models.md`.
- [ ] Follow the dependency graph. Start dependent work only after its parent is verified and merged.
  - [ ] <Unit 1> and <Unit 2> are independent and first. Both branch from the integration base.
  - [ ] <Unit 3> after <Unit 1>.
- [ ] Hold the file boundaries. <Unit id or class> touches only `<glob>`.
- [ ] Hold the review gate. Units changing critical interfaces wait for review before merge.

### Unit mechanics, for every unit

- [ ] Open the branch from the integration base.
- [ ] Run targeted lint and tests before prompt completion.
- [ ] Keep commits clean and single-purpose; rebase onto current base before code-ready report.

### Verdict and merge, for every unit

- [ ] Verification run at the code-ready head: unit test pass, live surface pass, perf baseline check.
- [ ] Reviewer verdict: clean only when all checks pass and findings are resolved.
- [ ] Merge via `quaqua merge <slug> <branch>...` one unit at a time.

## <Task as a verb phrase> (<Unit id>)

**Depends on.** <Unit id, or None.>

**Files.**

- [ ] Edit `<path>`.
- [ ] Create `<path>`.
- [ ] Delete `<path>`.

**Build.**

- [ ] <One change. Name the symbol and the file.>

**You see.**

- [ ] <One observable result, with the exact log line or screen state.>

**Verify, unit.** Tests alone are not sufficient verification. A unit is verified only when its unit, live, and perf boxes are all checked.

- [ ] <Test file and the case it gains.> Run `<command>`.

**Verify, live.** Tests alone are not sufficient verification. A unit is verified only when its unit, live, and perf boxes are all checked.

- [ ] Lane 1. Regression check against base. Run <load-bearing scenario> at base and head. Pass when <predicate>.
- [ ] Lane 2. <Scenario.> Pass when <predicate>.

**Verify, perf.** Tests alone are not sufficient verification. A unit is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. <What is measured at both base and head.>
- [ ] Probe. <The command or procedure run at base and head.>
- [ ] Baseline. Record base value first.
- [ ] Rule. <Head against base, with the threshold that fails.>

**Review gate.**

- [ ] Independent reviewer verdict recorded at head SHA.

**Merge.**

- [ ] Verified at exact head SHA.
- [ ] Rebased onto current integration branch.
- [ ] Integrated via `quaqua merge`.

## Close the program

- [ ] Every box above is checked with its evidence.
- [ ] Report to the coordinator with summary, verify command outputs, and final integration status.

## Appendix A. Prototype evidence

<Each open question a prototype answered, with the branch, the SHA, and artifact links. Each question that stays unproven.>

## Appendix B. Alternatives rejected

<Each approach weighed and why it lost.>

## Appendix C. Risks

<Each risk with the unit it lands in and what the owner watches.>

## Appendix D. Links and reading list

<Specifications and references to read before editing.>
```

## Worker
