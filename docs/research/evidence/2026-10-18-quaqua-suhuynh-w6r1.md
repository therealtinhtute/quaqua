# Review — Wave 6 restructure (quaqua/sude-w6 @ 1c10944) — round 1/2

- **Diff review**: `20cd4d2..1c10944`, 6 files, +295/−260. Toàn bộ file nằm trong scope brief; không scope creep.
- **Phương pháp**: đọc đủ diff; đối chiếu từng nhánh predicate hook cũ ↔ `plan_with_deferred`/`propagate_deferrals` ↔ `filter_ops_for_check`; chạy lại test theo env brief; tái hiện 2 chế độ môi trường cho các phụ thuộc ngoài scope.

## Xác nhận đạt brief
- (1)+(2): `HerdrDriver` dời vào `src/herdr.rs:176-184`; một impl duy nhất `impl HerdrDriver for Herdr` (`herdr.rs:186-251`); `CliHerdrDriver` = alias (`grid/apply.rs:12-13`). `FakeHerdrDriver` ra khỏi `src/` — chỉ còn `tests/common/mod.rs:10`, sao chép 1:1 ngữ nghĩa so với base; grep `crates/quaqua/src` = 0 hit cho cả `FakeHerdrDriver` lẫn `HERDR_STUB_SNAPSHOT`.
- (3): nhánh stub trong snapshot/move/rename đã xoá sạch (hunk `apply.rs @@ -73,185 +53,6`).
- (4): hook dùng `filter_ops_for_check` (`hook.rs:299-308`). Đối chiếu từng dòng: vòng lặp 44 dòng cũ của hook là bản sao thủ công của `propagate_deferrals` (đã chạy sẵn trong `plan_with_deferred`, `plan.rs:654`), nên phần deferral tương đương; delta thật nằm ở F3/F4.
- **Test (tôi chạy lại)**: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **150 passed / 0 failed** (grid **53/0** = 50 cũ + 3 mới; 0 test cũ bị xoá/sửa thân — diff grid.rs chỉ đổi import + append 3 test). `cargo check --locked --all-targets` → 0 warning.
- Builder đã ghi `## Challenge` đúng yêu cầu brief cho phụ thuộc ngoài scope (sude-w6.md §3) — nhưng hệ quả chưa được xử lý (F1/F2).

## Findings

### F1 — [CAO] `tests/test_herdr_grid_rust.sh` (chạy mặc định trong `tests/run.sh`) gãy vì backdoor bị xoá
- Diff: `herdr.rs:187-199` — snapshot impl mới không còn nhánh `HERDR_STUB_SNAPSHOT`; `apply.rs:12-13` — chỗ impl cũ (có stub) bị thay bằng alias.
- Tái hiện, env không có herdr (`env -u HERDR_BIN_PATH PATH=/usr/bin:/bin`):
  `HERDR_STUB_SNAPSHOT=tests/fixtures/herdr-grid/only-custom-tab.json QUAQUA_OWNER=cos GRID_CHECK_TIMEOUT_SEC=1 target/debug/quaqua grid reconcile --check --tab mono-workers --only sude-isl,suhuynh-islr1 --dry-run`
  → `error: grid check snapshot failed after 1s: cannot run herdr api snapshot...`, rc=1 — trong khi `tests/test_herdr_grid_rust.sh:97` đòi rc=0.
- Tái hiện, env có herdr thật (`HERDR_BIN_PATH=/opt/zerobrew/bin/herdr` — chính env session này): cùng lệnh với `--tab quaqua-workers` → in `[]`, rc=0 — trong khi `tests/test_herdr_grid_rust.sh:111` đòi fail "pending ops".
- `tests/run.sh:55` chạy cargo gate + vòng lặp `:69` chạy script này mặc định (chỉ 2 slow suite bị gate) ⇒ **gate fast của repo đỏ** với bản này; suite `--slow` đỏ thêm `tests/test_prompt_worker.sh:336`.
- Đề xuất: mở scope xử lý trong wave (đưa nguồn snapshot injectable chính thức cho test) hoặc Lead chấp nhận có chủ đích + follow-up ngay — "đã ghi Challenge" là chưa đủ.

