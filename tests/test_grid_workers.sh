#!/usr/bin/env bash
# Sandbox test for grid-workers.sh layout planning. No live panes are touched:
# a stub herdr serves fixture state and records mutations.
# Usage: tests/test_grid_workers.sh   (exit 0 = pass)
set -euo pipefail

# This suite drives the CLI of grid-workers, which the R6
# cutover replaced with `quaqua grid reconcile`. Run against
# the shim it fails at its first case: the old CLI's plan/exec output, exit
# codes and stderr hints are now the Rust CLI's, so the suite cannot be kept
# as-is. Finding 2 (suhuynh-r6r1) requires a case-by-case mapping instead of a
# bare skip — every case below names its Rust replacement, or says why it was
# dropped. The Rust coverage lives in crates/quaqua/tests/grid.rs (33 tests)
# and the end-to-end shim driver tests/test_herdr_grid_rust.sh.
#
#   "1 worker: rebuild to 2/5|3/5"                     -> grid.rs::test_1_worker_split_beside_anchor
#   "1 worker: already in place"                       -> grid.rs::test_reconcile_only_with_custom_tab_converges_immediately
#   "2 workers: left|right"                            -> grid.rs::test_2_workers_lands_in_grid_tab_never_orchestrator_tab
#   "3 workers: A|B top + C below"                     -> grid.rs::test_differential_all_fixtures_against_plan_jq (plan.jq oracle)
#   "4 workers: 2x2"                                   -> grid.rs::test_differential_all_fixtures_against_plan_jq
#   "3 workers: already in grid"                       -> grid.rs::test_reconcile_only_with_custom_tab_converges_immediately
#   "3->4: place ni-delta right of sude-gamma"         -> grid.rs::test_deferred_op_propagates_to_dependent_splits
#   "3->2: rebuild"                                    -> grid.rs::test_repair_arranges_all_groups_using_tab_as_default
#   "2->3: rebuild"                                    -> grid.rs::test_repair_arranges_all_groups_using_tab_as_default
#   "back to 1 worker: return to CoS tab"              -> grid.rs::test_1_worker_split_beside_anchor
#   "working worker skipped"                           -> grid.rs::test_reconcile_honours_deferred_statuses
#   "working worker: no move"                          -> grid.rs::test_grid_deferred_statuses_from_config
#   "cross-workspace agent ignored"                    -> grid.rs::test_edge_fixture_worker_with_foreign_pane
#   "exec: 2 workers"                                  -> grid.rs::test_fake_herdr_driver_applies_ops_cleanly
#   "exec: rebuild via scratch tab"                    -> grid.rs::test_hook_failure_backoff_uses_post_failure_sig_and_scratch_normalization
#   "exec: back to CoS tab"                            -> grid.rs::test_1_worker_split_beside_anchor (apply path)
#   "names: two pi workers keep their own names"        -> grid.rs::test_repair_renames_with_new_pane_ids_and_respects_only
#   "names: unnamed, non-worker and <slug>-lead agents are not workers"
#                                                      -> grid.rs::test_edge_fixture_cross_checker_owner_group (+ cap.rs::is_task_worker_name)
#   "recheck: working before the first move -> nothing moved"
#                                                      -> grid.rs::test_apply_ops_rechecks_idle_and_fails_if_busy
#   "recheck: flip after the first move -> one move, no renames"
#                                                      -> grid.rs::test_apply_ops_rechecks_idle_and_fails_if_busy
#   "move no-op: new-tab changed=false -> stop, no renames"
#                                                      -> grid.rs::test_parse_move_response_and_changed_false
#   "move no-op: split changed=false -> stop after the first move"
#                                                      -> grid.rs::test_parse_move_response_and_changed_false
#   "workers tab: CoS tab labelled workers is not reused"
#                                                      -> grid.rs::test_reviewer_n1_scenario_b_never_lands_in_orchestrator_tab
#   "workers tab: tab with a plain shell is skipped"    -> grid.rs::test_repair_arranges_all_groups_using_tab_as_default
#   "id churn: anchors and renames follow the returned ids"
#                                                      -> grid.rs::test_repair_renames_with_new_pane_ids_and_respects_only
#   "--tab: new tab uses the project label"             -> grid.rs::test_reconcile_only_with_custom_tab_converges_immediately
#   "group: another project's workers are ignored"      -> grid.rs::test_edge_fixture_cross_checker_owner_group
#   "group: the CoS workers tab is ignored for a project label"
#                                                      -> grid.rs::test_repair_arranges_all_groups_using_tab_as_default
#   "group: agent in an unrelated tab is ignored"       -> grid.rs::test_edge_fixture_worker_with_foreign_pane
#   "--tab: existing project grid reused"               -> grid.rs::test_reconcile_only_with_custom_tab_converges_immediately
#   "exec: rebuild via the group's scratch tab"         -> grid.rs::test_hook_failure_backoff_uses_post_failure_sig_and_scratch_normalization
#   "scratch: parked worker rejoins the group"          -> grid.rs::test_hook_failure_backoff_uses_post_failure_sig_and_scratch_normalization
#   "names: workers tab with an unnamed session is not reused"
#                                                      -> grid.rs::test_repair_arranges_all_groups_using_tab_as_default
#   "--only: unlisted worker untouched, listed worker -> project tab"
#                                                      -> grid.rs::test_repair_renames_with_new_pane_ids_and_respects_only
#   "--only: two listed workers -> project tab, third untouched"
#                                                      -> grid.rs::test_repair_renames_with_new_pane_ids_and_respects_only
#   "--only: same-group unlisted occupant keeps the tab reusable"
#                                                      -> grid.rs::test_reconcile_only_with_custom_tab_converges_immediately
#   "owner: a foreign-owner worker is ignored, mine stays beside the anchor"
#                                                      -> grid.rs::test_edge_fixture_cross_checker_owner_group
#   "groups: each project gets its own tab"             -> grid.rs::test_edge_fixture_cross_checker_owner_group
#   "groups: a lone stranded worker returns beside the anchor"
#                                                      -> grid.rs::test_edge_fixture_missing_workers_tab
#   "--only: other-group co-owner -> project tab, other group untouched"
#                                                      -> grid.rs::test_edge_fixture_cross_checker_owner_group
#
# Dropped (bash-CLI surface only, the assertions cannot exist in the Rust CLI
# the same way): "--tab without a value -> usage error", "--only without a
# value -> usage error", "--check and --repair together -> error exit",
# "--only matching no agents -> no-op", "--repair -> runs arranger",
# "--check on clean layout -> exit 0", "--check with only deferred moves ->
# exit 0", "--check on pending ops -> times out exit 1", "--check --only ignores
# foreign ops -> exit 0", "--check --only honours a custom --tab", "--check
# --only still waits on the requested tab's ops", "recheck: replan hint on
# stderr", "move no-op: reason reported". Their Rust homes are
# grid.rs::test_reconcile_only_with_custom_tab_converges_immediately,
# test_check_only_waits_when_listed_split_dependency_is_unlisted and the
# end-to-end CLI cases in tests/test_herdr_grid_rust.sh; the exact usage text
# and exit codes are clap's now, not a jq/awk script's.
echo "SKIP: grid-workers replaced by quaqua grid reconcile (covered by test_herdr_grid_rust.sh)"
exit 0
