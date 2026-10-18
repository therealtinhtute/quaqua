# Run report: sude-finalfix — Fix 4 findings của final review

## Metadata
- Worker: `sude-finalfix`
- Base: `c404f9b` (`quaqua/clean-integ`)
- Commit: `e367313` (`quaqua/sude-finalfix`)
- Scope: `tests/test_herdr_grid_rust.sh`, `tests/test_pipeline_grid.sh`, `tests/test_prompt_worker.sh`, `tests/run.sh`, `tests/test_docs.sh`, `crates/quaqua/tests/worker_unit.rs`, và các chuỗi ref trong `crates/quaqua/src/` (chỉ message/USAGE/comment).

## Changes
- **F1 [tests/test_herdr_grid_rust.sh]**: Bổ sung hermetic stub herdr (`$T/bin/herdr`) xử lý lệnh `api snapshot` bằng fixture `$HERDR_STUB_SNAPSHOT`, export `HERDR_BIN_PATH="$T/bin/herdr"` và `HERDR_SOCKET_PATH="$T/herdr.sock"`. Suite chạy độc lập với daemon herdr bên ngoài, pass cả 2 trường hợp matching tab và foreign tab mà không đụng daemon thật hay bị 60s timeout.
- **F2 [crates/quaqua/tests/worker_unit.rs]**: Chuyển 2 test grid timeout (`test_spawn_worker_grid_timeout_fails` và `test_cleanup_worker_grid_timeout_fails`) từ backdoor cũ `HERDR_STUB_SNAPSHOT` sang fail-closed hermetic `HERDR_BIN_PATH=/dev/null`. Loại bỏ hoàn toàn hazard chạy repair thật trên daemon herdr hoặc move pane của cos worker khi idle.
- **F3 [tests/run.sh]**: Cập nhật `slow_cover` của `test_prompt_worker.sh` từ file cũ đã tách `crates/quaqua/src/worker/prompt.rs` sang thư mục module `crates/quaqua/src/worker/prompt/`, đồng bộ với mục pipeline.
- **F4 [crates/quaqua/src/ & tests/test_docs.sh]**:
  - Dọn sạch các chuỗi tham chiếu đến script bash đã xoá trong `crates/quaqua/src/`:
    - `crates/quaqua/src/gate/mod.rs`: `BLOCK_MESSAGE` bỏ danh sách 4 script cũ, chỉ hướng dẫn qua `quaqua worker spawn / quaqua lead spawn / quaqua worker cleanup / quaqua lead cleanup` hoặc `quaqua grid reconcile`.
    - `crates/quaqua/src/pipeline/phases/mod.rs`, `plan.rs`, `review.rs`, `spawn.rs`: thay `pipeline.sh --resume` → `quaqua pipeline --resume` và `prompt-worker.sh --resume` → `quaqua worker prompt --resume`.
    - `crates/quaqua/src/pipeline/mod.rs`, `worker/spawn.rs`, `lead/spawn.rs`, `lead/cleanup.rs`, `worker/prompt/mod.rs`, `worker/cleanup.rs`, `cap.rs`, `init.rs`: cập nhật `USAGE` và doc comment sang lệnh binary `quaqua ...`.
  - Siết `tests/test_docs.sh:62`: bỏ `crates/` khỏi regex loại trừ (`grep -v -E '^(scripts/hooks/|scripts/auditor/|docs/research/)'`). `test_docs.sh` nay quét cả `crates/quaqua/src/` và xác nhận 0 dangling ref.
- **Bổ sung: test_pipeline_grid.sh & test_prompt_worker.sh & tests/run.sh**:
  - `tests/test_pipeline_grid.sh`: Chuyển mốc nhận diện spawn từ file script cũ `^spawn sude-pgrid` sang `agent start sude-pgrid` (do Rust pipeline spawn worker in-process qua `herdr.agent_start`); xoá `$CALLS` trước section 1b để tránh prompt từ section 1 gây false positive; export `HERDR_SOCKET_PATH`.
  - `tests/test_prompt_worker.sh`: Ghim `HERDR_BIN_PATH="$T/bin/herdr"` và `HERDR_SOCKET_PATH="$T/herdr.sock"` trong runner; stub `api snapshot` đọc fixture từ `HERDR_STUB_SNAPSHOT` giúp pre-prompt grid check trả rc=1 trước khi dispatch prompt, loại bỏ phụ thuộc môi trường gây exit 3 (watchdog timeout).
  - `tests/run.sh`: Bọc vòng lặp in output failure `${failed[@]}` trong `if ((${#failed[@]})); then ... fi` để tránh lỗi bash 3.2 `failed[@]: unbound variable` dưới `set -u` khi toàn bộ test suite pass.

