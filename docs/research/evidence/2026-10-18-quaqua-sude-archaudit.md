# Báo cáo Audit Kiến trúc `crates/quaqua` & Kế hoạch Restructure theo Wave

- **Target commit**: `a708fcd20bda010e83288e67248ba74325cfa4d5` (origin/main)
- **Quy mô codebase**: 30 files Rust trong `crates/quaqua/src/`, tổng cộng 10,427 LOC (đo bằng `wc -l`).
- **Phụ thuộc ngoài** (`crates/quaqua/Cargo.toml`): 4 dependencies được ghim chặt (`clap = "=4.6.7"`, `serde = "=1.0.229"`, `toml = "=1.1.8"`, `serde_json = "=1.0.150"`).
- **Tính chất**: Read-only audit — không chỉnh sửa code trong repo.

---

## 1. Bản đồ Module & Đồ thị Phụ thuộc

`crates/quaqua/src/lib.rs:4-18` khai báo 15 module cấp cao. Phân tích AST và statement imports (`use crate::...`) cho thấy toàn bộ crate tạo thành một **Directed Acyclic Graph (DAG) tuyệt đối** ở cấp module: **0 cycles**.

### 1.1. Bảng chức năng & Quy mô từng Module

| Module | File count | LOC | Chức năng (1 dòng tóm tắt) | Module phụ thuộc trực tiếp |
| :--- | :---: | :---: | :--- | :--- |
| `config` | 1 | 728 | Phân tích cú pháp, resolve đè theo project và validate schema `team.toml`. | `workspace` |
| `workspace` | 1 | 59 | Xác định thư mục gốc workspace (`QUAQUA_HOME` → `$HOME/quaqua` → `.`). | *(none)* |
| `gate` | 3 | 639 | Parser shell tokenizer và kiểm tra policy bảo vệ lệnh herdr CLI (split/tab/workspace). | *(none)* |
| `init` | 1 | 269 | Khởi tạo cấu trúc thư mục workspace (`rules/`, `runs/`, hook herdr, git config). | *(none)* |
| `hash` | 1 | 143 | Cung cấp thuật toán tính băm MD5 chuẩn (RFC 1321) thuần Rust không external deps. | *(none)* |
| `git` | 1 | 238 | Wrapper thực thi Git CLI (worktree add/remove, branch, diff, rev-parse HEAD). | `hash` |
| `herdr` | 1 | 165 | Wrapper giao tiếp tiến trình herdr daemon (`HerdrOps` trait: agent list/get/read/start). | *(none)* |
| `json` | 1 | 276 | Bộ phân tích cú pháp JSON thủ công (được viết nhằm tránh dependency). | *(none)* |
| `cap` | 1 | 280 | Quản lý hạn ngạch tiến trình worker (tối đa 4 task worker / orchestrator). | `config`, `herdr`, `json` |
| `worker` | 5 | 2,027 | Quản lý vòng đời task worker: spawn (worktree), prompt (watchdog), cleanup, stop. | `cap`, `config`, `git`, `grid`, `hash`, `herdr`, `json`, `workspace` |
| `grid` | 6 | 2,424 | Thuật toán lập layout pane (1, 2, 3, 2x2, overflow), apply lệnh herdr và reconcile. | `config` |
| `lead` | 3 | 659 | Quản trị phiên Project Lead (`<slug>-lead`), workspace herdr riêng, lock INDEX.md. | `config`, `git`, `herdr`, `json`, `worker` |
| `taskforce` | 1 | 274 | Điều phối taskforce tạm thời chỉ đọc (`advisor` 1 lens, `dual-lens` 2 lens). | `cap`, `config`, `git`, `herdr`, `pipeline`, `worker` |
| `merge` | 1 | 332 | Điều phối hàng đợi merge branch Peer vào `quaqua/<slug>-integ`, verify test suite. | `config`, `git`, `lead` |
| `pipeline` | 1 | 1,406 | Cỗ máy trạng thái SDLC tự động (spawn → plan → build → review → fix loop). | `cap`, `config`, `git`, `grid`, `herdr`, `lead`, `worker` |
| **Tổng cộng** | **30** | **10,427** | *(Bao gồm cả main.rs 490 LOC, lib.rs 18 LOC)* | **Strict DAG (0 Cycles)** |

### 1.2. Chi tiết các Submodule

- **`grid/` (6 files, 2,424 LOC)**:
  - `grid/types.rs` (143 LOC): Định nghĩa các struct `Snapshot`, `Workspace`, `Tab`, `Pane`, `Tokens`, `Rect`, `Op`.
  - `grid/plan.rs` (659 LOC): Tính toán danh sách `Op` di chuyển/chia pane dựa theo số worker và trạng thái deferred.
  - `grid/apply.rs` (482 LOC): Thực thi `Op` thông qua `HerdrDriver` trait; chứa `CliHerdrDriver` và `FakeHerdrDriver`.
  - `grid/reconcile.rs` (649 LOC): Vòng lặp lấy snapshot hiện thời, so sánh với layout plan, lặp lại cho tới khi hội tụ hoặc timeout.
  - `grid/hook.rs` (480 LOC): Hook handler phản ứng khi herdr phát sinh sự kiện thay đổi pane; chứa `reconcile_step`.
  - `grid/mod.rs` (11 LOC): Re-export các struct và hàm public.
- **`worker/` (5 files, 2,027 LOC)**:
  - `worker/spawn.rs` (688 LOC): Parse CLI, tạo git worktree, khởi tạo env, chọn harness, split herdr pane, chạy `agent_start`.
  - `worker/prompt.rs` (1,039 LOC): Vòng lặp watchdog theo dõi worker, đọc pane fingerprint, đếm budget/stall/idle, bắt câu hỏi bị chặn.
  - `worker/cleanup.rs` (198 LOC): Thu dọn worktree, đóng pane herdr, xóa state file, lưu trữ report cũ.
  - `worker/stop.rs` (96 LOC): Gửi phím ngắt (`esc esc` cho opencode, `esc` cho omp) vào agent pane.
  - `worker/mod.rs` (6 LOC): Re-export module.
