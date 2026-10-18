# Harness effort & cost — audit + spec + plan

Status: **ready to execute** (agent khác chạy; ngoài SLP của quaqua) · audit by CoS 2026-10-17
Repo: `~/quaqua` · Evidence: `reports/usage-2026-10-17.md`, `runs/pipeline-*.md`, `~/.local/share/opencode/opencode.db` (read-only), console request-logs

## 0. TL;DR
1. `kinds.tieude.effort="max"` **chưa bao giờ có tác dụng**: `spawn.rs` bỏ qua `effort` cho harness `opencode`.
2. `quaqua config validate` chỉ so `models.md` ↔ `team.toml` (doc↔doc), không so với **runtime** → drift im lặng.
3. Watchdog/pipeline churn là chi phí lớn nhất về thời gian: 3 ngày có **13 lần watchdog exit** (4×6, 3×4, 5×1, 2×1, 1×1) và **21 dòng `status: blocked`** so với **1 dòng `accepted`**.
4. Tiền: 4 ngày = **$3.646** theo request-log (máy này $1.457 + máy khác $2.189 — anh iu xác nhận máy khác); meter Go nói $2.71 (lệch ~25%, chưa rõ công thức).
5. Cắt effort không phải đòn lớn: worker **đang chạy `high`** (mức rẻ); `max` chỉ ở session tương tác (2.2×/step). Đòn lớn còn lại: **bớt step + bớt respawn**.

## 1. Audit vòng 1 — chỗ tiền chảy (số thật)
- Ngày: 14/10 $0.0067 · 15/10 $0.2064 · **16/10 $3.1875** · 17/10 $0.2454. Model: `deepseek-v4.1-flash` = $3.64/$3.65.
- Cụm grid 16/10: **663 step · $0.844** · in 1.395M / out 188k / reasoning 358k / cacheRead 102.4M · **1/6 task vào main**; 2 task bị thay/kill ($0.23 = 27%), 5 nhánh còn trên integ.
- Cost split: cacheRead 36% · reasoning 25% · input 25% · output 13%. Per step: 2.104 in / 283 out / 540 reasoning / **154k cacheRead**.
- Phân loại tool-call: **product 71%**, **process 6%**, other 23% → giấy tờ không tốn token; churn mới tốn.

## 2. Audit vòng 2 — check code + runtime
### 2.1 effort cho `opencode` là config chết
- Code: `crates/quaqua/src/worker/spawn.rs:355` tính `effort` nhưng nhánh `"opencode"` (≈358) chỉ gọi `opencode -- --standalone --auto`; model truyền qua `OPENCODE_CONFIG_CONTENT = {"model": "<team.toml model>", "instructions": [...]}` — **không variant** (≈311). `omp` dùng `--thinking high`, `claude` dùng `--effort medium` → chỉ opencode bị bỏ.
- Runtime: 100% session tieude worker 16–17/10 (`gridfix`, `gridrs`, `gridown`, `gridown2`, `spawneff`, `gridfix3`, `capfix`, `blkfix`, `escstop`, `lfsync`, `gsping*`) chạy variant **`high`**; `max` chỉ có ở session tương tác (`ses_eb6b7d478…` = audit này, `tieude-smk1`).
- Giá variant (đo trên DB, không phải test chuẩn): `high` 1.289 step · reasoning 460/step · **$0.00105/step**; `max` 178 step · reasoning 731/step · **$0.00232/step** (2.2×).
- Catalog: `opencode-go/deepseek-v4.1-flash` = `low | high | max` (**không có `medium`**); `deepseek-v4.1-pro` = `high | max`.

### 2.2 `config validate` không bắt drift runtime
`quaqua config validate` → `ok` dù `team.toml` ghi `effort="max"` và runtime không thể áp dụng. `crates/quaqua/src/config.rs:410` so `(harness, model, effort)` giữa `models.md` và `team.toml` — cùng một mặt trận doc, không có mặt thứ ba.

