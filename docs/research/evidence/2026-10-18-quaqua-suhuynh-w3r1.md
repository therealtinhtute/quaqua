# Review — Wave 3 (quaqua/sude-w3 @ f33ddfa) — round 1/2

- **Diff**: `20cd4d2..f33ddfa` (`HEAD` = tip `quaqua/sude-w3`; `HEAD^` = 20cd4d2), 8 files, +386/−304. Đúng 8/8 file trong scope brief + Resolution; không scope creep.
- **Phương pháp**: đọc đủ diff + file cuối; so từng hàm được dời với bản gốc ở base; chạy lại test; tự dựng phép so old-awk ↔ transform mới trên INDEX.md thật; chạy fast gate toàn bộ.
- **Branch**: chưa push (`git branch -r --contains HEAD` rỗng).

## Xác nhận đạt brief
- (1) `crates/quaqua/src/launch.rs` mới (170 dòng) chứa `Launch`, `LaunchCmd`, `resolve_launch`, `build_launch_cmd`, `claude_trust_dialog_up`; `lib.rs:13` khai báo `pub mod launch;`. So byte-với bản base: 4/5 định danh **identical**; `claude_trust_dialog_up` chỉ đổi `fn` → `pub fn` (`launch.rs:171`). Không đổi args/model/effort per harness.
- (1b) Lead import thẳng `crate::launch::` (`lead/spawn.rs:41,58,239`); `git grep 'crate::worker' src/lead` = 0 hit, `git grep 'crate::lead' src/worker` = 0 hit ⇒ phụ thuộc `lead → worker` đã phá. `worker/spawn.rs:11` re-export (`pub use crate::launch::{…}`) giữ tương thích cho `pipeline/`, `taskforce/` (chúng vẫn đi qua `worker::spawn` cho các API khác — hợp lệ).
- (2) Hai khối awk thay bằng `update_project_index_owner` (`lead/spawn.rs:338-384`; gọi từ `lead/spawn.rs:321` và `lead/cleanup.rs:235`). Cơ chế lock GIỮ NGUYÊN: `projects/INDEX.md.lock.d`, 200 × 50 ms (`lead/spawn.rs:348-366`), nhả lock sau closure kể cả khi lỗi (`:382`); ghi atomic `INDEX.md.tmp.<pid>` + rename (`:372-378`). `git grep 'Command::new("awk")' crates/quaqua/src` = **0 hit** (trước: 2 block + `lead/cleanup.rs`).
- (3) Unit test trên fixture: `worker_unit.rs:1937` (transform + round-trip byte-identical + slug không tồn tại), `:1975` (fixture file thật: update/reset/không sót tmp-lock, INDEX thiếu → Err), `:2018` (8 thread đua). 3 test mới, không xoá/sửa thân test cũ (diff worker_unit = đổi import + append).
- (4) Resolution: bỏ stub awk ở 2 test, dùng đua thật + ghi chú (`test_cleanup_lead.sh:143-145`, `test_spawn_lead.sh:253-255`); **không** thêm hook test-only nào vào `src/`.
- **Byte-identity (kiểm độc lập)**: chạy chương trình awk cũ (lấy từ base) vs bản mô phỏng đúng thuật toán Rust trên `~/quaqua/projects/INDEX.md` thật (slug `zimage`→`zimage-lead`, `llmhub`→`cos`) → `diff` **IDENTICAL**. File thật: LF (0 ký tự `\r`), mọi dòng bảng ≥4 field.
- **Test tôi chạy lại**: `env -u RUSTUP_TOOLCHAIN -u HERDR_BIN_PATH cargo test --locked` → **150 passed / 0 failed** (+ doc-test ok); `bash tests/test_cleanup_lead.sh` ×3 → PASS; `bash tests/test_spawn_lead.sh` ×3 → PASS; fast gate 25 suite (`tests/test_*.sh` trừ 2 slow; `HERDR_BIN_PATH` unset) → **25 passed / 0 failed**.

## Findings