- **`lead/` (3 files, 659 LOC)**:
  - `lead/spawn.rs` (418 LOC): Khởi tạo Lead session, tạo workspace herdr, nhánh git integration, dispatch lead brief, lock `INDEX.md`.
  - `lead/cleanup.rs` (223 LOC): Kiểm tra liveness của Peer, thu hồi owner trong `INDEX.md`, đóng workspace herdr.
  - `lead/mod.rs` (18 LOC): CLI dispatch.
- **`gate/` (3 files, 639 LOC)**:
  - `gate/tokenizer.rs` (354 LOC): POSIX shell lexer phân tách token, redirection, subshell.
  - `gate/policy.rs` (223 LOC): Luật cấm các lệnh vi phạm quy hoạch herdr (`pane split`, `tab create`, `workspace create`).
  - `gate/mod.rs` (62 LOC): Đọc stdin JSON từ herdr hook và trả về kết quả allow/block.

### 1.3. Điểm rò rỉ ranh giới & Phụ thuộc lạc chỗ (Boundary Leaks)

1. **`merge` phụ thuộc `lead::spawn`** (`crates/quaqua/src/merge/mod.rs:188`):
   `handle_merge` gọi `crate::lead::spawn::find_quaqua_root()`. Hàm tìm kiếm thư mục root chứa `projects/INDEX.md` vốn là trách nhiệm của `workspace.rs`, nhưng lại bị đặt trong `lead/spawn.rs:34-79`, khiến module `merge` phải import xuyên qua `lead::spawn`.
2. **`lead::spawn` phụ thuộc `worker::spawn`** (`crates/quaqua/src/lead/spawn.rs:103, 284`):
   Mặc dù Lead không phải là task worker (không có worktree, không tính vào worker cap per `AGENTS.md`), `lead/spawn.rs` phải phụ thuộc vào `worker::spawn::{Launch, resolve_launch, build_launch_cmd}` vì logic khởi tạo harness bị nhét bên trong `worker/spawn.rs:112-250`.
3. **`pipeline` phụ thuộc `worker::cleanup`** (`crates/quaqua/src/pipeline/mod.rs:836, 883, 1014, 1206, 1275`):
   `pipeline/mod.rs` gọi `crate::worker::cleanup::find_engine_script()` để tìm các file shell script (`spawn-worker.sh`, `prompt-worker.sh`, `cleanup-worker.sh`). Hàm tiện ích tìm script lại nằm lạc trong `worker/cleanup.rs:38-83`.

---

## 2. Shared Code Paths Trùng lặp (Deduplication Audit)

Đo đạc chính xác qua grep call-sites và đối chiếu code cho thấy 8 vị trí trùng lặp logic nghiêm trọng:

### 2.1. Logic `resolve_launch` và `build_launch_cmd`
- **Vị trí**: `crates/quaqua/src/worker/spawn.rs:112-250` (139 LOC).
- **Call-sites**:
  - `worker/spawn.rs:359` (khởi tạo worker chính).
  - `worker/spawn.rs:516` (thực thi launch command).
  - `worker/spawn.rs:640` (khởi tạo fallback launch khi primary fail).
  - `lead/spawn.rs:103` (trong `resolve_lead_launch`).
  - `lead/spawn.rs:284` (xây dựng lệnh chạy cho Lead).
- **Vấn đề**: Logic map harness (opencode format `#effort`, claude format prefix `claude-`, omp flags) áp dụng chung cho mọi agent session nhưng lại nằm ở `worker/spawn.rs`.
- **Đề xuất**: Tách thành `crates/quaqua/src/launch.rs` (ước tính ~150 LOC). `worker` và `lead` cùng dùng chung.

### 2.2. Fake-Clock Resolver `now_sec()`
- **Vị trí**: Copy-paste nguyên văn tại 3 files:
  1. `crates/quaqua/src/pipeline/mod.rs:99-111` (13 LOC)
  2. `crates/quaqua/src/worker/prompt.rs:232-244` (13 LOC)
  3. `crates/quaqua/src/grid/reconcile.rs:36-47` (12 LOC)
- **Vấn đề**: Đọc biến môi trường `PROMPT_WORKER_CLOCK_FILE` để mock thời gian trong test (per rule `tieude-capfix2.md`), nếu không có thì đọc `SystemTime::now()`. Ba bản sao giống hệt từng byte code.
- **Đề xuất**: Gom về `src/workspace.rs` hoặc `src/time.rs` (~15 LOC).

### 2.3. Kiểm tra tiến trình sống `is_pid_alive`
- **Vị trí**: Duplicate tại 3 nơi:
  1. `crates/quaqua/src/cap.rs:27-35` (hàm `is_pid_alive(pid_str: &str)` chạy `kill -0 <pid>`)
  2. `crates/quaqua/src/grid/hook.rs:45-53` (hàm `is_pid_alive(pid_str: &str)` chạy `kill -0 <pid>`)
  3. `crates/quaqua/src/worker/prompt.rs:387` (inline command: `Command::new("kill").arg("-0").arg(p_pid)`)
- **Đề xuất**: Gom về `src/workspace.rs` hoặc module tiện ích tiến trình (~10 LOC).

### 2.4. Khóa Spinlock thư mục và Update Bảng `projects/INDEX.md`
- **Vị trí**:
  1. `crates/quaqua/src/lead/spawn.rs:366-410` (45 LOC)
  2. `crates/quaqua/src/lead/cleanup.rs:178-220` (43 LOC)
- **Vấn đề**: Cả hai hàm đều dùng chung cơ chế spinlock tạo thư mục `projects/INDEX.md.lock.d` (lặp tối đa 200 lần, sleep 50ms), sau đó spawn tiến trình ngoài `Command::new("awk")` với tham số `-F|` để cập nhật cột `owner` của slug dự án, ghi ra file `INDEX.md.tmp.<pid>` và rename đè lên.
- **Đề xuất**: Viết hàm thuần Rust `update_project_index_owner(root: &Path, slug: &str, new_owner: &str) -> Result<(), ...>` (ước tính ~50 LOC), loại bỏ hoàn toàn việc gọi subprocess `awk`.