## Git Diff Stat
```
crates/quaqua/src/cap.rs                    |  1 -
crates/quaqua/src/gate/mod.rs               |  2 +-
crates/quaqua/src/init.rs                   |  7 +++----
crates/quaqua/src/lead/cleanup.rs           |  9 ++++-----
crates/quaqua/src/lead/spawn.rs             |  9 ++++-----
crates/quaqua/src/pipeline/mod.rs           | 14 +++++++-------
crates/quaqua/src/pipeline/phases/mod.rs    | 12 ++++++------
crates/quaqua/src/pipeline/phases/plan.rs   |  2 +-
crates/quaqua/src/pipeline/phases/review.rs |  4 ++--
crates/quaqua/src/pipeline/phases/spawn.rs  |  4 ++--
crates/quaqua/src/worker/cleanup.rs         |  3 +--
crates/quaqua/src/worker/prompt/mod.rs      |  7 +++----
crates/quaqua/src/worker/spawn.rs           |  5 ++---
crates/quaqua/tests/worker_unit.rs          | 12 ++++--------
tests/run.sh                                | 12 +++++++-----
tests/test_docs.sh                          |  2 +-
tests/test_herdr_grid_rust.sh               | 27 +++++++++++++++++++++++++++
tests/test_pipeline_grid.sh                 |  4 +++-
tests/test_prompt_worker.sh                 | 14 ++++++++++----
 19 files changed, 88 insertions(+), 62 deletions(-)
```

## Results Across Environments

| Item | Env (a) Không có herdr (`env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin`) | Env (b) Có herdr thật ambient (`HERDR_BIN_PATH=/opt/zerobrew/bin/herdr`) |
|---|---|---|
| `cargo test --locked -p quaqua` | **PASS**: 162 passed (10 suites) | **PASS**: 162 passed (10 suites) |
| `tests/test_herdr_grid_rust.sh` (F1) | **PASS**: 30 differential + 4 grid cases | **PASS**: 30 differential + 4 grid cases |
| `worker_unit.rs` grid timeout (F2) | **PASS**: 2 passed | **PASS**: 2 passed |
| `tests/test_docs.sh` (F4 siết chặt) | **PASS**: 0 dangling scripts/*.sh in crates/ | **PASS**: 0 dangling scripts/*.sh in crates/ |
| `tests/test_pipeline_grid.sh` | **PASS**: builder/reviewer grid check in-process | **PASS**: builder/reviewer grid check in-process |
| `tests/test_prompt_worker.sh` | **PASS**: pre-prompt check rc=1 | **PASS**: pre-prompt check rc=1 |
| `bash tests/run.sh` | **PASS**: 22 passed, 0 failed, 6 skipped | **PASS**: 22 passed, 0 failed, 6 skipped |

## Evidence: Herdr Isolation in Environment (b)
Snapshot trước và sau khi chạy toàn bộ test suite trong môi trường (b) khẳng định tuyệt đối không có rò rỉ:
- Tổng số agents: 3 trước = 3 sau
  - `w9:p2W` tab `w9:t26` ws `w9` (opencode): pane_id, tab_id, workspace_id giữ nguyên
  - `w9:p4A` tab `w9:t2E` ws `w9` (suhuynh-finalreview): pane_id, tab_id, workspace_id giữ nguyên
  - `w9:p4B` tab `w9:t2E` ws `w9` (sude-finalfix): pane_id, tab_id, workspace_id giữ nguyên
- Tổng số panes: 3 trước = 3 sau, mọi pane location giữ nguyên.
- Không pane nào bị move, không tab/workspace nào được tạo mới, không agent nào bị spawn ngoài sandbox test.

## Risks
- None: Toàn bộ thay đổi giới hạn chặt chẽ trong các chuỗi hiển thị / USAGE / comment và thiết lập sandbox hermetic trong test suite, giữ nguyên logic nghiệp vụ của binary.

## Blockers
none

## Lessons
- Khi bóc tách hoặc xóa backdoor môi trường khỏi mã nguồn Rust, mọi test suite (kể cả bash lẫn unit tests) phải được ghim hermetic `HERDR_BIN_PATH` và `HERDR_SOCKET_PATH` trỏ vào stub/tempdir thay vì dựa vào PATH hoặc môi trường ambient.
