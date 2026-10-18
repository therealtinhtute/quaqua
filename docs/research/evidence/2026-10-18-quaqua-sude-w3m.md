# Run report: sude-w3m

## Changes
- Merged `quaqua/clean-integ` (`00e225f`) into `quaqua/sude-w3m` (base `463732f`).
- Resolved merge conflicts:
  - `crates/quaqua/tests/worker_unit.rs`: combined unit test additions from both branches (`test_update_index_content_markdown_table`, `test_update_project_index_owner_fixture_atomic`, `test_update_project_index_owner_concurrency`, `test_cleanup_worker_cleans_budget_grace_cost_and_metadata`, `test_state_atomic_write_and_metadata_roundtrip`).
  - `tests/test_spawn_lead.sh` & `tests/test_cleanup_lead.sh`: updated invocation to binary `${QUAQUA_BIN:-quaqua}` with `QUAQUA_ROOT="$T"` and preserved Wave 3's real concurrent race (no barrier awk).
- Sealed hermetic test environment in `tests/test_spawn_lead.sh` and `tests/test_cleanup_lead.sh`:
  - Exported `HERDR_BIN_PATH="$T/bin/herdr"` and `HERDR_SOCKET_PATH="$T/herdr.sock"`.
  - Added `"workspace get"` handler to `$T/bin/herdr` stub in `tests/test_spawn_lead.sh`.
  - Passed `HERDR_BIN_PATH` and `HERDR_SOCKET_PATH` across helper functions and background concurrency arrays (`ce=(...)`).

## Root Cause of Leak
- File and line: `crates/quaqua/src/herdr.rs:43`
  ```rust
  let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string());
  ```
- Cause: `Herdr::new()` checks the `HERDR_BIN_PATH` environment variable first before falling back to searching `PATH`. Because the ambient environment has `HERDR_BIN_PATH=/opt/zerobrew/bin/herdr` exported, `quaqua` invoked the live herdr binary directly instead of `$T/bin/herdr` on `PATH`.
  - On `tests/test_spawn_lead.sh`, happy path called live herdr, creating real workspaces (`wW`/`wX`) and agent `coucou-island-lead`.
  - On `tests/test_cleanup_lead.sh`, cleanup called live herdr for `agent get baro-lead`, which failed with `agent_not_found` from real herdr, producing `FAIL: team: exit 1: no live Lead 'baro-lead'`.
- Fix: Pin `export HERDR_BIN_PATH="$T/bin/herdr"` and `export HERDR_SOCKET_PATH="$T/herdr.sock"` in both test suites so all Rust `Herdr` instances invoke the local test stub.

## Evidence

### Commits & Diffs
- Merge commit: `71a0656` (`Merge branch 'quaqua/clean-integ' into quaqua/sude-w3m`)
- Fix commit: `d73c1f8` (`fix(test): seal herdr stub isolation in lead spawn and cleanup tests`)

`git log --oneline -3`:
```
d73c1f8 fix(test): seal herdr stub isolation in lead spawn and cleanup tests
71a0656 Merge branch 'quaqua/clean-integ' into quaqua/sude-w3m
00e225f Merge branch 'quaqua/sude-w4a' into quaqua/clean-integ
```

`git diff --stat 71a0656..d73c1f8`:
```
 tests/test_cleanup_lead.sh | 7 +++++--
 tests/test_spawn_lead.sh   | 8 ++++++--
 2 files changed, 11 insertions(+), 4 deletions(-)
```

### Suite Output
Command: `env -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin QUAQUA_BIN=$(pwd)/target/debug/quaqua bash tests/test_spawn_lead.sh`
```
ok: herdr-grid -> Lead pane tagged as orchestrator (role=orchestrator, owner=coucou-island-lead), no grid_ready
ok: brief -> redirects reach the Lead through the CoS only
ok: happy path -> get/workspace/start/prompt order, claude args, rules file, owner updated
ok: duplicate -> refused after agent get, no workspace/start, INDEX untouched
ok: uppercase slug -> exit 1, no herdr/git
ok: underscore slug -> exit 1, no herdr/git
ok: --flag arg -> exit 1, no herdr/git
ok: single-dash slug -> exit 1, no herdr/git
ok: unknown slug -> exit 1, no herdr/git
ok: row but no card -> exit 1, no herdr/git
ok: lead name >32 chars (registered slug) -> exit 1, no herdr/git
ok: 32-char lead name (27-char slug) accepted
ok: 'mono' updates only its own row (mono-desk untouched)
ok: LEAD_KIND overrides strategy.lead_kind
ok: missing quaqua/<slug>-integ -> created from main before the workspace
ok: master-only repo -> integration branch created from master
ok: origin/HEAD set -> integration branch created from develop
ok: origin/HEAD without a local ref -> integration branch created from origin/develop
ok: origin/HEAD stale -> integration branch created from main
ok: origin/HEAD set -> integration branch created from it (precedence over main)
ok: no default branch in the project repo -> refused before side effects
ok: stale owner cell -> respawn allowed (agent registry is the truth)
ok: INDEX '~/repo' expands to a literal HOME (no patsub_replacement corruption)
ok: concurrent spawn vs cleanup on different rows -> both updates kept
PASS
```

