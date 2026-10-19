# Báo cáo Triển khai: Wave 5 Architecture Restructure & Bug Fixes SDLC Pipeline

## Changes
- **Decompose `pipeline/mod.rs` (Goal 1)**:
  - Tách cỗ máy trạng thái monolithic 1.406 LOC và `handle_pipeline` 982 dòng thành cấu trúc module hóa rõ ràng:
    - `crates/quaqua/src/pipeline/state.rs` (123 LOC): Chứa struct `PipelineState`, phương thức `load` và `save` nguyên tử.
    - `crates/quaqua/src/pipeline/phases/mod.rs` (600 LOC): Chứa `PipelineContext`, helper logic (`wait_file`, `finish`, `check_clock`, `check_impl`, `has_real_blockers`, `blocked_outcome`, `pipeline_finished`, `harness_of_kind`, `select_default_reviewer`, `log_challenge`, `log_blockers`).
    - `crates/quaqua/src/pipeline/phases/spawn.rs` (105 LOC): Handler khởi tạo worktree builder và split pane.
    - `crates/quaqua/src/pipeline/phases/plan.rs` (52 LOC): Handler dispatch plan brief và pause tại `plan-ready` (exit 5).
    - `crates/quaqua/src/pipeline/phases/build.rs` (110 LOC): Handler build brief, kiểm tra commit git, và đánh giá size gate cho review.
    - `crates/quaqua/src/pipeline/phases/review.rs` (218 LOC): Handler spawn reviewer tại HEAD SHA, wait report, dọn dẹp worktree reviewer, và phân loại verdict.
    - `crates/quaqua/src/pipeline/phases/fix.rs` (64 LOC): Handler fix brief với context review findings và kiểm tra commit fix.
    - `crates/quaqua/src/pipeline/mod.rs` (569 LOC): Dispatcher mỏng phân tích CLI args (`--resume` / initial start) và vòng lặp match 25 dòng điều phối `StepResult` (`Continue` / `Done`).
  - Toàn bộ các submodule phase đều strictly <300 LOC.
- **Xóa Shell Trampolining (Goal 2)**:
  - Xóa bỏ `pub fn find_engine_script(name: &str) -> Option<PathBuf>` tại `crates/quaqua/src/worker/cleanup.rs` (-52 LOC dead weight).
  - Loại bỏ hoàn toàn 5 nhánh `Command::new("bash")` cho spawn/prompt/cleanup trong pipeline (`pipeline/mod.rs`), chuyển sang gọi trực tiếp in-process:
    - `crate::worker::spawn::spawn_worker` (bảo toàn biến môi trường `PIPELINE_TASK`, `QUAQUA_OWNER`, `GRID_SKIP`).
    - `crate::worker::prompt::prompt_worker` (bảo toàn biến môi trường `PROMPT_WORKER_BUDGET_SEC`).
    - `crate::worker::cleanup::cleanup_worker` (bảo toàn biến môi trường `PIPELINE_TASK`, `GRID_SKIP`).
- **Xóa Bảng Hardcode `harness_of` (Goal 3)**:
  - Xóa bỏ bảng tra cứu mapping cứng `harness_of` (`crates/quaqua/src/pipeline/mod.rs:60-67`).
  - Thay bằng `harness_of_kind(kind: &str, cfg: &Config) -> Option<&str>` tra cứu 100% động qua `cfg.kinds.get(kind)`.
  - Cập nhật unit test `test_sude_harness_mapped_to_omp` trong `crates/quaqua/tests/worker_unit.rs` để test qua `load_team_config`.
