# Run report: sude-hermpin

## Changes
Pin hermetic `herdr` environment (`HERDR_BIN_PATH` and `HERDR_SOCKET_PATH`) across all test suites in `tests/` that use a stubbed herdr executable and invoke `${QUAQUA_BIN:-quaqua}`.

- Commit SHA: `01afa0a47e8da81c85b03696229a8de370e0b999`
- Branch: `quaqua/sude-hermpin` (based on `main` at `573b8f0`)
- `git diff --stat`:
```text
 tests/test_auditor.sh                | 5 ++++-
 tests/test_cleanup_worker.sh         | 4 +++-
 tests/test_init_workspace.sh         | 2 ++
 tests/test_pipeline.sh               | 8 +++++++-
 tests/test_prompt_worker.sh          | 1 +
 tests/test_prompt_worker_watchdog.sh | 9 ++++++---
 tests/test_spawn_worker.sh           | 8 ++++++--
 tests/test_taskforce_cap.sh          | 2 ++
 tests/test_worker_cap.sh             | 7 +++++--
 9 files changed, 36 insertions(+), 10 deletions(-)
```

## Audit Table (22 suites)

| Suite | Có stub? | Pin trước | Pin sau | Ghi chú |
|---|---|---|---|---|
| `cargo_test` | Không | N/A | N/A | Rust crate tests, không dùng herdr stub |
| `tests/test_auditor.sh` | Có (`$T/bin/herdr`) | Chỉ PATH | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Export top-level và truyền trong `run()` |
| `tests/test_cleanup_lead.sh` | Có (`$T/bin/herdr`) | Đủ | Đủ | Giữ nguyên mẫu sude-w3m |
| `tests/test_cleanup_worker.sh` | Có (`$T/bin/herdr`) | Thiếu `SOCKET` | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Bổ sung `HERDR_SOCKET_PATH` top-level + `run_cleanup` |
| `tests/test_config.sh` | Không | N/A | N/A | Thuần validate/get team.toml |
| `tests/test_docs.sh` | Không | N/A | N/A | Thuần doc/link consistency |
| `tests/test_gate_shim.sh` | Không | N/A | N/A | Unit test shim fail-closed, không có herdr stub |
| `tests/test_herdr_gate.sh` | Không | N/A | N/A | Unit test jq parser cho hook commands |
| `tests/test_herdr_grid_plan.sh` | Không | N/A | N/A | Unit test jq cho plan.jq |
| `tests/test_herdr_grid_rust.sh` | Có (`$T/bin/herdr`) | Đủ | Đủ | Đã pin đủ từ trước |
| `tests/test_init_workspace.sh` | Có (`$T/bin/herdr`) | Chỉ PATH | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Export top-level trước các lệnh `quaqua init` |
| `tests/test_install_auditor.sh` | Không | N/A | N/A | Sandbox installer test (systemd/launchd) |
| `tests/test_merge_queue.sh` | Không | N/A | N/A | Git merge queue tests, không stub herdr |
| `tests/test_pipeline_grid.sh` | Có (`$T/bin/herdr`) | Đủ | Đủ | Đã pin đủ từ trước |
| `tests/test_pipeline.sh` | Có (`$T/bin/herdr`, `$T/rgbin/herdr`) | Chỉ 1 nhánh `run_pipe2` | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Pin cả 2 stub `$T/bin` và `$T/rgbin`, ngăn leak spawn `sude-isl` |
| `tests/test_pre_commit.sh` | Không | N/A | N/A | Git hook portable paths check, không stub herdr |
| `tests/test_prompt_worker_watchdog.sh` | Có (`$T/bin/herdr`) | 0 mention | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Export top-level + truyền trong `run`, `run_brief`, `resume` |
| `tests/test_prompt_worker.sh` | Có (`$T/bin/herdr`) | Đã pin env | Đủ | Bổ sung export PATH top-level cho đồng bộ |
| `tests/test_spawn_lead.sh` | Có (`$T/bin/herdr`) | Đủ | Đủ | Giữ nguyên mẫu sude-w3m |
| `tests/test_spawn_worker.sh` | Có (`$T/bin/herdr`, `$T/bin2/herdr`) | Thiếu `SOCKET`, thiếu `bin2` | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Pin cả 2 stub `$T/bin` và `$T/bin2`, giữ PATH cục bộ để real-git không bị ô nhiễm |
| `tests/test_taskforce_cap.sh` | Có (`$T/bin/herdr`) | Chỉ PATH | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Export top-level cho các lệnh `cap`, `worker spawn`, `taskforce` |
| `tests/test_worker_cap.sh` | Có (`$T/bin/herdr`) | 0 mention | `HERDR_BIN_PATH` + `HERDR_SOCKET_PATH` | Export top-level + truyền trong `spawn()` và pipeline cap check |