### F2 — [TRUNG BÌNH] 2 test `worker_unit.rs` mất tính hermetic, phụ thuộc môi trường; có thể chạm daemon herdr thật
- `tests/worker_unit.rs:1601,1652` set `HERDR_STUB_SNAPSHOT` — nay vô hiệu; driver đi thẳng `Command::new(&self.bin)` (`herdr.rs:188`).
- Với PATH của brief (không có herdr) 2 test vẫn xanh nhưng **vì lý do khác thiết kế**: fail đến từ "grid repair failed: cannot run herdr api snapshot", không phải đường hội tụ/timeout của stub như trước.
- Ở môi trường có `HERDR_BIN_PATH`/herdr trên PATH, `run_grid_hook` → `run_grid_repair` (`cleanup.rs:95-97`) dựng repair thật trên daemon → có thể **move pane thật** (spawn test: `spawn.rs:677`; cleanup test: `cleanup.rs:196`), và kỳ vọng test không còn đảm bảo. Bằng chứng read-only: chính driver đó với `HERDR_BIN_PATH` set + PATH rút gọn đã snapshot thành công daemon thật (F1 case 2).
- Cần sửa khi mở scope: bỏ stub khỏi 2 test, ép `HERDR_BIN_PATH` về binary không tồn tại (giữ fail-path ổn định), hoặc inject driver.

### F3 — [THẤP–TRUNG BÌNH] hook thêm owner filter mới — chưa từng có, chưa test
- `hook.rs:298-308`: khi `QUAQUA_OWNER` set, `filter_ops_for_check` loại op không thuộc owner đó (`reconcile.rs:89-93`; owner thiếu token = "cos"). Hook cũ không lọc owner.
- Chưa xác minh production có set `QUAQUA_OWNER` trong env hook không (plugin hiện chạy `hook.sh` bash; không thấy wiring trong repo). Suite chạy trong session này có `QUAQUA_OWNER=cos` nên nhánh này đã chạy, nhưng không chứng minh được đúng cho owner khác (vd lead).
- Bất đối xứng còn lại: `run_reconcile` mặc định owner `"cos"` khi env unset (`reconcile.rs:146-150`), hook truyền `None` (không lọc) — hai semantics owner vẫn khác khi env unset; nếu mục tiêu là "một predicate" thì cần chốt semantics này.

### F4 — [THẤP] đổi thời điểm xoá `failed_ops.json`/`failed_snap.json` (hành vi production, không nêu trong report)
- Diff hunk `hook.rs @@ -284,10 +285,6 @@` bỏ khối `if ops.is_empty() { remove_file... }`; nay xoá khi `non_deferred.is_empty()` (`hook.rs:310-314`).
- Delta: (a) ops không rỗng nhưng toàn deferred → nay xoá (trước giữ); (b) shadow mode + ops rỗng → nay không xoá nữa (trước xoá); (c) ops bị owner filter loại hết → nay xoá + return (trước apply).
- Ảnh hưởng backoff thực tế nhỏ (signature đổi khi status đổi), nhưng brief yêu cầu zero behavior change ngoài backdoor ⇒ cần ghi nhận/khôi phục tương đương hoặc test pin lại.

## Quan sát (không tính finding)
- O1: `HerdrOps` front nay cũng đọc `HERDR_BIN_PATH` (`herdr.rs:37-49`) — trước chỉ `CliHerdrDriver` đọc. Chỉ đổi khi env set; tại đây env trỏ đúng herdr nên vô hại, nhưng là delta ngoài backdoor.
- O2: "hợp nhất" thực tế = 1 type impl 2 trait (`herdr.rs:176` + `HerdrOps`), không phải gộp còn 1 trait — hợp lệ theo latitude của brief ("tự quyết với evidence"); chỉ nên mô tả đúng như vậy.
- O3: test hội tụ mới (`tests/grid.rs:1626`) chạy live mode thật (`edge_state_dir` ghi shadow=0), không bị vacuous; 3 test mới đúng chỗ trống của wave.

## Risks
- Merge nguyên trạng ⇒ `tests/run.sh` fast đỏ (F1); `--slow` đỏ thêm (test_prompt_worker.sh:336).
- `cargo test` trong env có HERDR_BIN_PATH/daemon thật ⇒ F2 (side effect lên grid thật + flaky).

Verdict: 4 findings