### 2.5. Hai hệ thống Trait trừu tượng Herdr tách biệt
- **Vị trí**:
  1. `crates/quaqua/src/herdr.rs:5-34` (`pub trait HerdrOps` với 15 methods, implement bởi `struct Herdr`).
  2. `crates/quaqua/src/grid/apply.rs:12-16` (`pub trait HerdrDriver` với 3 methods `snapshot`, `move_pane`, `focus_tab`, implement bởi `CliHerdrDriver` và `FakeHerdrDriver`).
  3. `crates/quaqua/src/init.rs:189, 246`: Bỏ qua cả hai trait trên, gọi thẳng `Command::new("herdr")`.
- **Vấn đề**: Hai trait song song phục vụ cùng một binary `herdr`, dẫn đến việc mock trong test bị phân mảnh (`HerdrOps` mock bằng file stub / wrapper trong test, còn `grid` mock bằng `FakeHerdrDriver`).
- **Đề xuất**: Đưa `HerdrDriver` vào trong `herdr.rs` hoặc hợp nhất thành một trait thống nhất.

### 2.6. Trùng lặp Parser JSON: `json.rs` tự chế vs `serde_json`
- **Vị trí**: `crates/quaqua/src/json.rs:1-276` (276 LOC).
- **Vấn đề**: File mang doc-comment:
  `//! Lightweight JSON parser in pure Rust without external dependencies.`
  `//! Satisfies --locked constraint by not requiring serde_json.`
  Tuy nhiên, `serde_json = "=1.0.150"` **đã nằm trong `Cargo.toml`** và được dùng rộng rãi trong toàn bộ `gate/mod.rs`, `grid/*`, `init.rs`, `main.rs`, và thậm chí ngay tại `worker/prompt.rs:157`!
  Trong khi đó, `cap.rs:70`, `lead/cleanup.rs:51`, `lead/spawn.rs:168`, `worker/cleanup.rs:121`, `worker/prompt.rs:454`, `worker/spawn.rs:529`, `worker/stop.rs:33` vẫn đang dùng `crate::json::parse_json`.
- **Đề xuất**: Xóa bỏ toàn bộ `crates/quaqua/src/json.rs` (-276 LOC dead weight), chuyển toàn bộ các caller sang dùng `serde_json::Value`.

### 2.7. Xử lý State File phân tán và Parsing Ad-hoc
- **Vị trí**:
  - `cap.rs:114-156`: Quét thư mục `runs/`, tìm file `pipeline-*.state` và `taskforce-*.state`, duyệt từng dòng, kiểm tra prefix `lead=`, `pipeline_pid=`, `builder=`, `task=`, `finished=0`.
  - `pipeline/mod.rs:214-260`: `PipelineState::load` tự parse từng dòng theo định dạng `key=value`, strip dấu ngoặc kép.
  - `taskforce/mod.rs:188-195`: Ghi format thô `task=...\nkind=...\nowner=...\nfinished=0\n`.
  - `worker/spawn.rs:496-497`, `worker/cleanup.rs:157, 170`, `worker/prompt.rs:701`: Đọc/ghi phân tán các file `.owner`, `.grid`, `.budget`, `.grace`, `.autoext` bằng raw string IO.
- **Vấn đề**: Không có schema chung cho metadata của worker/pipeline. Thậm chí `cleanup.rs:190` chỉ xóa `.autoext` nhưng bỏ quên `.budget`, `.grace`, `.cost80`, `.cost100`.
- **Đề xuất**: Tạo struct `WorkerMetadata` và `TaskState` tập trung hóa tại `src/state.rs` (ước tính ~120 LOC).

### 2.8. Hiện tượng "Shell Trampolining" (`find_engine_script`)
- **Vị trí**: `crates/quaqua/src/worker/cleanup.rs:38-83` và `crates/quaqua/src/pipeline/mod.rs:836, 883, 1014, 1206, 1275`.
- **Vấn đề**: `pipeline/mod.rs` kiểm tra xem có script `spawn-worker.sh`, `prompt-worker.sh`, `cleanup-worker.sh` không. Nếu có, nó spawn bash subprocess. Nhưng nội dung của các bash script đó lại là:
  `exec "${QUAQUA_BIN:-quaqua}" worker spawn "$@"`!
  Nghĩa là: Rust binary `quaqua` spawn bash script → bash script gọi lại binary `quaqua` bằng subprocess! Nếu không tìm thấy script, nó mới chạy in-process Rust.
- **Đề xuất**: Xóa cơ chế fallback trampolining; `pipeline` luôn gọi thẳng hàm Rust in-process (`worker::spawn::spawn_worker`, `worker::prompt::prompt_worker`, `worker::cleanup::cleanup_worker`).

---

## 3. Điểm Phình to (Bloat Points)

### 3.1. Danh sách 30 Files theo LOC (Đo đạc thực tế)