### 2.3 Watchdog / pipeline churn
- 3 ngày: `watchdog exit` 4×6, 3×4, 5×1, 2×1, 1×1. Hôm nay `pipeline-gridapply.md`: blocked 3 lần (exit 1, exit 3, exit 3) trước khi có `commit c706700`; `pipeline-spawneff.md`: exit 4 (budget) → `--extend 2400` → **exit 2** (cap) và worker chết vì opencode-go API timeout sau 3h33m (WIP 17 file phải cứu bằng `.spawneff.wip.patch` rồi làm lại bằng omp).
- Nghi vấn 1: "cap counted 3 for gridown2 (reserve 2 + builder 1, double count)" làm `spawneff start refused` 2 lần.
- Nghi vấn 2: idle-contract giữa herdr `agent start` và TUI của omp/opencode (memory 17/10: "opencode state still idle-fallback, not hook").
- Nghi vấn 3: provider timeout (opencode-go) không có đường resume tại chỗ.

### 2.4 CI không hermetic
PR #44 đỏ sau push `8d5991d`: test spawn cần `herdr`, CI (ubuntu, `cargo test --locked`) không có. (Tên test lead báo: `test_spawn_worker_fallback_scenarios`; cần verify trên nhánh PR.)

### 2.5 Meter ≠ log
Tuần: request-log $3.6460 vs meter Go $2.7104; 5h: log $0.0703 vs meter $0.0148. Local estimate (DB) cũng khác (session audit: DB $0.42 vs log ~$0.07–0.25). Chưa xác định được công thức → để mở.

## 3. Research — hướng làm và căn cứ
### 3.1 Wire effort cho opencode (2 ứng viên, phải test thật)
- Schema `https://opencode.ai/config.json`: `model` nhận `anyOf`: string `^[^/#]+/[^#]+(?:#[^#]+)?$` **hoặc** object `{providerID, model, variant}` (cả 3 đều required trừ variant).
- Docs `opencode.ai/v2/docs/config`: *"The root default currently does not retain a `#variant`; agent and command model references can select one."* → string `#variant` ở root có thể bị bỏ; **object form** và **`agents.<name>.model`** là 2 đường còn lại.
- Đề xuất: thử `OPENCODE_CONFIG_CONTENT.model = {providerID, model, variant}` trước; nếu không ăn → `default_agent="build"` + `agents.build.model = "<provider>/<model>#<effort>"`. Đo bằng `opencode api GET /api/session/<id>` → `.data.model.variant`.

### 3.2 Cost budget — đọc cost thật bằng API local (không cần SQLite)
`opencode api GET /api/session/<sessionID>` trả `cost`, `tokens{input,output,reasoning,cache}`, `model.variant`, `time.idle`, `outcome` — đủ cho watchdog. Ví dụ thật:
`{"id":"ses_eb6b7d478…","model":{"id":"deepseek-v4.1-flash","providerID":"opencode-go","variant":"max"},"cost":0.4242,"tokens":{...}}`
Fallback: đọc read-only `session_v2 WHERE directory = <worktree>` (đã kiểm chứng chạy được, không cần checkpoint WAL).
Muốn list session theo directory thì phải kiểm tra route list (`/api/session…`) — nếu không có, dùng DB.

### 3.3 Usage audit command (text-only, không dashboard)
Nguồn: console request-logs (`/console/api/request-logs`, paged; token lấy runtime từ `opencode auth export opencode`, **không bao giờ in ra**), `/inference/go/v1/usage` (meter), local API/DB (attribution), `runs/*.md` + git (outcome). Ra bảng: ngày · task (steps/tokens/$/wall/outcome) · product-vs-process · waste flags (respawn, killed, idle>30′, review rounds) · meter · **cảnh báo máy lạ** (UA version ≠ local). `--days N`, `--json`.

### 3.4 Churn
Không đoán — cần 1 repro nhỏ cho mỗi nguyên nhân (fake herdr/pane, fake agent idle, cap math) rồi sửa đúng chỗ; ưu tiên 2 cái lặp nhiều nhất (exit 4 và exit 3).

## 4. Spec & Plan (task cho agent thực hiện)
Quy ước: mỗi W = 1 nhánh `quaqua/<tên>` + 1 commit (hoặc nhiều commit nhỏ), test hermetic, không push/merge.

