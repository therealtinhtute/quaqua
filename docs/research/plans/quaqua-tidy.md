# quaqua-tidy — dọn nốt tầng test-script (Nhóm 1) + rename fixtures

Status: **ready to execute** · CoS 2026-10-18 · nguồn: audit `raw/2026-10-18-quaqua-sude-testaudit-r2.md` §Proposals
Repo: `~/quaqua` · Base: main sau PR #49

## 0. TL;DR
- **T1 — Retire Nhóm 1** (4 suite bash, ~701 dòng) đã **trùng 100% coverage với Rust tests**: `test_config.sh`, `test_worker_cap.sh`, `test_taskforce_cap.sh`, `test_pre_commit.sh`. Trước khi xoá: sàng từng case — case nào CHƯA có tương ứng Rust thì port sang Rust test, rồi xoá file + refs.
- **T2 — Rename `tests/fixtures/herdr-grid` → `herdr-quaqua`** + update refs (danh sách dưới).
- **Không làm**: Nhóm 2 (merge dần khi có dịp — keep list), Nhóm 3 (e2e — giữ nguyên), `scripts/` (đã về 2 file + units).
- Kết quả kỳ vọng: tests/ từ 22 → **18 suite** (giảm 4), −~700 dòng; gate `run.sh --slow` xanh với số suite mới; cargo không giảm coverage (đã kiểm chứng từng case).

## 1. Chi tiết

### T1 — Retire Nhóm 1 (4 suite)
| Suite | Dòng | Coverage Rust đối ứng | Việc |
|---|---|---|---|
| `tests/test_config.sh` | 109 | `crates/quaqua/tests/config.rs` (20 fixture, get, cap, rule→key) | Sàng case → port case thiếu (nếu có) → xoá |
| `tests/test_worker_cap.sh` | 195 | `worker_unit.rs` (cap refusal, owner isolation, pipeline slot) | nt |
| `tests/test_taskforce_cap.sh` | 173 | `worker_unit.rs` (cap weights, taskforce reservation) | nt |
| `tests/test_pre_commit.sh` | 194 | `init.rs` (embedded template + hooksPath + refresh) | nt — lưu ý case portable-paths behavior (9 case) có thể cần 1 Rust test riêng cho hàm hook? Hook là shell-text template → case behavior chạy qua init sandbox như hiện tại — nếu port khó, GIỮ suite này (ghi lý do). |

Quy tắc sàng (bắt buộc, mỗi file): liệt kê từng `ok:`/case trong suite → đối chiếu tên case Rust → case nào không đối ứng 1-1 → viết test Rust mới (co-located, `assert_cmd`-free, theo mẫu worker_unit) → chỉ xoá file khi danh sách còn 0 case mồ côi. Ghi bảng "case → nơi port" vào report.
- Xoá file + mọi refs (`grep` `test_config|test_worker_cap|test_taskforce_cap|test_pre_commit` trong `tests/run.sh`, README, docs, slow_cover).
- Gate: `env -u HERDR_BIN_PATH -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin bash tests/run.sh --slow` → 0 failed (số suite = 22 − số file xoá thực tế); `cargo test --locked` không giảm số pass ròng (case port +N).

### T2 — Rename fixtures
- `git mv tests/fixtures/herdr-grid tests/fixtures/herdr-quaqua`
- Refs cần update (từ plugren report §Exceptions): `crates/quaqua/tests/grid.rs:64`, `tests/test_herdr_grid_rust.sh:4,10` (+ mktemp prefix ~:18 cosmetic), `tests/test_herdr_grid_plan.sh:39`, `tests/test_prompt_worker.sh:342`.
- Gate: cargo grid suite + 2 suite herdr_grid_* xanh; grep 0 ref `fixtures/herdr-grid` (trừ docs/research + memory — lịch sử).

## 2. Guardrails
- Base main sau #49; mỗi T 1 nhánh `quaqua/<worker>`; không push; merge theo quy trình.
- KHÔNG đụng Nhóm 3, `scripts/`, `crates/quaqua/src` (chỉ thêm test Rust nếu T1 sàng ra case mồ côi).
- KHÔNG xoá suite nào khi bảng sàng chưa đủ (0 case mồ côi hoặc port xong).
- Hermetic: mọi suite/test chạy phải pin (đã có từ hermpin); giữ nguyên.
- **Định dạng report**: heading đúng `## Blockers` / `## Lessons` (không đánh số); `## Blockers` body đúng chữ `none`.

## 3. Team & thứ tự
- T1 và T2 **file-area không đụng nhau** → **2 worker song song** (T1 chạm 4 suite + worker_unit/config/init tests; T2 chạm fixtures + grid.rs + 3 suite khác).
- Thứ tự merge: cái nào xanh trước merge trước.

## 4. Prompt dán cho executor
```text
Đọc trước: ~/quaqua/docs/plans/active/quaqua-tidy.md (chi tiết T1/T2, quy tắc sàng, guardrails).
Repo ~/quaqua, base = main sau PR #49. Làm ĐÚNG 1 task (T1 retire Nhóm 1 / T2 rename fixtures) — không gộp.
T1: BẮT BUỘC kèm bảng "case → nơi port (Rust test mới hoặc có sẵn)" trước khi xoá file; case mồ côi phải port sang Rust trước.
Gate mỗi task: cargo test --locked + env -u HERDR_BIN_PATH -u QUAQUA_OWNER -u GRID_TAB PATH=/usr/bin:/bin:$HOME/.cargo/bin bash tests/run.sh --slow (0 failed).
Report: runs/<name>.md — ## Evidence (command + kết quả), ## Blockers (đúng chữ none), ## Lessons. Không push.
```