| STT | File Path | LOC | Nhận xét kiến trúc |
| :---: | :--- | :---: | :--- |
| 1 | `crates/quaqua/src/pipeline/mod.rs` | **1,406** | File lớn nhất repo; chứa cỗ máy trạng thái monolithic 982 dòng. |
| 2 | `crates/quaqua/src/worker/prompt.rs` | **1,039** | Chứa vòng lặp watchdog 457 dòng gộp mọi logic budget/cost/stall/landing. |
| 3 | `crates/quaqua/src/config.rs` | **728** | Trần phẳng, chủ yếu là struct TOML và hàm `validate` 176 dòng. |
| 4 | `crates/quaqua/src/worker/spawn.rs` | **688** | Gộp worktree setup, launch command builder và fallback loop. |
| 5 | `crates/quaqua/src/grid/plan.rs` | **659** | Thuật toán chia pane và xếp layout đa tab. |
| 6 | `crates/quaqua/src/grid/reconcile.rs` | **649** | Vòng lặp reconcile layout với herdr. |
| 7 | `crates/quaqua/src/main.rs` | **490** | CLI entry point, match lệnh và fallback dispatch. |
| 8 | `crates/quaqua/src/grid/apply.rs` | **482** | Thực thi op layout; chứa `FakeHerdrDriver` test fixture. |
| 9 | `crates/quaqua/src/grid/hook.rs` | **480** | Hook listener; chứa bản sao reconcile thu nhỏ 205 dòng. |
| 10 | `crates/quaqua/src/lead/spawn.rs` | **418** | Khởi tạo Lead; chứa `find_quaqua_root` và spinlock awk. |
| 11 | `crates/quaqua/src/gate/tokenizer.rs` | **354** | Parser POSIX shell lexer. |
| 12 | `crates/quaqua/src/merge/mod.rs` | **332** | Vòng lặp merge queue và check command. |
| 13 | `crates/quaqua/src/cap.rs` | **280** | Đếm slot worker và parse state files. |
| 14 | `crates/quaqua/src/json.rs` | **276** | Parser JSON tự chế (dead weight). |
| 15 | `crates/quaqua/src/taskforce/mod.rs` | **274** | Điều phối advisor / dual-lens. |
| 16 | `crates/quaqua/src/init.rs` | **269** | Khởi tạo workspace và symlink hook. |
| 17 | `crates/quaqua/src/git.rs` | **238** | Git CLI wrapper. |
| 18 | `crates/quaqua/src/gate/policy.rs` | **223** | Luật chặn lệnh herdr nguy hiểm. |
| 19 | `crates/quaqua/src/lead/cleanup.rs` | **223** | Thu dọn Lead session; chứa spinlock awk. |
| 20 | `crates/quaqua/src/worker/cleanup.rs` | **198** | Thu dọn worker; chứa `find_engine_script`. |
| 21 | `crates/quaqua/src/herdr.rs` | **165** | Wrapper lệnh herdr daemon. |
| 22 | `crates/quaqua/src/grid/types.rs` | **143** | Dữ liệu hình học và snapshot JSON. |
| 23 | `crates/quaqua/src/hash.rs` | **143** | MD5 thuần Rust. |
| 24 | `crates/quaqua/src/worker/stop.rs` | **96** | Phím ngắt agent harness. |
| 25 | `crates/quaqua/src/gate/mod.rs` | **62** | Entry point gate hook. |
| 26 | `crates/quaqua/src/workspace.rs` | **59** | Root resolver chuẩn. |
| 27 | `crates/quaqua/src/lead/mod.rs` | **18** | Re-export lead. |
| 28 | `crates/quaqua/src/lib.rs` | **18** | Crate root. |
| 29 | `crates/quaqua/src/grid/mod.rs` | **11** | Re-export grid. |
| 30 | `crates/quaqua/src/worker/mod.rs` | **6** | Re-export worker. |

### 3.2. Bảng 16 Hàm >100 Dòng

| STT | File:Line | Tên hàm | Độ dài | Trách nhiệm hiện tại & Đề xuất tách (Decomposition) |
| :---: | :--- | :--- | :---: | :--- |
| 1 | `pipeline/mod.rs:425-1406` | `handle_pipeline` | **982** LOC | **God-function**: Ôm toàn bộ cỗ máy trạng thái, parse CLI args, closure inline, match 404 dòng. Đề xuất: tách thành struct `PipelineRunner` và các submodule `phases/{spawn, plan, build, review, fix}.rs`. |
| 2 | `grid/reconcile.rs:151-613` | `run_reconcile` | **463** LOC | Reconcile orchestrator: xử lý snapshot, retry, diff layout, repair execution. Đề xuất: tách riêng `DiffEngine`, `ConvergenceChecker`, `LayoutApplier`. |
| 3 | `worker/prompt.rs:583-1039` | `prompt_worker` | **457** LOC | Watchdog loop: gộp observe herdr, check cost SQLite, budget extension, prompt landing, nudge, question detection. Đề xuất: tách `WatchdogLoop`, `CostMonitor`, `PromptNudger`. |
| 4 | `worker/spawn.rs:333-688` | `spawn_worker` | **356** LOC | Khởi tạo worker: resolve config, worktree add, split pane herdr, retry fallback loop, token reporting. Đề xuất: tách `WorktreeSetup` và `HarnessLauncher`. |
| 5 | `lead/spawn.rs:106-418` | `spawn_lead` | **313** LOC | Khởi tạo Lead: slug validation, herdr workspace, git integ branch, rules concating, index locking awk. Đề xuất: tách `LeadWorkspace` và `IndexLock`. |
| 6 | `grid/plan.rs:367-659` | `plan_with_deferred` | **293** LOC | Lập kế hoạch layout: group sorting, tính ô rect, tab assignment, overflow pagination. Đề xuất: tách `TabAllocator` và `RectGeometry`. |
| 7 | `grid/apply.rs:259-482` | `apply_ops` | **224** LOC | Thực thi lệnh dịch chuyển pane: matching target, split ratio, snapshot validation. Đề xuất: chia nhỏ theo từng loại `Target` enum. |
| 8 | `cap.rs:62-280` | `count_owner_slots` | **219** LOC | Đếm slot worker: parse agent list json, đọc file `.owner`, quét `runs/` cho pipeline và taskforce state. Đề xuất: tách hàm quét live agent và hàm parse state file. |
| 9 | `grid/hook.rs:215-419` | `reconcile_step` | **205** LOC | Reconcile thu nhỏ trong hook: detect closed pane, in-flight poll, apply ops. Đề xuất: dùng chung logic với `grid/reconcile.rs`. |
| 10 | `lead/cleanup.rs:34-223` | `cleanup_lead` | **190** LOC | Dọn dẹp Lead: kiểm tra Peer liveness, index locking awk, đóng workspace. Đề xuất: tách phần liveness check và index update. |
| 11 | `config.rs:328-503` | `validate` | **176** LOC | Validate toàn bộ `team.toml` (timers, kinds, models, efforts, projects). Đề xuất: tách thành `validate_timers`, `validate_kinds`, `validate_projects`. |
| 12 | `merge/mod.rs:183-332` | `handle_merge` | **150** LOC | Merge queue: vòng lặp merge tuần tự, chạy check command, reset --hard khi fail. Đề xuất: tách `MergeStep` thành hàm độc lập. |
| 13 | `main.rs:366-490` | `main` | **125** LOC | Clap dispatching và mapping error codes. Đề xuất: tách các nhánh subcommand vào các helper function. |
| 14 | `gate/tokenizer.rs:230-354` | `tok` | **125** LOC | Phân loại ký tự shell, handle quote, heredoc, whitespace. Đề xuất: tách state machine handlers. |
| 15 | `hash.rs:4-123` | `md5_hex` | **120** LOC | 4 round biến đổi MD5 với macro unroll. Không cần tách vì tuân thủ đúng RFC 1321. |
| 16 | `taskforce/mod.rs:166-274` | `run_dual_lens` | **109** LOC | Điều phối 2 worker song song, tạo state file, prompt brief. Đề xuất: tách thành 2 phase launch và wait. |