Command: `env -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin QUAQUA_BIN=$(pwd)/target/debug/quaqua bash tests/test_cleanup_lead.sh`
```
ok: team workspace -> closed, owner reset to cos, dead Peer owner file ignored
ok: Lead in the caller's workspace -> only its pane closed
ok: foreign-labelled workspace -> only the Lead pane closed
ok: live Peer -> refused, nothing closed, INDEX untouched
ok: missing Lead and bad slug -> refused
ok: Lead lookup transport error -> abort (not 'no live Lead'), nothing touched
ok: Peer transport error -> abort, nothing closed, INDEX untouched
ok: Peer non-not_found JSON error -> abort
ok: Peer get rc 0 with a non-JSON body -> abort
ok: concurrent cleanups on different rows -> both updates kept, no torn file
PASS
```

### Hermetic Proof (`herdr agent list`)
Baseline before tests:
```json
{
  "id": "cli:agent:list",
  "result": {
    "agents": [
      {
        "agent": "opencode",
        "agent_session": {
          "agent": "opencode",
          "kind": "id",
          "source": "herdr:opencode",
          "value": "ses_eb61ab5c3ffeK3kaF75okkcwsI"
        },
        "agent_status": "idle",
        "completion_seq": 485,
        "cwd": "~/quaqua",
        "focused": true,
        "foreground_cwd": "~/quaqua",
        "pane_id": "w9:p2W",
        "revision": 2,
        "screen_detection_skipped": true,
        "state_change_seq": 485,
        "tab_id": "w9:t26",
        "terminal_id": "term_65e07483dce7e2",
        "terminal_title": "OC | Lời chào vui vẻ",
        "terminal_title_stripped": "OC | Lời chào vui vẻ",
        "workspace_id": "w9"
      },
      {
        "agent": "omp",
        "agent_session": {
          "agent": "omp",
          "kind": "path",
          "source": "herdr:omp",
          "value": "~/.omp/agent/sessions/-.wt-sude-w3m/2026-10-18T10-06-44-309Z_01a14e7a-5c55-70e4-9f08-4088c95da3fe.jsonl"
        },
        "agent_status": "working",
        "cwd": "~/.wt/sude-w3m",
        "focused": false,
        "foreground_cwd": "~/.wt/sude-w3m",
        "interactive_ready": true,
        "name": "sude-w3m",
        "pane_id": "w9:p48",
        "revision": 11040,
        "screen_detection_skipped": true,
        "state_change_seq": 482,
        "tab_id": "w9:t2D",
        "terminal_id": "term_65e1a8def73673f",
        "terminal_title": "π ⠴ Merge clean integ and hermeticize leads",
        "terminal_title_stripped": "π ⠴ Merge clean integ and hermeticize leads",
        "tokens": {
          "grid_group": "quaqua",
          "grid_owner": "cos",
          "grid_ready": "1",
          "grid_role": "worker",
          "grid_tab": "quaqua-workers"
        },
        "workspace_id": "w9"
      }
    ],
    "type": "agent_list"
  }
}
```

Check after tests:
```json
{
  "id": "cli:agent:list",
  "result": {
    "agents": [
      {
        "agent": "opencode",
        "agent_session": {
          "agent": "opencode",
          "kind": "id",
          "source": "herdr:opencode",
          "value": "ses_eb61ab5c3ffeK3kaF75okkcwsI"
        },
        "agent_status": "idle",
        "completion_seq": 485,
        "cwd": "~/quaqua",
        "focused": true,
        "foreground_cwd": "~/quaqua",
        "pane_id": "w9:p2W",
        "revision": 2,
        "screen_detection_skipped": true,
        "state_change_seq": 485,
        "tab_id": "w9:t26",
        "terminal_id": "term_65e07483dce7e2",
        "terminal_title": "OC | Lời chào vui vẻ",
        "terminal_title_stripped": "OC | Lời chào vui vẻ",
        "workspace_id": "w9"
      },
      {
        "agent": "omp",
        "agent_session": {
          "agent": "omp",
          "kind": "path",
          "source": "herdr:omp",
          "value": "~/.omp/agent/sessions/-.wt-sude-w3m/2026-10-18T10-06-44-309Z_01a14e7a-5c55-70e4-9f08-4088c95da3fe.jsonl"
        },
        "agent_status": "working",
        "cwd": "~/.wt/sude-w3m",
        "focused": false,
        "foreground_cwd": "~/.wt/sude-w3m",
        "interactive_ready": true,
        "name": "sude-w3m",
        "pane_id": "w9:p48",
        "revision": 11040,
        "screen_detection_skipped": true,
        "state_change_seq": 482,
        "tab_id": "w9:t2D",
        "terminal_id": "term_65e1a8def73673f",
        "terminal_title": "π ⠴ Merge clean integ and hermeticize leads",
        "terminal_title_stripped": "π ⠴ Merge clean integ and hermeticize leads",
        "tokens": {
          "grid_group": "quaqua",
          "grid_owner": "cos",
          "grid_ready": "1",
          "grid_role": "worker",
          "grid_tab": "quaqua-workers"
        },
        "workspace_id": "w9"
      }
    ],
    "type": "agent_list"
  }
}
```

## Risks
- Low: Changes are strictly confined to test script harnesses (`tests/test_spawn_lead.sh` and `tests/test_cleanup_lead.sh`). No production crate code (`src/`) was modified.

## Blockers
none

## Lessons
- Always pin HERDR_BIN_PATH="$T/bin/herdr" and HERDR_SOCKET_PATH in tests since Herdr::new() prefers HERDR_BIN_PATH over PATH.