- **W1 — Effort wiring + validator (bắt buộc)**
  - `spawn.rs`: truyền effort cho `opencode` (object form trước, fallback agents.form). Giữ nguyên hành vi `omp`/`claude`.
  - `config.rs`: validate phải **fail** khi `team.toml` khai `effort` mà harness không áp dụng được, hoặc kind thiếu `effort` khi harness cần; so thêm với catalog variant (`low|high|max`) nếu đọc được `models.opencode.ai/api.json`.
  - Acceptance: (a) `cargo test --locked -p quaqua` xanh + test mới khẳng định JSON có variant; (b) live: spawn `tieude-gspingeff` với `effort="low"` rồi `"max"`, `opencode api GET /api/session/<id>` trả đúng variant; (c) `quaqua config validate` non-zero trên fixture team.toml cố tình sai.
- **W2 — Cost budget (bắt buộc)**
  - `worker prompt`/`prompt-worker.sh`: đọc `Cost budget: $<x>`; mỗi vòng poll đọc cost session của worker (API local; fallback DB read-only); in `WATCHDOG cost 80%` / `100%` **một lần**, không kill, không re-send.
  - Acceptance: test với fake clock + fixture; demo live budget `$0.001` → thấy dòng 80%, worker vẫn hoàn thành; không regression time-budget.
- **W3 — Usage audit command (khuyến nghị)**
  - `scripts/audit/usage.py` (hoặc `quaqua usage`), text-only, `--days`, `--json`; kèm 1 test offline (fixture nhỏ, không gọi mạng).
  - Acceptance: chạy `--days 4` ra đúng bộ số trong `reports/usage-2026-10-17.md` (±2% cost); grep output không có `st_`/`rt_`; test offline xanh.
- **W4 — Watchdog churn (bắt buộc, sau repro)**
  - Repro + fix cho: (a) exit 4 lặp (budget đã hết nhưng agent còn tiến triển), (b) exit 3 lặp (idle/stall detection với omp/opencode), (c) cap double-count (`reserve + builder`), (d) provider timeout → resume cùng session thay vì spawn mới.
  - Acceptance: fixtures mới trong `tests/test_prompt_worker_watchdog.sh`; 1 lần chạy kiểu gridapply không sinh `- status: blocked` giả; `cargo test --locked -p quaqua` xanh.
- **W5 — CI hermetic (bắt buộc)**
  - Test spawn không được cần `herdr` thật: shim `herdr` qua PATH (hoặc bỏ nhánh live khỏi `cargo test`, để trong `tests/run.sh --slow`).
  - Acceptance: `PATH=/usr/bin:/bin cargo test --locked` xanh (không có herdr); CI PR #44 xanh.
- **W6 — Policy effort (chỉ doc + config, cần anh iu quyết)**
  - Interactive/CoS mặc định `high`; `max` chỉ khi được yêu cầu; ghi 2.2×/step làm căn cứ. Không tự đổi `team.toml` giá trị — chỉ wire, để anh iu chọn số.

## 5. Open questions (anh iu)
1. `kinds.tieude.effort` = `low|high|max` — chọn số nào (khuyến nghị `high`, `max` cho pha plan)?
2. Có muốn W3 (audit command) hay giữ audit ad-hoc theo yêu cầu?
3. Meter-vs-log lệch ~25%: mở ticket cho opencode hay bỏ qua?

## 6. Prompt dán cho agent thực hiện