### 3.3. Các Match Statements Khổng Lồ

1. `crates/quaqua/src/pipeline/mod.rs:1001-1404` (**404 lines**): `match state.phase.as_str()` nằm bên trong `loop { ... }`. Đây là toàn bộ state machine chuyển pha (spawn → plan → plan_wait → plan_review → build → build_wait → review → review_wait → fix → fix_wait). Cần tách mỗi arm thành một phase handler method.
2. `crates/quaqua/src/main.rs:371-488` (**118 lines**): `match cli.command` điều phối tất cả các subcommand (`Init`, `Config`, `Gate`, `Grid`, `Worker`, `Lead`, `Taskforce`, `Merge`, `Pipeline`).
3. `crates/quaqua/src/main.rs:191-278` (**88 lines**): `match args[0].as_str()` cơ chế fallback dispatch khi gọi không có subcommand rõ ràng.
4. `crates/quaqua/src/grid/apply.rs:354-437` (**84 lines**): `match &op.target` phân loại hành động chuyển pane (`NewTab` vs `Split`).
5. `crates/quaqua/src/gate/tokenizer.rs:274-350` (**77 lines**): `match c` trong bộ tách từ shell.
6. `crates/quaqua/src/config.rs:510-571` (**62 lines**): `match parts.as_slice()` trong lệnh `quaqua config get <path>`.

---

## 4. Conventions (Naming, Error Handling, Doc Comments)

### 4.1. Sự Phân Cực trong Error Handling
Codebase hiện tại bị chia cắt sâu sắc giữa hai trường phái xử lý lỗi:
- **Trường phái 1 (`Result<T, (i32, String)>`)**: Dùng ở các tầng điều phối và CLI (`worker/*`, `lead/*`, `pipeline`, `taskforce`, `merge`). Mã lỗi `i32` mang ngữ nghĩa watchdog (0: success, 1: usage, 2: cap, 3: stall, 4: budget, 5: blocked, 6: malformed).
- **Trường phái 2 (`Result<T, String>`)**: Dùng ở tầng thư viện thuật toán (`grid/*`, `config.rs`, `init.rs`, `json.rs`).
- **Hậu quả**: Tại `main.rs:174-184` và `pipeline/mod.rs:861, 898`, code phải liên tục convert thủ công bằng `.map_err(|e| (1, format!("... {e}")))` hoặc match vụng về.
- **Chuẩn hóa đề xuất**:
  Tạo kiểu lỗi tập trung:
  ```rust
  #[derive(Debug)]
  pub struct ExitError {
      pub code: i32,
      pub message: String,
  }
  pub type ExitResult<T> = Result<T, ExitError>;
  ```
  Implement `From<String>` (gán `code = 1`), `From<std::io::Error>`, và `From<serde_json::Error>` để tận dụng toán tử `?`.

### 4.2. Bất nhất trong Quy ước Đặt tên (Naming)
- Handler CLI gọi lộn xộn giữa:
  - `handle_*`: `handle_pipeline` (`pipeline/mod.rs:425`), `handle_merge` (`merge/mod.rs:183`), `handle_taskforce` (`taskforce/mod.rs:76`), `handle_lead` (`lead/mod.rs:9`).
  - `run_*`: `run_reconcile` (`grid/reconcile.rs:151`), `run_taskforce` (`taskforce/mod.rs:97`), `run_grid_check` (`grid/reconcile.rs:627`).
  - Động từ trực tiếp: `spawn_worker`, `cleanup_worker`, `prompt_worker`, `stop_worker`, `spawn_lead`, `cleanup_lead`.
- **Chuẩn hóa đề xuất**:
  - Tầng CLI adapter nhận raw slice args: quy ước thống nhất là `handle_<subcommand>(args, ...)` hoặc đặt trong submodule `cli`.
  - Tầng domain logic: dùng động từ hành động trực tiếp (`spawn`, `cleanup`, `prompt`, `reconcile`).

### 4.3. Bất cập trong Doc Comments & USAGE String Constants
- Tồn tại hơn 10 hằng số `const USAGE: &str = "# Usage: ..."` mang nguyên vẹn dấu comment `#` và tên file `.sh` từ thời bash script:
  - `worker/spawn.rs:14`: `# Usage: spawn-worker.sh ...`
  - `worker/prompt.rs:13`: `# Usage: prompt-worker.sh ...`
  - `lead/spawn.rs:12`: `# Usage: spawn-lead.sh ...`
  - `lead/cleanup.rs:12`: `# Usage: cleanup-lead.sh ...`
  - `merge/mod.rs:11`: `# Usage: quaqua merge ...`
  - `pipeline/mod.rs:14`: `# Usage: pipeline.sh ...`
- Khi người dùng chạy lệnh sai cú pháp hoặc `--help`, terminal in ra hướng dẫn chạy shell script thay vì hướng dẫn chạy lệnh `quaqua` tương ứng.
- **Chuẩn hóa đề xuất**: Loại bỏ các chuỗi bash USAGE, chuyển sang dùng thuộc tính `#[doc]` và `#[command(about, long_about)]` chuẩn của `clap`.

---

## 5. Dead Code & Test-Only Code trong `src/`

Đo đạc đối chiếu giữa các khai báo `pub` trong `src/` với toàn bộ call-sites trong `src/` và `tests/`:

### 5.1. Dead Code Tuyệt Đối (Không ai sử dụng)
1. **`strip_tab_suffix`** (`crates/quaqua/src/grid/plan.rs:151-153`):
   `pub fn strip_tab_suffix(label: &str) -> &str { label.strip_suffix("-2").unwrap_or(label) }`
   Hoàn toàn không có bất kỳ call-site nào trong cả `src/` lẫn `tests/`!
2. **`struct Role`** (`crates/quaqua/src/config.rs:139-148`):
   Được deserialize vào `Config.roles`, nhưng không có bất kỳ logic nào trong pipeline, gate, cap, hay worker kiểm tra quyền hạn `can_spawn`, `can_prompt`, `reports_to`. Chỉ được đọc tại `config get roles.<name>` và vòng lặp validate rỗng.

