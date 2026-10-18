# Báo cáo Thực thi FIX Phase — Task 'w6' (Sau Review Round 1 của suhuynh-w6r1)

- **Worker**: `sude-w6`
- **Branch**: `quaqua/sude-w6` (commits `1c10944` + `6b65929`)
- **Review Report**: `~/quaqua/runs/suhuynh-w6r1.md`

---

## 1. Kết quả Phản hồi & Xử lý 4 Findings của Reviewer

### 1.1. Xử lý Trực tiếp trong Scope: Findings F3 & F4 (Commit `6b65929`)
- **F3 (`hook.rs:298-308`: Hook thêm owner filter mới ngoài legacy semantics)**:
  - *Thực tế*: Hook cũ xử lý candidate workers trên toàn workspace bất kể owner nào kích hoạt event; việc thêm `owner_env` từ ambient `$QUAQUA_OWNER` vô tình làm hook bỏ qua pane của owner khác.
  - *Khắc phục*: Truyền `owner_filter = None` vào `filter_ops_for_check(&ops, &snap, None, None, None)` trong `reconcile_step`. Hook nay tiếp tục xử lý toàn bộ candidate workers trong workspace một cách nhất quán với legacy semantics.
- **F4 (`hook.rs:286-314`: Thay đổi thời điểm xóa `failed_ops.json` / `failed_snap.json`)**:
  - *Thực tế*: Đặt việc xóa file failed ops khi `non_deferred.is_empty()` làm mất bộ nhớ backoff khi các ops còn lại rơi vào trạng thái deferred hoặc đang bị backoff bỏ qua.
  - *Khắc phục*: Khôi phục khối `if ops.is_empty() { remove_file... }` trước shadow mode (đúng nguyên bản), và không xóa file khi `non_deferred.is_empty()`, bảo toàn trọn vẹn cơ chế backoff signature.

### 1.2. ## Challenge: Phản biện Kỹ thuật Findings F1 & F2 (Phụ thuộc Ngoài Scope)
Reviewer `suhuynh-w6r1` ghi nhận F1 [CAO] và F2 [TRUNG BÌNH] liên quan đến việc các test bên ngoài scope bị gãy hoặc mất tính hermetic khi backdoor `HERDR_STUB_SNAPSHOT` bị xóa. Chúng tôi giữ nguyên kết luận phản biện kỹ thuật vì các lý do sau:

1. **Ràng buộc Mâu thuẫn Trực tiếp giữa Brief và Hệ quả của F1/F2**:
   - Brief Wave 6 bắt buộc 3 điều:
     (a) `(3) xoá backdoor test HERDR_STUB_SNAPSHOT khỏi CliHerdrDriver`;
     (b) `grep chứng minh HERDR_STUB_SNAPSHOT = 0 trong src/`;
     (c) Ràng buộc phạm vi nghiêm ngặt: *"Scope: src/herdr.rs, src/grid/*.rs, src/grid/mod.rs, src/lib.rs, crates/quaqua/tests/grid.rs, crates/quaqua/tests/common/* (nếu tạo). File ngoài scope chứa import bị ảnh hưởng → ghi ## Challenge với evidence, không tự đụng."*
   - `tests/test_herdr_grid_rust.sh` (dòng 91, 105) và `crates/quaqua/tests/worker_unit.rs` (dòng 1601, 1652) là các file **nằm ngoài scope được phép sửa**. Cả hai đều trực tiếp tiêu thụ biến môi trường backdoor `HERDR_STUB_SNAPSHOT`.
   - Nếu worker tự ý sửa file ngoài scope -> vi phạm Hard Constraints.
   - Nếu worker giữ lại backdoor `HERDR_STUB_SNAPSHOT` trong `src/` để các file ngoài scope pass -> vi phạm trực tiếp Mục tiêu (3) và điều kiện kiểm thử `grep HERDR_STUB_SNAPSHOT = 0 trong src/`.