### F1 — [THẤP] Import thừa trong test ⇒ warning mới
- `crates/quaqua/tests/worker_unit.rs:11` — `pub use`-style import kéo thêm `claude_trust_dialog_up` và `LaunchCmd` nhưng **không test nào dùng** (2 tên chỉ xuất hiện ở dòng 11).
- Bằng chứng: `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo check --locked --all-targets` → `warning: unused imports: 'LaunchCmd' and 'claude_trust_dialog_up'` (1 warning, 0 error). Base: 0 warning ở target này.
- Sửa: bỏ 2 tên khỏi dòng import (hoặc thêm test thật cho `claude_trust_dialog_up`).

### F2 — [THẤP] "Byte-identical" không đúng với input CRLF và dòng <4 field
- `crates/quaqua/src/lead/spawn.rs:389` — `content.lines()` cắt `\r`; `:392` — `parts.len() >= 4` guard.
- Tái hiện (input dựng bằng `printf`, so với chương trình awk cũ nguyên văn):
  - CRLF: awk giữ `| a-lead |\r\n …`; bản mới xuất `\n` cho **mọi** dòng ⇒ file CRLF bị viết lại toàn bộ line-ending.
  - Dòng 3 field `| a | ~/Lab/a` (thiếu pipe cuối): awk nối thêm ô owner (`| a | ~/Lab/a | a-lead`); bản mới **bỏ qua dòng** (không match).
- Thực tế vô hại: INDEX.md thật là LF và mọi dòng ≥4 field (đã kiểm); awk-vs-mới IDENTICAL trên file thật. Nhưng comment `lead/spawn.rs:335-337` khẳng định byte-identical vô điều kiện ⇒ nên thu hẹp câu chữ (LF, bảng well-formed) hoặc thêm 1-2 assert pin hành vi cho 2 ca trên.
- Không chặn merge nếu Lead chấp nhận — đây là ghi nhận đúng mức "hành vi khác ngoài miền input thực tế", không phải lỗi chức năng.

## Quan sát (không tính finding)
- O1: 2 test .sh giờ chỉ còn **xác suất** bắt race (đúng như Resolution (a) cho phép; ghi chú có trong test). Detector chính là test 8-thread `worker_unit.rs:2018` — cũng không deterministic nhưng tín hiệu cao (tmp cùng-pid + 8 luồng: mất lock ⇒ thiếu row hoặc file rách ⇒ fail). Muốn deterministic: pre-tạo `INDEX.md.lock.d` trong test và assert Err "is locked" (đắt 10 s do timeout).
- O2: delta đường lỗi ngoài danh sách "zero behavior change": bản cũ `let _ = fs::write/rename` (im lặng), bản mới trả `Err` (`lead/spawn.rs:375-378`, `lead/cleanup.rs:235`) ⇒ nếu ghi INDEX fail, `spawn-lead` exit 1 **sau** khi đã start agent, **trước** khi prompt lead. Fail-loudly hợp lý, chỉ ghi nhận.
- O3: `worker/spawn.rs:11` re-export giữ path cũ `crate::worker::spawn::{Launch,…}` sống thêm — có chủ đích (pipeline/taskforce), nhưng "một nhà cho launch" bị mềm; nếu W6 muốn chặt thì xoá re-export sau khi sửa caller.
- O4: môi trường: với `PATH` tối giản của brief **và** `RUSTUP_TOOLCHAIN=stable`, bước doc-test đỏ vì thiếu binary rustdoc (`could not execute process …/stable-…/bin/rustdoc … No such file or directory`) — thuộc toolchain, không do diff: bỏ `RUSTUP_TOOLCHAIN` là doc-test ok; 150/0 vẫn nguyên.

## Risks
- Warning ở F1 sẽ hiện ở mọi `cargo check --all-targets` tiếp theo (dễ bị coi là hồi quy) — sửa 1 dòng.
- F2 chỉ là ranh giới input; không ảnh hưởng INDEX.md thật.
- Nếu test race .sh được dùng làm bằng chứng "lock còn hoạt động" thì bằng chứng đó giờ yếu — nên dẫn `worker_unit.rs:2018` khi báo cáo thay vì 2 test .sh.

## Blockers
none

Verdict: 2 findings
