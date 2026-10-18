# Final review sáp nhập `quaqua-clean` — diff `a708fcd..c404f9b`

- Tip: `c404f9b` (quaqua/clean-integ, 79 files, +5079/−4900); base `a708fcd`.
- Read-only: không sửa file repo nào, không push; chỉ ghi report này.
- Đã đọc: diff tổng + toàn bộ module đổi nhiều (pipeline/{mod,state,phases/*}, worker/{prompt/*,spawn,cleanup,worktree,stop}, launch.rs, state.rs, util.rs, cap.rs, herdr.rs, grid/{apply,hook,reconcile,plan}, lead/{spawn,cleanup}, workspace.rs, tests/ + scripts/ đổi) và so từng hàm với bản a708fcd bằng `git show`, ưu tiên đường resume/kill/timeout của pipeline + watchdog.
- Đã chạy: `cargo check --locked -p quaqua` → PASS; `cargo test --locked -p quaqua` → worker_unit FAILED (56 passed, 2 failed); `cargo test --locked -p quaqua --test worker_unit grid_timeout` → 0 passed, 2 failed; tái hiện 2 lệnh của `tests/test_herdr_grid_rust.sh`; `grep -rn HERDR_STUB_SNAPSHOT crates/quaqua/src` → 0 hit; snapshot herdr read-only trước/sau khi chạy test (không pane nào bị move).

## Findings

### F1 — [CAO] `tests/test_herdr_grid_rust.sh` gãy: còn tiêu thụ backdoor `HERDR_STUB_SNAPSHOT` đã bị xoá
- `tests/test_herdr_grid_rust.sh:91,105` đặt `HERDR_STUB_SNAPSHOT=…/only-custom-tab.json` rồi kỳ vọng fixture này lái `--check`; nhưng `crates/quaqua/src` đã **0 consumer** của biến (wave 6, commit 1c10944) ⇒ binary gọi daemon herdr thật, fixture bị bỏ qua.
- Tái hiện đúng lệnh của script với binary tip: `HERDR_STUB_SNAPSHOT=tests/fixtures/herdr-grid/only-custom-tab.json QUAQUA_OWNER=cos target/debug/quaqua grid reconcile --check --tab quaqua-workers --only sude-isl,suhuynh-islr1 --dry-run --team-file team.toml` → in `[]`, rc=0; script (dòng 110-113) đòi fail kèm `pending ops` ⇒ suite FAIL. Ở env không có herdr thì lệnh dòng 91-99 fail sau 60s snapshot-timeout, cũng FAIL (đã ghi ở `runs/suhuynh-w6r1.md` F1).
- Hệ quả: `tests/run.sh:69` chạy suite này mặc định ⇒ **gate fast đỏ** ngoài `cargo test`.
- Lịch sử: đã báo ở w6r1 F1; `runs/sude-w6.fix1.md` §1.2 Challenge "ngoài scope", `runs/pipeline-w6.md` ghi "review dồn về final pass sáp nhập" — tới tip chưa có migration nào (`git log a708fcd..c404f9b -- tests/test_herdr_grid_rust.sh` rỗng).

### F2 — [CAO] 2 test `worker_unit.rs` còn tiêu thụ backdoor → `cargo test` đỏ + chạy repair thật trên daemon herdr
- `crates/quaqua/tests/worker_unit.rs:1667,1718` set `HERDR_STUB_SNAPSHOT` (spawn/cleanup grid-timeout); stub không còn được đọc ⇒ `run_grid_hook` → `run_grid_repair` (`worker/cleanup.rs:44` → `grid/reconcile.rs:606-618`, `CliHerdrDriver` = herdr thật) → `run_grid_check`.
- Tái hiện: `cargo test --locked -p quaqua` → `test_spawn_worker_grid_timeout_fails` panic tại worker_unit.rs:1692, `test_cleanup_worker_grid_timeout_fails` tại 1756 ("FAILED. 56 passed; 2 failed"); chạy riêng `cargo test --locked -p quaqua --test worker_unit grid_timeout` → "0 passed; 2 failed". Trong env hiện tại (herdr live, PATH có /opt/zerobrew/bin) grid hook hội tụ — worker cos duy nhất đang `working` → op deferred → spawn/cleanup trả Ok, còn test đòi Err.
- Hazard kèm theo: nếu có pane worker cos đang idle, chính 2 test này sẽ `pane move` pane thật (test set `GRID_TAB=quaqua-workers`, `QUAQUA_OWNER=cos`, remove `GRID_SKIP`). Lần chạy này không move (snapshot read-only trước/sau: pane `w9:p4A` vẫn ở `w9:t26`) — hazard nằm ở điều kiện idle.
- Lịch sử: w6r1 F2 + Challenge đã đề xuất `HERDR_BIN_PATH=/dev/null` cho 2 test (hoặc stub qua binary); chưa land.

### F3 — [TRUNG BÌNH] `tests/run.sh:31` slow_cover trỏ file đã xoá → slow suite prompt không còn được kích hoạt
- Dòng 31: `printf '%s' 'tests/test_prompt_worker.sh crates/quaqua/src/worker/prompt.rs'` — file `crates/quaqua/src/worker/prompt.rs` không tồn tại (wave 4a tách thành `worker/prompt/{mod,watchdog,observer,cost}.rs`).
- Kiểm chứng: `ls crates/quaqua/src/worker/prompt.rs` → No such file; mô phỏng touched = `crates/quaqua/src/worker/prompt/watchdog.rs` → `grep -qF "crates/quaqua/src/worker/prompt.rs"` NO HIT ⇒ suite bị SKIP dù watchdog vừa đổi. (Mục pipeline đã đổi đúng thành thư mục `crates/quaqua/src/pipeline/` — bất đối xứng cho thấy đây là sót.)

### F4 — [THẤP] Chuỗi hướng dẫn còn trỏ script đã xoá; `test_docs.sh` vừa loại `crates/` khỏi check nên không bắt được
- `crates/quaqua/src/gate/mod.rs:11` (BLOCK_MESSAGE — thông điệp chặn chính của `quaqua gate`; bản bash fallback đã được sửa nhưng bản Rust thì không): "…only via scripts/ (spawn-worker.sh, spawn-lead.sh, cleanup-worker.sh, cleanup-lead.sh)…".
- `crates/quaqua/src/pipeline/phases/mod.rs:252,257,292,310,346,361`: blocker in ra "run 'pipeline.sh --resume …'" — `command -v pipeline.sh` → not found. Tương tự `pipeline/mod.rs:22-66` (USAGE), `worker/spawn.rs:20-22`, `lead/spawn.rs:12,19,23`, `worker/prompt/mod.rs:22-27`, `worker/cleanup.rs:41`.
- `tests/test_docs.sh:62` (thêm trong đợt này) loại `^(crates/|docs/research/|…)` khỏi check "mọi ref `scripts/*.sh` phải resolve"; bỏ loại trừ thì check fail đúng 10 ref chết trong `crates/quaqua/src` (cap.rs, init.rs, lead/{cleanup,spawn}.rs, worker/{cleanup,prompt/mod,spawn}.rs) — exclusion đang che chính các ref này.

## Quan sát (không tính finding)
- O1: `is_lessons_heading` (`worker/prompt/watchdog.rs:46-79`) nay nhận cả heading đánh số/indent (`## 5. Lessons`), có test pin (worker_unit.rs:2274). Không phải regression so với bản trong repo (brief.md của nhánh không có dòng cấm), nhưng rules ở workspace `~/quaqua/rules/_all/brief.md:27` (uncommitted) nói "never numbered … fails exit 6" — CoS cần chốt trước khi commit rule đó.
- O2: `parse_prompt_args`/`PromptAction` (`worker/prompt/mod.rs:153-215`) là code chết: `main.rs:210-241` tự parse, không caller nào trong src; lại phân kỳ (nhận `--extend 0`, thứ tự arg khác). Chỉ test dùng (worker_unit.rs:2331).
- O3: Chuyển INDEX.md awk→Rust tương đương (trim/split/join; giả định LF như doc). Test concurrency barrier cũ bị thay bằng chạy song song xác suất (test_{spawn,cleanup}_lead.sh) + unit 8-thread (worker_unit.rs:2159) — yếu hơn về tính tất định, chấp nhận được.
- O4: Các refactor lớn đã đối chiếu từng hàm với a708fcd và giữ hành vi: cap (owner/slot), herdr bin (`HERDR_BIN_PATH`), apply/hook deferral propagation (nay một nguồn `filter_ops_for_check` + `apply_ops` — khớp logic cũ), prompt/watchdog (budget/grace/autoext/nudge/landing/cost), pipeline state format (byte-identical), worktree prep/push-block, launch cmd, cleanup file set (mở rộng đúng theo W2). `json.rs`→serde_json chỉ khác độ chặt (hết trailing-garbage) — không nơi nào dựa vào hành vi cũ.

## Risks
- Merge nguyên trạng trong env thật (herdr live) ⇒ gate fast đỏ: `cargo test` (2 test) + `test_herdr_grid_rust.sh`; slow suite `test_prompt_worker.sh:~336` (cùng stub, kỳ vọng "grid check failed" rc=1) phụ thuộc layout live — cũng sẽ đỏ khi worker cos không idle.
- `cargo test` trong env có daemon herdr có thể move pane thật (F2) — cần migrate test trước khi merge, hoặc chấp nhận có chủ đích kèm follow-up ngay.

Verdict: 4 findings

## Blockers
none

## Lessons
- Gỡ backdoor env khỏi src phải kèm migration mọi consumer (kể cả test ngoài scope wave) trong cùng đợt; "verify bằng PATH rút gọn" che mất gate đỏ thật — 2026-10-18, runs/suhuynh-finalreview.md