## Verification & Leak-Check

### Ambient Env Leak-Check (TRƯỚC / SAU)
Ambient environment: `HERDR_BIN_PATH=/opt/zerobrew/bin/herdr`, `HERDR_SOCKET_PATH=~/.config/herdr/herdr.sock`.
- **Trước**:
  - Agents: 2 (`opencode` on `w9:p2W`, `omp` on `w9:p4P`)
  - Panes: 2 (`w9:p2W`, `w9:p4P`)
- **Sau**:
  - Agents: 2 (`opencode` on `w9:p2W`, `omp` on `w9:p4P`)
  - Panes: 2 (`w9:p2W`, `w9:p4P`)
- **Leak count**: 0 mới (không có pane rác, không có agent rác).

### Grep Verification
Mọi suite có stub đều được pin đầy đủ:
```text
tests/test_auditor.sh:60:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_auditor.sh:61:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_cleanup_lead.sh:51:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_cleanup_lead.sh:52:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_cleanup_worker.sh:63:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_cleanup_worker.sh:64:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_herdr_grid_rust.sh:41:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_herdr_grid_rust.sh:42:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_init_workspace.sh:79:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_init_workspace.sh:80:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_pipeline.sh:114:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_pipeline.sh:115:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_pipeline.sh:686:export HERDR_BIN_PATH="$T/rgbin/herdr"
tests/test_pipeline.sh:687:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_pipeline_grid.sh:131:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_pipeline_grid.sh:132:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_prompt_worker.sh:71:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_prompt_worker.sh:72:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_prompt_worker_watchdog.sh:120:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_prompt_worker_watchdog.sh:121:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_spawn_lead.sh:75:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_spawn_lead.sh:76:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_spawn_worker.sh:131:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_spawn_worker.sh:132:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_spawn_worker.sh:662:export HERDR_BIN_PATH="$T/bin2/herdr"
tests/test_spawn_worker.sh:663:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_taskforce_cap.sh:65:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_taskforce_cap.sh:66:export HERDR_SOCKET_PATH="$T/herdr.sock"
tests/test_worker_cap.sh:67:export HERDR_BIN_PATH="$T/bin/herdr"
tests/test_worker_cap.sh:68:export HERDR_SOCKET_PATH="$T/herdr.sock"
```

## Blockers
- `tests/test_pipeline.sh:530` fails assertion `grep -q 'no commits.*no changes' "$runsdir/pipeline-nop.md"` because in `crates/quaqua/src/pipeline/phases/mod.rs:334`, `let _ = std::fs::write(&self.pfile, pcontent);` was accidentally omitted during refactor 7d3dc4e (dead store before `self.finish`). Fixing it in `crates/` would violate the hard constraint "không đụng crates/". Adding that line allows `test_pipeline.sh` to pass 100%.

## Lessons
- In test suites with both stubbed git and real-git phases, never export `PATH="$T/bin:$PATH"` globally; scope stub PATH locally to avoid poisoning real git.
