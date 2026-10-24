# Farm layouts — delivery roadmap

CoS dispatches; three Leads run lanes; anh iu approves at four checkpoints. Target: ~6–7 working days.

## Token key decision (wave 0, 2026-10-24)
Use **`grid_role=cos`** (D3), not a separate `role` key.
- grid already treats `cos` as a non-worker anchor: `grid/plan.rs:264` and `:302` (owner lookup accepts `orchestrator`/`cos`/`lead`); every worker check compares against `"worker"` only (`grid/plan.rs:48`, `grid/reconcile.rs:293`, `grid/hook.rs:101,123,217`, `grid/apply.rs:222`). Grid tests already use `grid_role=cos` (`grid/plan.rs:822`, `grid/reconcile.rs:2291`).
- Conflict to fix in P0: `init.rs:920` tags the CoS pane `grid_role=orchestrator`, which the P0 mapping reads as `lead`. P0 changes it to `grid_role=cos` (or calls `cos tag`), so init and `cos tag` agree.
- The sidebar chip template reads `$grid_role` instead of the hand-test `$role`.

## Lanes
| Lane | Lead | Phases | Starts after |
|---|---|---|---|
| — | CoS | P0 (split: `sude-farmid` identity+depth, `sude-costag` cos tag+protection) | wave 0 |
| A gate | `gate-lead` | P1 → P3 shadow → (wait) → P4 | CP1 |
| B bus | `bus-lead` | P2 → P5 → P6 | CP1 |
| C freeze | `freeze-lead` | P7 → P8 | P1 merged |

Each Lead: per phase one `sude` builder + one `suhuynh` reviewer (≤4 peers, `team.toml` `max_peers`). Leads report to the CoS inbox; only the CoS pushes/opens PRs, after anh iu approves.

## Checkpoints (anh iu)
- CP1 (D0): P0 PR, plus a real-herdr hand test.
- CP2 (D2): P1 + P2 PRs (stands in for the E1 sign-off on `journal_sqlite.rs`).
- CP3 (D5): shadow `would_deny` counts from real use; 0 false denies → P4.
- CP4 (D7): P4 + P8 (stands in for the K4 sign-off on `taskforce/`); release call.

## Per-phase gate
1. The phase's proving tests (design.md rollout table), red-then-green; rebuild the binary before bash suites.
2. `cargo test --workspace` green on a branch rebased onto latest `main`.
3. Cross-harness review by intent.
4. Grep `crates/quaqua/tests/*.rs` and `tests/*.sh` before changing any `team.toml` value.
5. Report `runs/<lead>-<phase>.md`; CoS posts one line to anh iu.

## Merge order
`config.rs` is touched by P1, P7, P8: lane C always rebases after lane A merges.