### 5.2. Test-Only Code Bị Compile vào Production Binary
1. **`FakeHerdrDriver`** (`crates/quaqua/src/grid/apply.rs:166-250` - **85 LOC**):
   Một mock driver hoàn chỉnh mô phỏng herdr trong bộ nhớ (với methods `new`, `snapshot`, `move_pane`, `focus_tab`). Struct này chỉ được dùng trong `tests/grid.rs`, nhưng lại nằm trực tiếp trong `src/grid/apply.rs` và được re-export tại `grid/mod.rs` và `lib.rs`.
2. **Backdoor `HERDR_STUB_SNAPSHOT`** (`crates/quaqua/src/grid/apply.rs:78, 100, 151`):
   Trong `CliHerdrDriver`, mã nguồn kiểm tra biến môi trường `HERDR_STUB_SNAPSHOT` để đọc file JSON thay vì gọi tiến trình herdr thật. Đây là code test nằm trong driver production.
3. **Hardcoded Table `harness_of`** (`crates/quaqua/src/pipeline/mod.rs:60-67`):
   ```rust
   pub fn harness_of(kind: &str) -> Option<&'static str> {
       match kind {
           "tieude" | "detu" => Some("opencode"),
           "sude" => Some("omp"),
           "ni" | "suhuynh" | "truonglao" => Some("claude"),
           _ => None,
       }
   }
   ```
   Hàm này hardcode bảng mapping harness thay vì tra cứu từ `team.toml` (`cfg.kinds`), vi phạm nguyên tắc "Cấu hình đổi không cần sửa code". Chỉ còn một test trong `tests/worker_unit.rs:877` gọi trực tiếp vào nó.

---

## 6. Kế hoạch Restructure theo Wave

Kế hoạch được sắp xếp theo thứ tự ưu tiên: **Từ rẻ nhất, rủi ro thấp nhất, giá trị tức thì nhất đến sâu hơn**. Mỗi wave chỉ có **≤1 file-area owner**, có rủi ro và phương án verify độc lập.

```
+----------------------------------------------------------------------------------+
| Wave 1: Dọn Dead Code & Gom Tiện ích Chung (util.rs, workspace.rs)               |
| Impact: Xóa dead code, gom now_sec / is_pid_alive, chuyển find_quaqua_root       |
+----------------------------------------+-----------------------------------------+
                                         |
+----------------------------------------v-----------------------------------------+
| Wave 2: Xóa Parser json.rs & Chuẩn hóa State Files IO (state.rs)                 |
| Impact: Xóa 276 LOC json.rs, chuyển sang serde_json, gom đọc ghi .owner/.state   |
+----------------------------------------+-----------------------------------------+
                                         |
+----------------------------------------v-----------------------------------------+
| Wave 3: Tách Độc lập Agent Launch Engine (launch.rs)                             |
| Impact: Phá vỡ phụ thuộc lead -> worker, loại bỏ shell-out awk khi lock index    |
+----------------------------------------+-----------------------------------------+
                                         |
+----------------------------------------v-----------------------------------------+
| Wave 4: Tách Module Watchdog & Worker (worker/prompt/)                           |
| Impact: Giảm prompt.rs từ 1039 LOC thành 4 module mạch lạc, rõ ràng state        |
+----------------------------------------+-----------------------------------------+
                                         |
+----------------------------------------v-----------------------------------------+
| Wave 5: Phân rã Cỗ máy Trạng thái Pipeline (pipeline/phases/)                    |
| Impact: Xóa god-function 982 dòng, xóa trampoline bash scripts                   |
+----------------------------------------+-----------------------------------------+
                                         |
+----------------------------------------v-----------------------------------------+
| Wave 6: Hợp nhất Trait Herdr & Dọn dẹp grid/*                                    |
| Impact: Đưa FakeHerdrDriver ra khỏi production, đồng nhất Reconcile predicate    |
+----------------------------------------------------------------------------------+
```

---

### Wave 1: Dọn Dead Code & Gom Tiện ích Chung
- **Mục tiêu**: Loại bỏ code thừa, gom các hàm tiện ích trùng lặp tuyệt đối, xóa phụ thuộc lạc chỗ giữa `merge` và `lead`.
- **Phạm vi (File-area Owner)**: `crates/quaqua/src/workspace.rs`, `crates/quaqua/src/grid/plan.rs`, `crates/quaqua/src/util.rs` (mới).
- **Các bước thực hiện**:
  1. Xóa `strip_tab_suffix` trong `grid/plan.rs:151`.
  2. Tạo `crates/quaqua/src/util.rs`:
     - Chuyển `now_sec()` về `util.rs` (xóa 3 bản copy tại `pipeline/mod.rs:99`, `worker/prompt.rs:232`, `grid/reconcile.rs:36`).
     - Chuyển `is_pid_alive()` về `util.rs` (xóa 3 bản copy tại `cap.rs:27`, `grid/hook.rs:45`, `worker/prompt.rs:387`).
  3. Di chuyển `find_quaqua_root()` từ `lead/spawn.rs:34-79` sang `workspace.rs`. Cập nhật call-site tại `merge/mod.rs:188` (chấm dứt việc `merge` import `lead::spawn`).
- **Rủi ro**: Cực kỳ thấp (0 risk về thay đổi hành vi logic).
- **Cách verify**:
  - `cargo check --locked -p quaqua`
  - `cargo test --locked -p quaqua --test config --test init --test cli`

---

### Wave 2: Xóa Bỏ Parser `json.rs` & Chuẩn hóa State Files IO
- **Mục tiêu**: Loại bỏ 276 LOC parser tự chế thừa thãi; chuẩn hóa đọc ghi metadata của worker và pipeline.
- **Phạm vi (File-area Owner)**: `crates/quaqua/src/json.rs` (xóa), `crates/quaqua/src/state.rs` (mới), và các caller (`cap.rs`, `worker/cleanup.rs`, `worker/stop.rs`, `lead/cleanup.rs`).
- **Các bước thực hiện**:
  1. Chuyển đổi 7 caller của `crate::json::parse_json` sang `serde_json::from_str::<serde_json::Value>` hoặc struct có kiểu.
  2. Xóa file `crates/quaqua/src/json.rs` và bỏ khai báo `pub mod json;` trong `lib.rs`.
  3. Tạo `crates/quaqua/src/state.rs`: định nghĩa struct `WorkerMetadata` (chứa owner, grid, budget, grace, autoext) và `ExecutionState` (chứa pipeline/taskforce state), cung cấp phương thức `save()` / `load()` an toàn, atomic.
  4. Sửa `worker/cleanup.rs` để dọn sạch sẽ tất cả file metadata (`.budget`, `.grace`, `.cost80`, `.cost100`) thay vì sót như hiện tại.