```text
Đọc trước: ~/quaqua/docs/plans/active/harness-effort-cost-audit.md (audit + spec + plan, có số và file:line).

Repo: ~/quaqua (Rust workspace, crate crates/quaqua). Làm trên nhánh riêng quaqua/<task>, mỗi workstream 1 nhánh + commit. KHÔNG push, KHÔNG merge main.

Bối cảnh đã kiểm chứng (không cần chứng minh lại):
1) crates/quaqua/src/worker/spawn.rs:355 tính `effort` nhưng nhánh harness "opencode" (≈358) bỏ qua nó; model chỉ truyền qua OPENCODE_CONFIG_CONTENT = {"model": "...", "instructions": [...]} → worker tieude chạy variant `high` (default), còn team.toml khai effort="max" (config chết). omp dùng --thinking, claude dùng --effort.
2) `quaqua config validate` trả ok dù runtime không áp dụng được effort: crates/quaqua/src/config.rs:410 chỉ so models.md ↔ team.toml.
3) Churn: 3 ngày có 13 watchdog exit (4×6, 3×4, 5×1, 2×1, 1×1) và 21 dòng `- status: blocked` so với 1 `accepted`; pipeline-gridapply hôm nay blocked 3 lần trước khi commit.
4) CI PR #44 đỏ vì test spawn cần herdr thật (ubuntu runner không có).

Việc cần làm, theo thứ tự ưu tiên:
W1 — Wire effort cho opencode + validator bắt drift runtime.
   Thử theo thứ tự: (a) OPENCODE_CONFIG_CONTENT.model dạng object {providerID, model, variant}; (b) default_agent="build" + agents.build.model="<provider>/<model>#<variant>". Căn cứ: schema opencode.ai/config.json cho model anyOf string-có-#variant | object; docs nói root model không giữ #variant nhưng agent/command giữ.
   Đo thật bằng: opencode api GET /api/session/<id> → .data.model.variant (phải đúng effort khai báo).
   Validator: fail khi kind khai `effort` mà harness không áp dụng được, hoặc thiếu `effort` khi harness cần.
   Acceptance: cargo test --locked -p quaqua xanh + test mới khẳng định JSON có variant; live spawn 1 worker effort="low" và 1 worker "max", đọc variant đúng; `quaqua config validate` non-zero trên fixture sai.
W2 — Cost budget cho worker: đọc `Cost budget: $<x>` trong brief; mỗi poll đọc cost session (opencode api GET /api/session/<id>, fallback: SQLite read-only session_v2 theo directory) và in `WATCHDOG cost 80%` / `100%` một lần, không kill, không re-send. Test với fake clock + fixture; demo live budget $0.001.
W3 — scripts/audit/usage.py (hoặc `quaqua usage`), text-only, --days/--json: ghép console request-logs (token lấy runtime qua `opencode auth export opencode`, KHÔNG in ra), /inference/go/v1/usage, local DB/API, runs/ + git. Ra: bảng ngày · task (steps/tokens/$/wall/outcome) · product-vs-process · waste flags (respawn, killed, idle>30′, review rounds) · meter · cảnh báo máy lạ. Test offline bằng fixture (không gọi mạng).
W4 — Churn: repro + fix 4 nguyên nhân: (a) exit 4 lặp khi budget hết nhưng agent còn tiến triển; (b) exit 3 lặp do idle/stall detection với omp/opencode; (c) cap double-count (reserve 2 + builder 1); (d) provider timeout (opencode-go) → resume cùng session thay vì spawn mới. Fixture trong tests/test_prompt_worker_watchdog.sh.
W5 — CI hermetic: shim `herdr` qua PATH cho test spawn (hoặc đẩy nhánh live sang tests/run.sh --slow). Acceptance: PATH=/usr/bin:/bin cargo test --locked xanh; CI PR xanh.

Ràng buộc:
- Không đổi giá trị trong team.toml (chỉ wire; anh iu chọn số effort).
- Không in/ghi token, credential; đọc ~/.local/share/opencode/opencode.db bằng mode=ro.
- Mọi kết luận kèm command + kết quả thật; không suy đoán.
- Test phải hermetic; live test tách riêng và ghi lại lệnh.

Bàn giao: file runs/<name>.md với `## Evidence` (command + kết quả) cho từng W, `## Blockers` nếu có, kết thúc bằng dòng `Verdict:`. Mỗi W nêu: branch, commit SHA, lệnh verify, kết quả.
```

## 7. Guardrails
- Không `git push`, không merge `main`, không sửa giá trị `team.toml` (chỉ wire).
- Không in/ghi token; audit đọc `opencode.db` bằng `mode=ro`.
- Mọi kết luận phải kèm command + kết quả (bằng chứng), không suy đoán.
