# sude-testaudit

## Summary
- **Commits**:
  - `c75f40a`: `chore(tests): delete 6 dead self-skip test suites (-1402 lines)`
  - `2bd0732`: `fix(tests): sync bash test assertions with team.toml cap 8`
- **Dead code removal**: 6 self-skip test suites deleted (`test_r3_shims.sh`, `test_r4_shims.sh`, `test_r5_shims.sh`, `test_grid_workers.sh`, `test_herdr_grid_apply.sh`, `test_herdr_grid_hook.sh`).
- **Cap-drift fix**: Updated 3 bash test suites (`tests/test_config.sh`, `tests/test_worker_cap.sh`, `tests/test_taskforce_cap.sh`) from hardcoded cap 4 to cap 8, matching `team.toml` (cap 8) and `crates/quaqua/tests/config.rs`.
- **Suite counts**: Total suites reduced from 28 to 22 (cargo_test + 21 active bash suites).
- **Gate status**: `tests/run.sh --slow` passes cleanly: **22 passed, 0 failed, 0 skip**.

## Audit Table (28 files in tests/)

| File | Dòng | Status | Case thật | Overlap Rust (`crates/quaqua/tests/*.rs`) | Khuyến nghị |
|---|---|---|---|---|---|
| `run.sh` | 117 | RUNNER | n/a (runner) | n/a (`tests/run.sh` điều phối cargo + bash suites) | KEEP (runner chính) |
| `test_auditor.sh` | 361 | LIVE | 13 | Thấp (`auditor.rs` unit tests chỉ check arg parse, không test harness invocation) | KEEP (e2e gate cho auditor harness) |
| `test_cleanup_lead.sh` | 163 | LIVE | 10 | Cao (`worker_unit.rs` test lead cleanup logic, INDEX lock) | MERGE vào Rust e2e |
| `test_cleanup_worker.sh` | 169 | LIVE | 8 | Cao (`worker_unit.rs` test cleanup worktree + branch preservation) | MERGE vào Rust e2e |
| `test_config.sh` | 109 | LIVE | 7 | 100% trùng (`crates/quaqua/tests/config.rs` phủ 20 fixture, get, cap, rule->key) | MERGE / RETIRE (trùng 100%) |
| `test_docs.sh` | 91 | LIVE | 6 | Không trùng (lint repo structure, markdown rules, AGENTS, entrypoints) | KEEP (repo-level lint gate) |
| `test_gate_shim.sh` | 83 | LIVE | 5 | Trung bình (`gate.rs` test tokenizer, suite này test fallback hook khi thiếu binary) | KEEP (bảo vệ shell fallback hook) |
| `test_grid_workers.sh` | 86 | SELF-SKIP | 0 | 100% dead weight (`grid.rs` 33 tests + `test_herdr_grid_rust.sh`) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_herdr_gate.sh` | 145 | LIVE | 6 | Cao (`crates/quaqua/tests/gate.rs` 8 tests bao phủ toàn bộ whitelist/blacklist) | MERGE vào Rust e2e |
| `test_herdr_grid_apply.sh` | 373 | SELF-SKIP | 0 | 100% dead weight (`grid.rs` in-process reconcile thay thế `apply.sh`) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_herdr_grid_hook.sh` | 927 | SELF-SKIP | 0 | 100% dead weight (`grid.rs` in-process hook thay thế `hook.sh`) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_herdr_grid_plan.sh` | 425 | LIVE | 14 | Thấp (oracle độc lập `plugins/herdr-grid/plan.jq`) | KEEP (oracle differential test) |
| `test_herdr_grid_rust.sh` | 178 | LIVE | 7 | Trung bình (differential runner so sánh quaqua vs `plan.jq`) | KEEP (differential gate) |
| `test_init_workspace.sh` | 318 | LIVE | 19 | Cao (`crates/quaqua/tests/init.rs` 13 tests phủ fresh clone, seed, hooks) | MERGE vào Rust e2e |
| `test_install_auditor.sh` | 160 | LIVE | 8 | Không trùng (test installer `scripts/auditor/install-auditor.sh`) | KEEP (test cho installer script) |
| `test_merge_queue.sh` | 154 | LIVE | 4 | Trung bình (test cơ chế rollback `quaqua merge` khi check fail) | KEEP / MERGE |
| `test_pipeline.sh` | 761 | LIVE | 40 | Một phần (`pipeline_prompt.rs` test brief parse; suite này test full state machine) | KEEP (e2e pipeline state gate) |
| `test_pipeline_grid.sh` | 281 | LIVE | 4 | Trung bình (test in-process grid reconciliation của pipeline) | KEEP |
| `test_pre_commit.sh` | 194 | LIVE | 14 | Cao (`crates/quaqua/tests/init.rs` test embedded template + hooksPath) | MERGE vào Rust `init.rs` |
| `test_prompt_worker.sh` | 412 | LIVE | 20 | Cao (`pipeline_prompt.rs` + `worker_unit.rs` test headers, brief, budget) | KEEP / MERGE |
| `test_prompt_worker_watchdog.sh` | 614 | LIVE | 42 | Trung bình (test watchdog engine, fake clock, stall, idle, nudges, exit 0-6) | KEEP (regression watchdog engine) |
| `test_r3_shims.sh` | 5 | SELF-SKIP | 0 | 100% dead weight (wrapper shims obsolete) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_r4_shims.sh` | 5 | SELF-SKIP | 0 | 100% dead weight (wrapper shims obsolete) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_r5_shims.sh` | 5 | SELF-SKIP | 0 | 100% dead weight (wrapper shims obsolete) | **DELETE** (đã xoá ở `c75f40a`) |
| `test_spawn_lead.sh` | 274 | LIVE | 16 | Cao (`worker_unit.rs` test lead spawn, name constraints, branch check) | MERGE vào Rust e2e |
| `test_spawn_worker.sh` | 795 | LIVE | 44 | Cao (`worker_unit.rs` phủ ~70% naming, fallback, kind, worktree reuse) | KEEP / MERGE |
| `test_taskforce_cap.sh` | 173 | LIVE | 6 | Cao (`worker_unit.rs` test cap weights và taskforce reservation) | MERGE vào Rust e2e |
| `test_worker_cap.sh` | 195 | LIVE | 7 | 100% trùng (`worker_unit.rs` test cap refusal, owner isolation, pipeline slot) | MERGE vào Rust e2e |

## Proof of Self-Skip for Deleted Files
1. `tests/test_r3_shims.sh`, `tests/test_r4_shims.sh`, `tests/test_r5_shims.sh` (5 dòng mỗi file):
   - Thân code chỉ gồm 2 dòng: `echo "SKIP: ..."; exit 0`. Hoàn toàn không có case test nào.
2. `tests/test_grid_workers.sh` (86 dòng):
   - Toàn bộ dòng 7-84 chỉ là comment map case sang `crates/quaqua/tests/grid.rs` từ thời R6 migration.
   - Dòng 85-86: `echo "SKIP: grid-workers replaced by quaqua grid reconcile (covered by test_herdr_grid_rust.sh)"; exit 0`. Thân dưới không có code thực thi.
3. `tests/test_herdr_grid_apply.sh` (373 dòng):
   - Dòng 46-49:
     ```bash
     if [[ ! -f "$apply_sh" ]]; then
       echo "SKIP: plugins/herdr-grid/apply.sh has been replaced by quaqua in R6 cutover"
       exit 0
     fi
     ```
   - Biến `$apply_sh` trỏ tới `plugins/herdr-grid/apply.sh` đã bị xoá hoàn toàn trong R6 cutover.
   - Điều kiện `[[ ! -f "$apply_sh" ]]` luôn luôn đúng → file luôn in SKIP và `exit 0` ngay dòng 48. Thân dưới (dòng 50–373) là dead code 100% unreachable.
4. `tests/test_herdr_grid_hook.sh` (927 dòng):
   - Dòng 71-74:
     ```bash
     if [[ ! -f "$hook_src" ]]; then
       echo "SKIP: plugins/herdr-grid/hook.sh has been replaced by quaqua in R6 cutover"
       exit 0
     fi
     ```
   - Biến `$hook_src` trỏ tới `plugins/herdr-grid/hook.sh` đã bị xoá hoàn toàn trong R6 cutover.
   - Dòng 72 in SKIP và dòng 73 `exit 0` ngay lập tức. Toàn bộ dòng 75–927 hoàn toàn unreachable.

`bash -x` trace xác nhận tất cả 6 file dừng ngay ở lệnh `echo SKIP` và `exit 0`, không chạm tới bất kỳ tài nguyên hay case test nào.

## Evidence

### 1. Commits & Git Diffs
- **Commit 1 (`c75f40a`)**: `chore(tests): delete 6 dead self-skip test suites (-1402 lines)`
  ```
   tests/test_grid_workers.sh     |  86 ----
   tests/test_herdr_grid_apply.sh | 374 -----------------
   tests/test_herdr_grid_hook.sh  | 927 -----------------------------------------
   tests/test_r3_shims.sh         |   5 -
   tests/test_r4_shims.sh         |   5 -
   tests/test_r5_shims.sh         |   5 -
   6 files changed, 1402 deletions(-)
  ```
- **Commit 2 (`2bd0732`)**: `fix(tests): sync bash test assertions with team.toml cap 8`
  ```
   tests/test_config.sh        |  4 +--
   tests/test_taskforce_cap.sh | 41 +++++++++++++++---------
   tests/test_worker_cap.sh    | 77 ++++++++++++++++++++++++++++-----------------
   3 files changed, 76 insertions(+), 46 deletions(-)
  ```

### 2. Reference Verification
- `git grep -E 'test_(r3_shims|r4_shims|r5_shims|grid_workers|herdr_grid_apply|herdr_grid_hook)'` → 0 matches.
- `git grep -E '\b(r3_shims|r4_shims|r5_shims|grid_workers|herdr_grid_apply|herdr_grid_hook)\b'` → 0 matches.
- `bash tests/test_docs.sh` → PASS (exit 0).

### 3. Gate Execution
- **Command**:
  ```bash
  env -u HERDR_BIN_PATH -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin bash tests/run.sh --slow
  ```
- **Output**:
  ```
  PASS cargo_test
  PASS test_auditor.sh
  PASS test_cleanup_lead.sh
  PASS test_cleanup_worker.sh
  PASS test_config.sh
  PASS test_docs.sh
  PASS test_gate_shim.sh
  PASS test_herdr_gate.sh
  PASS test_herdr_grid_plan.sh
  PASS test_herdr_grid_rust.sh
  PASS test_init_workspace.sh
  PASS test_install_auditor.sh
  PASS test_merge_queue.sh
  PASS test_pipeline_grid.sh
  PASS test_pipeline.sh
  PASS test_pre_commit.sh
  PASS test_prompt_worker_watchdog.sh
  PASS test_prompt_worker.sh
  PASS test_spawn_lead.sh
  PASS test_spawn_worker.sh
  PASS test_taskforce_cap.sh
  PASS test_worker_cap.sh

  22 passed, 0 failed (of 22)
  ```

### 4. Comparison Before vs After
| Metric | Trước dọn | Sau dọn & fix cap | Thay đổi |
|---|---|---|---|
| Tổng số suite phát hiện | 28 (cargo + 27 bash) | 22 (cargo + 21 bash) | −6 suites (xoá 6 file dead) |
| Số suite PASS | 19 | 22 | +3 (đồng bộ cap 8 giúp 3 suite xanh) |
| Số suite SKIP | 6 | 0 | −6 (xoá sạch dead SKIP suites) |
| Số suite FAIL | 3 | 0 | −3 (0 failure) |
| Tổng dòng code `tests/*.sh` | 7,573 | 6,201 | −1,372 dòng net (−1,402 dead, +30 test data cap 8) |

## Proposals for Remaining Bash Tests (Đề xuất, không thực thi)

### Bảng phân loại và lộ trình merge/port
| Nhóm | Suites | Dòng | Đánh giá & Đề xuất |
|---|---|---|---|
| **Nhóm 1: Merge ngay vào Rust (Trùng 100% với unit test)** | `test_config.sh`, `test_worker_cap.sh`, `test_taskforce_cap.sh`, `test_pre_commit.sh` | 701 | **Khuyến nghị RETIRE / MERGE**: Toàn bộ logic đã có test tương ứng hoàn chỉnh trong `crates/quaqua/tests/{config,worker_unit,init}.rs`. Giữ các file bash này có nguy cơ drift assertion lặp lại khi đổi config trong tương lai. |
| **Nhóm 2: Gom vào Rust integration test** | `test_cleanup_worker.sh`, `test_cleanup_lead.sh`, `test_spawn_lead.sh`, `test_gate_shim.sh`, `test_herdr_gate.sh`, `test_init_workspace.sh` | 1,152 | **Khuyến nghị MERGE dần theo đợt**: Các suite này chủ yếu test CLI wrapper và file state đơn giản, có thể chuyển thành Rust `assert_cmd` integration tests để chạy chung trong `cargo test --locked`. |
| **Nhóm 3: Giữ nguyên trong Bash (E2E & Heavy Orchestration)** | `test_pipeline.sh`, `test_spawn_worker.sh`, `test_prompt_worker_watchdog.sh`, `test_prompt_worker.sh`, `test_pipeline_grid.sh`, `test_merge_queue.sh`, `test_auditor.sh`, `test_install_auditor.sh`, `test_docs.sh`, `test_herdr_grid_plan.sh`, `test_herdr_grid_rust.sh` | 4,348 | **Khuyến nghị KEEP**: Mô phỏng sâu tương tác shell, herdr daemon IPC, tmux/herdr pane events, virtual clock watchdog, và oracle `plan.jq`. Viết lại toàn bộ bằng Rust sẽ cực kỳ tốn kém và khó debug. |

### Verdict
**KHÔNG ĐÁNG port toàn bộ:** Chi phí viết lại ~6k dòng bash e2e sang Rust đòi hỏi thêm nhiều crate phụ trợ (`assert_cmd`, `tempfile`, mock subshells, pty) và bảo trì phức tạp hơn; chỉ nên dọn/retire Nhóm 1 (~700 dòng trùng lặp 100% với unit test Rust), còn lại giữ bash làm tầng gate e2e hermetic.

## Blockers
none

## Lessons
- Khi nâng giá trị config toàn cục trong `team.toml` (như cap 4→8), cần grep và cập nhật đồng thời cả Rust integration tests (`tests/*.rs`) lẫn bash suites (`tests/*.sh`) để tránh drift assertion giữa 2 tầng gate.