- **Rủi ro**: Thấp (đảm bảo parse JSON của các trường `agent get` và `agent list` từ herdr giữ nguyên kiểu dữ liệu).
- **Cách verify**:
  - `cargo test --locked -p quaqua --test worker_unit --test pipeline_prompt`

---

### Wave 3: Tách Độc lập Agent Launch Engine & Pure Rust Index Locking
- **Mục tiêu**: Phá vỡ phụ thuộc ngược `lead/spawn.rs -> worker/spawn.rs`; loại bỏ việc spawn subprocess `awk`.
- **Phạm vi (File-area Owner)**: `crates/quaqua/src/launch.rs` (mới), `crates/quaqua/src/worker/spawn.rs`, `crates/quaqua/src/lead/spawn.rs`, `crates/quaqua/src/lead/cleanup.rs`.
- **Các bước thực hiện**:
  1. Tạo `crates/quaqua/src/launch.rs`: Chuyển `Launch`, `resolve_launch`, `build_launch_cmd`, `claude_trust_dialog_up` từ `worker/spawn.rs:112-250` sang `launch.rs`.
  2. Cập nhật `worker/spawn.rs` và `lead/spawn.rs` import từ `crate::launch::*`.
  3. Viết hàm thuần Rust `update_project_index_owner(root, slug, new_owner)` đặt tại `lead/mod.rs` hoặc `workspace.rs`: parse markdown table row, thay thế cột owner, ghi atomic qua file tạm.
  4. Thay thế 2 khối spinlock awk (45 dòng tại `lead/spawn.rs:366` và 43 dòng tại `lead/cleanup.rs:178`) bằng hàm thuần Rust trên.
- **Rủi ro**: Thấp. Hàm cập nhật markdown table cần được test kỹ các trường hợp khoảng trắng phân cách cột `|`.
- **Cách verify**:
  - `cargo test --locked -p quaqua --test worker_unit`
  - Viết 1 unit test kiểm tra hàm `update_project_index_owner` trên fixture `INDEX.md`.

---

### Wave 4: Tách Module Watchdog & Worker Prompt
- **Mục tiêu**: Giảm độ phình của `worker/prompt.rs` (từ 1,039 LOC xuống các submodule nhỏ <300 LOC), phân định rõ ràng giữa việc quan sát (observe) và phán quyết (watchdog).
- **Phạm vi (File-area Owner)**: Thư mục `crates/quaqua/src/worker/prompt/`.
- **Các bước thực hiện**:
  1. Chuyển `worker/prompt.rs` thành thư mục `crates/quaqua/src/worker/prompt/`:
     - `observer.rs` (~250 LOC): Hàm `observe()`, chụp text pane herdr, tính hash diff git, kiểm tra trạng thái liveness.
     - `watchdog.rs` (~350 LOC): Vòng lặp `while !report_ready`, kiểm tra stall, idle, budget deadline, grace, autoext.
     - `cost.rs` (~120 LOC): Đọc SQLite cost qua session id, trigger cảnh báo 80%/100%.
     - `mod.rs` (~200 LOC): Parse CLI args, hàm entrypoint `prompt_worker`.
  2. Tách `worker/spawn.rs` (688 LOC): tách logic chuẩn bị worktree git ra `worker/worktree.rs` (~150 LOC).
- **Rủi ro**: Trung bình. Watchdog timing và các mã thoát (watchdog exit codes 2, 3, 4, 5, 6) phải được bảo toàn chính xác từng giây.
- **Cách verify**:
  - `cargo test --locked -p quaqua --test worker_unit --test pipeline_prompt`

---

### Wave 5: Phân rã Cỗ máy Trạng thái Pipeline
- **Mục tiêu**: Xóa bỏ file dài nhất repo (`pipeline/mod.rs` 1,406 LOC, `handle_pipeline` 982 LOC); loại bỏ hoàn toàn cơ chế trampoline qua bash script.
- **Phạm vi (File-area Owner)**: Thư mục `crates/quaqua/src/pipeline/`.
- **Các bước thực hiện**:
  1. Tách `pipeline/mod.rs` thành cấu trúc:
     - `pipeline/state.rs` (~250 LOC): `PipelineState`, load/save, check_clock.
     - `pipeline/phases/mod.rs`: Trait hoặc dispatcher cho các phase.
     - `pipeline/phases/spawn.rs`, `plan.rs`, `build.rs`, `review.rs`, `fix.rs` (mỗi file ~150-200 LOC).
  2. Xóa bỏ `find_engine_script` và các khối `if let Some(psh) = prompt_sh { Command::new("bash")... }`. Pipeline gọi trực tiếp in-process:
     - `crate::worker::spawn::spawn_worker`
     - `crate::worker::prompt::prompt_worker`
     - `crate::worker::cleanup::cleanup_worker`
  3. Xóa bảng mapping cứng `harness_of` trong `pipeline/mod.rs:60-67`, chuyển 100% việc xác định harness qua `cfg.kinds.get(kind)`.
- **Rủi ro**: Trung bình - cao. Cần đảm bảo state transition giữa các phase khi bị ngắt quãng (`--resume`) đọc đúng commit SHA và branch tip.
- **Cách verify**:
  - `cargo test --locked -p quaqua --test pipeline_prompt`

---

