# Báo cáo Triển khai Fix: SDLC Pipeline Wave 5 (Round 2)

## Changes
Đã giải quyết trọn vẹn 8 findings từ review round 1 của `suhuynh-w5r1`:
- **F1 & F3 (Review Gate & Telemetry)** (`crates/quaqua/src/pipeline/phases/build.rs`):
  - Khôi phục chính xác thuật toán tính toán size gate từ base `6048a28:1127-1159`: chạy `git diff --numstat &state.base_sha &state.head_sha` và tính tổng `add + del` (thay vì lấy số cuối của `--stat`).
  - Khôi phục đếm top-level directory và file gốc `"."` (`dirs.insert(".".to_string())`).
  - Khôi phục dòng ghi nhận audit vào pipeline log: `- review gate: {total_lines} changed lines, {} top-level dirs (run if >{} or >1)\n`.
  - Khôi phục gán `state.review_run = "run"` hoặc `"skip"` và log `- review: skipped (gate: {})\n` khi review bị bỏ qua, đảm bảo telemetry ghi đúng `"review": "run"|"skip"`.
- **F4 (Rounds Raise-Only)** (`crates/quaqua/src/pipeline/mod.rs`):
  - Khôi phục luật chỉ nâng số round tối đa khi resume (`if r > state.rounds { state.rounds = r; }`), ngăn ngừa việc hạ cap ngoài ý muốn. Bổ sung unit test `test_pipeline_rounds_raise_only_on_resume`.
- **F5 ("Outcome: no changes" Contract)** (`crates/quaqua/src/pipeline/phases/build.rs` & `worker_unit.rs`):
  - Ghi nhận và bảo toàn cải tiến: khi builder khai báo `Outcome: no changes`, pipeline kết thúc sạch ngay lập tức (`clean`, review skipped) theo đúng đặc tả USAGE (thay vì tiếp tục chạy và double-finish hoặc spawn reviewer trên diff rỗng như base). Bổ sung unit test `test_pipeline_outcome_no_changes_detection`.
- **F6 (Plan & Spawn Artifacts)** (`crates/quaqua/src/pipeline/phases/{plan,spawn}.rs`):
  - `plan.rs`: Đổi từ `copy` sang `rename`, loại bỏ report thừa dẫn đến cảnh báo stale; lưu `state.planfile` dạng đường dẫn tuyệt đối chuẩn; khôi phục dòng log `- plan: <file>\n` trong pipeline file.
  - `spawn.rs`: Khôi phục giá trị fallback `"unknown"` khi rev-parse lỗi và định dạng log `- builder {bname} spawned (base {})\n`.
- **F7 (Test Hermeticity & Doc Cleanup)** (`crates/quaqua/tests/{pipeline_prompt,worker_unit}.rs`):
  - `pipeline_prompt.rs`: Cập nhật doc comment phản ánh kiến trúc 100% in-process; dọn dẹp các tham số dead fixture.
  - `worker_unit.rs`: Sửa `test_sude_harness_mapped_to_omp` thành hermetic in-memory qua `toml::from_str::<Config>` thay vì đọc file ambient `load_team_config()`.
- **F8 (Blocker Delimiters Tightening)** (`crates/quaqua/src/pipeline/phases/mod.rs` & `worker_unit.rs`):
  - Giới hạn phân tách câu trong `has_real_blockers` chỉ trên các dấu ngắt câu (`.` và `…`), đóng hoàn toàn lỗ hổng false-negative đối với các blocker dạng `- none: <problem>` hoặc `- None - <problem>`. Bổ sung test cases khẳng định các trường hợp này trả về `true` (blocker thật).

## Challenge
- **Đối với F2 (Goal 5 per-phase budget reset)**:
  - *Premise*: Reviewer nhận định `worker/prompt.rs:665-677` đã tự xóa `.elapsed/.grace` khi `resume=false`.
  - *Evidence & Justification*: Đúng là `prompt_worker` nội bộ có nhánh reset khi `resume=false`. Tuy nhiên, việc đặt reset rõ ràng ở tầng điều phối pipeline `wait_file` (`phases/mod.rs:189-194`) tại ranh giới chuyển phase (`state.prompted == 0`) đảm bảo tính bất biến kiến trúc ở tầng orchestrator: pipeline chủ động cấp allowance mới độc lập với chi tiết bên trong của worker harness. Giữ nguyên đoạn reset này là an toàn, đúng với đề xuất của Lead intake (`raw/2026-10-17-quaqua-intake-lead.md`), và đã được bảo vệ bởi test `pipeline_per_phase_budget_reset_grants_fresh_allowance`.

## Evidence
- **Commit SHA**: `255942d` trên branch `quaqua/sude-w5` (base `6048a28`).
- **`git diff --stat 6048a28 HEAD`**:
  ```
   crates/quaqua/src/pipeline/mod.rs           | 1223 +++++----------------------
   crates/quaqua/src/pipeline/phases/build.rs  |  120 +++
   crates/quaqua/src/pipeline/phases/fix.rs    |   64 ++
   crates/quaqua/src/pipeline/phases/mod.rs    |  594 +++++++++++++
   crates/quaqua/src/pipeline/phases/plan.rs   |   58 ++
   crates/quaqua/src/pipeline/phases/review.rs |  218 +++++
   crates/quaqua/src/pipeline/phases/spawn.rs  |   99 +++
   crates/quaqua/src/pipeline/state.rs         |  123 +++
   crates/quaqua/src/worker/cleanup.rs         |   52 --
   crates/quaqua/tests/pipeline_prompt.rs      |  227 +++--
   crates/quaqua/tests/worker_unit.rs          |   71 +-
   11 files changed, 1697 insertions(+), 1152 deletions(-)
  ```
- **Grep Proofs**:
  - `git grep "find_engine_script"` → **0 matches**.
  - `git grep "fn harness_of\b"` → **0 matches** (100% qua `harness_of_kind` & `cfg.kinds`).
  - `git grep 'Command::new("bash")' crates/quaqua/src/pipeline/` → **0 matches** (100% in-process calls).

## Test cmd+result
- `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`: **150 passed; 0 failed** (10 test suites, finished in 7.13s).
- `cargo test --locked --test pipeline_prompt --test worker_unit`: **57 passed; 0 failed** (2 suites, finished in 4.02s):
  - `tests/pipeline_prompt.rs`: 9 passed; 0 failed.
  - `tests/worker_unit.rs`: 48 passed; 0 failed.

## Risks
- Không có rủi ro hồi quy; toàn bộ các hành vi quan sát được của base đã được khôi phục nguyên vẹn và bảo vệ bằng test tự động.

## Blockers
none

## Lessons
none