2. **Tính Hợp lệ của Test Suite Nội bộ**:
   - Dưới môi trường kiểm chuẩn của brief (`env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`), toàn bộ 10 test suites đều **PASS 150/150 (0 fail)**.
   - Toàn bộ 53/53 tests trong `tests/grid.rs` đều xanh tuyệt đối.
3. **Đề xuất Hành động Tiếp theo cho Lead**:
   - F1 và F2 là hệ quả kiến trúc tất yếu khi cắt bỏ backdoor trong production driver trong khi các test consumer bên ngoài chưa được cấp phép di trú trong cùng wave.
   - Lead xử lý dứt điểm khi mở scope cho task test-migration:
     - Với `test_herdr_grid_rust.sh`: Bổ sung cờ CLI tường minh `--snapshot <path>` cho `quaqua grid reconcile`, hoặc stub `api snapshot` qua `HERDR_BIN_PATH` (giống pattern của `tests/test_spawn_worker.sh`).
     - Với `worker_unit.rs`: Set `HERDR_BIN_PATH=/dev/null` tại 2 test case để bảo đảm fail-path độc lập với ambient daemon.

---

## 2. Evidence

### 2.1. Commit History & Git Diff Stat
- **Commit 1**: `1c10944` — *refactor(grid): wave 6 unify HerdrDriver into HerdrOps, isolate FakeHerdrDriver, remove HERDR_STUB_SNAPSHOT, align reconcile predicate*
- **Commit 2**: `6b65929` — *fix(grid): resolve F3 and F4 in hook reconcile_step (preserve owner-agnostic semantics and backoff timing)*
- **Tổng Diff Stat (`HEAD~2`..`HEAD`)**:
```text
 crates/quaqua/src/grid/apply.rs   | 203 +-------------------------------------
 crates/quaqua/src/grid/hook.rs    |  61 +++---------
 crates/quaqua/src/grid/mod.rs     |   2 +-
 crates/quaqua/src/herdr.rs        | 106 ++++++++++++++++++--
 crates/quaqua/tests/common/mod.rs | 113 +++++++++++++++++++++
 crates/quaqua/tests/grid.rs       |  66 ++++++++++++-
 6 files changed, 293 insertions(+), 258 deletions(-)
```

### 2.2. Kiểm tra Tuyệt đối Không còn Backdoor trong src/
```bash
git grep "HERDR_STUB_SNAPSHOT" crates/quaqua/src/
# Kết quả: 0 hits
```

### 2.3. Test Results
- Suite Grid:
```bash
cargo test --locked --test grid
# Kết quả: 53 passed; 0 failed; 0 ignored; finished in 0.21s
```
- Full Workspace:
```bash
env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked
# Kết quả: cargo test: 150 passed (10 suites, 0.00s)
```

---

## Changes
- `crates/quaqua/src/grid/hook.rs`: Sửa `reconcile_step` truyền `owner_filter = None` vào `filter_ops_for_check` (giữ nguyên legacy owner-agnostic semantics, giải quyết F3) và khôi phục logic xóa failed ops/snap files chỉ khi `ops.is_empty()` (bảo toàn backoff memory, giải quyết F4).

## Test cmd+result
- `cargo test --locked --test grid` -> **53 passed; 0 failed**
- `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` -> **150 passed; 0 failed**
- `git grep "HERDR_STUB_SNAPSHOT" crates/quaqua/src/` -> **0 hits**

## Risks
- Các shell tests ngoài scope (`test_herdr_grid_rust.sh`, `test_prompt_worker.sh`) và 2 test trong `worker_unit.rs` cần được Lead lên kế hoạch di trú stub sang `HERDR_BIN_PATH` hoặc cờ `--snapshot` như đã nêu chi tiết trong mục `## Challenge`.

## Blockers
none

## Lessons
- Khi cắt bỏ một backdoor môi trường trong production code, các test phụ thuộc gián tiếp ngoài scope phải được bảo vệ bằng wrapper/shim chính thức hoặc cách ly bằng stub binary thay vì duy trì backdoor trong binary chính — 2026-10-18, runs/sude-w6.md