### Wave 6: Hợp nhất Trait Herdr & Dọn dẹp `grid/*`
- **Mục tiêu**: Đưa test mock (`FakeHerdrDriver`) và backdoor test (`HERDR_STUB_SNAPSHOT`) ra khỏi production binary; đồng nhất logic kiểm tra hội tụ giữa hook và reconcile.
- **Phạm vi (File-area Owner)**: `crates/quaqua/src/herdr.rs`, `crates/quaqua/src/grid/`.
- **Các bước thực hiện**:
  1. Hợp nhất `HerdrDriver` vào hệ thống `HerdrOps` trong `herdr.rs`.
  2. Đặt `FakeHerdrDriver` dưới cờ `#[cfg(test)]` hoặc chuyển sang thư mục `crates/quaqua/tests/common/driver.rs` (giảm 85 LOC trong production code).
  3. Xóa bỏ các nhánh kiểm tra biến môi trường `HERDR_STUB_SNAPSHOT` trong `CliHerdrDriver` (`grid/apply.rs:78, 100, 151`).
  4. Tái cấu trúc `grid/hook.rs:215` (`reconcile_step`) để dùng chung engine so khớp hội tụ với `grid/reconcile.rs:151` (`run_reconcile`), triệt tiêu tình trạng check lệch predicate (như ghi nhận tại `tieude-gridfix.md`).
- **Rủi ro**: Trung bình. Bộ test `tests/grid.rs` phụ thuộc nhiều vào trạng thái mô phỏng của `FakeHerdrDriver`.
- **Cách verify**:
  - `cargo test --locked -p quaqua --test grid`

---

## 7. Bảng Tổng hợp So sánh Sau Restructure

| Tiêu chí | Hiện trạng (`a708fcd`) | Sau 6 Waves Restructure | Mức độ cải thiện |
| :--- | :---: | :---: | :--- |
| **Tổng số dòng code (LOC)** | 10,427 LOC | ~9,600 LOC | Giảm ~800 LOC dead code & duplicate |
| **Files >1,000 LOC** | 2 files (`pipeline`, `prompt`) | **0 files** (file dài nhất <600 LOC) | Triệt tiêu hoàn toàn god-files |
| **Hàm >100 LOC** | 16 hàm (lớn nhất 982 dòng) | **≤2 hàm** (`validate`, `md5_hex`) | Modular hóa rõ ràng |
| **Match statements >100 LOC** | 2 matches (404 dòng, 118 dòng) | **0 matches** | State machine hóa |
| **Cơ chế JSON** | 2 cơ chế song song (tự chế + serde) | 1 cơ chế duy nhất (`serde_json`) | Xóa 276 LOC parser thừa |
| **Shell Trampolining** | Có (Rust → Bash → Rust binary) | **Không** (100% in-process calls) | Tăng tốc độ, an toàn process env |
| **Mô hình Error Handling** | Phân mảnh (`(i32, String)` vs `String`) | Thống nhất (`ExitResult<T>`) | Loại bỏ hàng chục `map_err` chuyển đổi |
| **Backdoor test trong production** | Có (`HERDR_STUB_SNAPSHOT`, `FakeHerdrDriver`)| **Không** (được cô lập trong test) | Code production sạch, an toàn |

---

## Changes
- Không có bất kỳ thay đổi file code nào trong repository (tuân thủ nghiêm ngặt ràng buộc read-only).
- Đã xuất bản báo cáo audit kiến trúc đầy đủ tại `runs/sude-archaudit.md`.

## Test cmd+result
- Kiểm tra tính toàn vẹn biên dịch workspace:
  `cargo check --locked -p quaqua` → **OK** (0 warnings, 0 errors, thời gian 4.58s).
- Kiểm tra toàn bộ các test binary được compile hợp lệ:
  `cargo test --locked -p quaqua --no-run` → **OK** (Hoàn thành trong 2.73s).
- Chạy kiểm tra test suite `config`:
  `cargo test --locked -p quaqua --test config` → **OK** (`10 passed; 0 failed; 0 ignored; finished in 0.36s`).

## Risks
- **Rủi ro hồi quy Watchdog (Wave 4)**: Việc chia tách `worker/prompt.rs` có thể ảnh hưởng đến thứ tự kiểm tra mã thoát (exit code 2..6) nếu không giữ đúng thứ tự ưu tiên quan sát. *Biện pháp giảm thiểu: Giữ nguyên vẹn thứ tự các nhánh if trong `watchdog.rs` và chạy trọn vẹn `tests/pipeline_prompt.rs`*.
- **Rủi ro hội tụ Grid (Wave 6)**: Việc dọn dẹp `FakeHerdrDriver` có thể làm hỏng các test case edge layout trong `tests/grid.rs`. *Biện pháp giảm thiểu: Giữ nguyên contract API của `FakeHerdrDriver`, chỉ chuyển file từ `src/` sang `tests/`*.

## Lessons
- Module-level DAG không bảo đảm file-level decoupling: dù kiến trúc tổng thể là DAG, các phụ thuộc tiện ích (`find_quaqua_root`, `Launch`, `now_sec`) vẫn bị rò rỉ chéo khi thiếu tầng `util`/`workspace` chuẩn hóa — 2026-10-18, runs/sude-archaudit.md.

## Evidence
Mọi số liệu, dòng code và đồ thị trong báo cáo được trích xuất và đo đạc trực tiếp bằng các lệnh sau:
1. Đo LOC toàn bộ files:
   `wc -l crates/quaqua/src/**/*.rs crates/quaqua/src/*.rs | sort -n`
2. Kiểm tra chu trình phụ thuộc module (DFS Cycle Detection):
   `python3 -c "..."` (phân tích AST imports bỏ qua comments, xác nhận 0 cycles).
3. Tìm kiếm và đo chính xác các hàm >100 dòng:
   `python3 -c "..."` (quét toàn bộ định nghĩa hàm và đo depth mở đóng ngoặc nhọn).
4. Xác định các match statements khổng lồ:
   `python3 -c "..."` (phân tích các khối match và đếm dòng bao bọc).
5. Xác định dead code `strip_tab_suffix` và `Role`:
   `git grep -n "strip_tab_suffix" crates/quaqua/`
   `git grep -n "Role" crates/quaqua/`
6. Kiểm tra các điểm trùng lặp code:
   `git grep -n "fn now_sec" crates/quaqua/src/`
   `git grep -n "is_pid_alive" crates/quaqua/src/`
   `git grep -n "find_engine_script" crates/quaqua/src/`
   `git grep -n "FakeHerdrDriver" crates/quaqua/`
   `git grep -n "HERDR_STUB_SNAPSHOT" crates/quaqua/`

Verdict: clean