- **Fix `blocked_outcome` False-Positive (Goal 4)**:
  - Tái cấu trúc thuật toán `has_real_blockers`: bóc tách tiền tố bullet (`-`, `*`, `+`), khoảng trắng, và markdown emphasis (`**`, `*`, `_`, `` ` ``).
  - Bóc tách mệnh đề khai báo đứng trước các dấu phân cách câu/danh sách (`.`, `:`, `;`, `—`, `…`, ` - `).
  - Khi mệnh đề khai báo là `none` (case-insensitive), ghi nhận là ghi chú thông tin hợp lệ (như `## Blockers\n- None. All done...`, `## Blockers\n- None. All 9 tests passed.`, `## Blockers\n- **None**. All done`, `## Blockers\n- None: all clear`), không coi là blocker (`continue`).
  - Các trường hợp mô tả blocker thật sự (như `## Blockers\n- None of the tests pass`, `## Blockers\n- cannot plan without spec`) được phân loại chính xác là blocker thật (`return true`).
- **Per-Phase Budget Reset (Goal 5)**:
  - Trong `wait_file`, khi bắt đầu một phase mới (`state.prompted == 0`):
    - Ghi ngân sách phase mới vào `runs/.<worker>.budget`.
    - Dọn dẹp/reset `.elapsed`, `.grace`, `.nudged`, `.autoext` của worker để phase mới nhận trọn vẹn allowance mà không bị chết đói bởi `rem = max(budget - elapsed, 1) = 1s`.
  - Khi resume giữa chừng cùng một phase (`state.prompted == 1`), giữ nguyên vẹn `.elapsed` và `.budget` để bảo toàn chính xác ngữ nghĩa của `--resume`.
- **Tests Update**:
  - `crates/quaqua/tests/worker_unit.rs`: Cập nhật `test_sude_harness_mapped_to_omp` và bổ sung test cases kiểm tra `has_real_blockers` cho informational notes.
  - `crates/quaqua/tests/pipeline_prompt.rs`: Cập nhật fixture để mock `herdr` trực tiếp in-process thay cho shim shell cũ, bổ sung test case `pipeline_per_phase_budget_reset_grants_fresh_allowance`.

## Evidence
- **Commit SHA**: `7d3dc4e9fa80b33cecf3c530e461f8027a4128f7` trên branch `quaqua/sude-w5` (base `quaqua/clean-integ`).
- **`git diff --stat`**:
  ```
   crates/quaqua/src/pipeline/mod.rs           | 1227 +++++----------------------
   crates/quaqua/src/pipeline/phases/build.rs  |  110 +++
   crates/quaqua/src/pipeline/phases/fix.rs    |   64 ++
   crates/quaqua/src/pipeline/phases/mod.rs    |  600 +++++++++++++
   crates/quaqua/src/pipeline/phases/plan.rs   |   52 ++
   crates/quaqua/src/pipeline/phases/review.rs |  218 +++++
   crates/quaqua/src/pipeline/phases/spawn.rs  |  105 +++
   crates/quaqua/src/pipeline/state.rs         |  123 +++
   crates/quaqua/src/worker/cleanup.rs         |   52 --
   crates/quaqua/tests/pipeline_prompt.rs      |  217 +++--
   crates/quaqua/tests/worker_unit.rs          |   13 +-
   11 files changed, 1631 insertions(+), 1150 deletions(-)
  ```
- **Grep Proofs**:
  - `git grep "find_engine_script"` → **0 matches**.
  - `git grep "fn harness_of\b"` → **0 matches** (100% qua `harness_of_kind` & `cfg.kinds`).
  - `git grep 'Command::new("bash")' crates/quaqua/src/pipeline/` → **0 matches** (100% in-process calls).
- **Test cho (4)**: Fixture report với `## Blockers\n- None. All done...` không bị coi là blocker; `## Blockers\n- None of the tests pass` bị coi là blocker (đã chứng minh bằng failing-first và passing-after trong `test_pipeline_blockers_none_exemption`).
- **Test cho (5)**: `pipeline_per_phase_budget_reset_grants_fresh_allowance` chứng minh phase mới nhận allowance mới, reset `.grace` và `.elapsed`, không kế thừa `rem=1s`.

## Test cmd+result
- `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`: **148 passed; 0 failed** (10 test suites, finished in 4.34s).
- `cargo test --locked --test pipeline_prompt --test worker_unit`: **55 passed; 0 failed** (2 suites, finished in 3.84s):
  - `tests/pipeline_prompt.rs`: 9 passed; 0 failed.
  - `tests/worker_unit.rs`: 46 passed; 0 failed.

## Risks
- **Rủi ro rò rỉ biến môi trường khi gọi in-process (`PIPELINE_TASK`, `QUAQUA_OWNER`, `GRID_SKIP`)**:
  - *Biện pháp đã xử lý*: Toàn bộ các biến môi trường được lưu lại giá trị cũ và khôi phục/xóa sạch bằng pattern scoped restore ngay sau khi gọi `spawn_worker` và `cleanup_worker`.
- **Rủi ro ảnh hưởng ngữ nghĩa `--resume` khi reset budget**:
  - *Biện pháp đã xử lý*: Logic reset chỉ thực hiện khi `state.prompted == 0` (bước chuyển phase mới). Khi resume giữa phase (`state.prompted == 1`), toàn bộ `.elapsed` và `.budget` được giữ nguyên vẹn.

## Blockers
none

## Lessons
none
