# Review — Wave 3 (quaqua/sude-w3 @ 463732f) — round 2/2

- **Diff**: `20cd4d208b4711b5e219f00c83b0fb7ec6f78012..463732f` (HEAD = tip `quaqua/sude-w3`), 8 files, +394/−305. Đúng 8/8 file trong scope brief + Resolution; không scope creep. Commit fix round-1 (`463732f`) chỉ chạm 2 file: doc comment `lead/spawn.rs` + import/1 assert trong `worker_unit.rs`.
- **Phương pháp**: đọc đủ diff cả 2 commit; so khối code dời với base bằng script; so bản transform Rust với chương trình awk cũ (trích nguyên văn từ base) trên `~/quaqua/projects/INDEX.md` thật; build binary từ worktree này và chạy lại toàn bộ gate; kiểm greps, scope, trạng thái branch.
- **Branch**: chưa push (`git branch -r --contains HEAD` = 0 dòng). Worktree sạch; `HEAD == quaqua/sude-w3 == 463732f`.

## Xác nhận 2 finding round-1 đã xử lý
- **F1 [Đã xử lý]** — `crates/quaqua/tests/worker_unit.rs:10` import còn đúng `{build_launch_cmd, resolve_launch, Launch}`; `cargo check --locked --all-targets` → **0 warning / 0 error**.
- **F2 [Đã xử lý]** — Doc 2 hàm đã thu hẹp miền: `lead/spawn.rs:335` và `lead/spawn.rs:389` ("On standard LF markdown files with well-formed table rows"); hành vi dòng <4 field được pin bằng assert tại `worker_unit.rs:1971-1973` ("preserved as-is"). CRLF ngoài miền đã tuyên bố; file thật LF (tự đo: 0 ký tự `\r`, 0 dòng bảng <4 field).

## Kiểm chứng độc lập (round 2)
- **Extraction byte-identical**: khối `Launch..build_launch_cmd` + `claude_trust_dialog_up` (base `worker/spawn.rs:112-263,301-318`) == `crates/quaqua/src/launch.rs` nguyên byte, modulo duy nhất `fn`→`pub fn` tại `launch.rs:165` (tất yếu để dùng chéo module) — so bằng script, kết quả `True`. Không đổi args/model/effort per harness.
- **INDEX.md zero behavior change (kiểm độc lập)**: output transform mới == chương trình awk cũ (nguyên văn từ base) trên `~/quaqua/projects/INDEX.md` thật cho 3 slug (`zimage`→`zimage-lead`, `llmhub`→`cos`, `monotinhtute`→`monotinhtute-lead`) — **byte-identical từng ca**, kể cả round-trip `zimage→zimage-lead→cos` tái tạo đúng nguyên bản. Lock giữ nguyên cơ chế mkdir (200×50ms: `lead/spawn.rs:351-352`), nhả lock mọi nhánh kể cả lỗi (`:383`), ghi atomic tmp+rename (`:375,:378`).
- **Test tôi chạy lại** (build cold trong worktree này):
  - `env -u RUSTUP_TOOLCHAIN -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked` → **150 passed / 0 failed** (19+1+10+8+50+5+8+49; `worker_unit` **49/0**, gồm 3 test mới tại `worker_unit.rs:1935,1977,2020`).
  - `tests/test_cleanup_lead.sh` ×5 → **5 PASS / 0 FAIL**; `tests/test_spawn_lead.sh` ×5 → **5 PASS / 0 FAIL** — chạy với `QUAQUA_BIN=target/debug/quaqua` build từ HEAD; đối chứng chạy với QUAQUA_BIN sai → FAIL, xác nhận binary mới thực sự được dùng.
- **Greps/constraints**: `Command::new("awk")` trong `src/lead/` = **0** (toàn `src/` = 0); `crate::worker` trong `src/lead/` = 0 và `crate::lead` trong `src/worker/` = 0 (phụ thuộc `lead → worker` đã phá); `crate::launch` được dùng từ `lead/spawn.rs:41,58,239` + re-export `worker/spawn.rs:11`; diff `src/` không thêm hook test-only nào (0 `env::var`/`cfg(test)`/`QUAQUA_TEST`); `tests/` không còn stub awk (0 `BARRIER_DIR`/`REAL_AWK`).
- **Coverage giữ nguyên**: các test args/model/effort per harness + review-mode (`worker_unit.rs:600-1026`) vẫn nguyên và pass sau khi đổi path import.

## Findings
- **none** (0).

## Ghi nhận / Risks (không tính finding)
- Race test trong 2 file `.sh` giờ mang tính xác suất — đúng phương án (a) của Resolution, đã ghi chú trong test (`test_cleanup_lead.sh:143-144`, `test_spawn_lead.sh:253-255`); detector chính là test 8-thread `worker_unit.rs:2020` (có cả assert không sót tmp/lock).
- Lỗi ghi/rename INDEX giờ trả `Err` thay vì bỏ qua im lặng (`lead/spawn.rs:377-379`) — fail-loudly sau khi đã start agent; đã ghi nhận từ round 1, chấp nhận được.
- Re-export `worker/spawn.rs:11` giữ path cũ sống cho `pipeline/` và `taskforce/` — chủ đích (compat), không phải scope creep; ứng viên dọn ở W6.
- rustfmt không được enforce trong repo (không config/CI; các file không đụng tới cũng "dirty") → style không tính là finding.

## Blockers
none

Verdict: clean
