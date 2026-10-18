# quaqua-poll — dọn `scripts/` về 2 file (auditor · usage · pre-commit → Rust)

Status: **ready to execute** — chạy SAU khi `quaqua-clean` merge main (anh iu 2026-10-18) · từ phiên think của CoS
Repo: `~/quaqua` · Base: main sau merge quaqua-clean (`c404f9b`+) · Evidence: phiên think 18/10 + inventory đo trên clean-integ

## 0. TL;DR (đã chốt cùng anh iu)
- **Đích:** `scripts/` = **2 executable file** — `scripts/hooks/herdr-gate.sh` (Claude integration: `.claude/settings.json:10` trỏ path — GIỮ) + `scripts/auditor/install-auditor.sh` (install script — GIỮ). Units launchd/systemd = config (giữ).
- **P1:** `scripts/auditor/auditor.sh` (235 dòng) → `quaqua auditor run`; units đổi ExecStart; xoá script khi xanh.
- **P2:** `scripts/audit/usage.py` (785 dòng) → `quaqua usage`; thêm dep **`ureq`** (rustls, pin version); offline fixtures; parity ±2%.
- **P3:** `scripts/hooks/pre-commit` (65 dòng) → `quaqua init` **sinh từ template nhúng** vào hook dir do init quản; repo bỏ file.
- **Nguyên tắc:** 3 phase ĐỘC LẬP — mỗi phase merge xanh mới xoá script tương ứng; **không gom test cuối** cho đợt này (mỗi phase xoá file nên phải gate ngay).

## 1. Hiện trạng (đo trên clean-integ, 18/10)
| File | Dòng | Vai trò | Ai trỏ vào |
|---|---|---|---|
| `scripts/auditor/auditor.sh` | 235 | self-audit 2×/ngày: build prompt + chạy harness headless + ghi `reports/audit-*.md` | units: `ExecStart=%h/quaqua/scripts/auditor/auditor.sh`, launchd `ProgramArguments = [@BASH@, <root>/scripts/auditor/auditor.sh]` |
| `scripts/auditor/install-auditor.sh` | 65 | installer units | thủ công + `tests/test_install_auditor.sh:14,50,78` |
| `scripts/auditor/{launchd,systemd}/*` | — | schedule config | installer |
| `scripts/audit/usage.py` (+`tests/`, `fixtures/`) | 785 | usage audit (console logs + meter + DB) | thủ công (không có trong `tests/run.sh`) |
| `scripts/hooks/herdr-gate.sh` | 39 | gate Claude Code: chặn `herdr pane split/move` trực tiếp | `.claude/settings.json:10` (GIỮ) |
| `scripts/hooks/pre-commit` | 65 | git hook portable-paths | `quaqua init` set `core.hooksPath=<root>/scripts/hooks` (`init.rs:105`); `tests/test_pre_commit.sh` |

## 2. Phases

### P1 — auditor.sh → `quaqua auditor run`
- Subcommand mới: `quaqua auditor run` (tùy chọn `--dry-run` để test): port nguyên hành vi — build prompt read-only, chọn harness (`AUDITOR_HARNESS`: mặc định `opencode run --standalone --auto -m <model>#low`; fallback `claude -p` với `--`), ghi `reports/audit-YYYY-MM-DD-HH.md`, diagnostics ra stderr; **không schedule** (schedule = units).
- `install-auditor.sh`: units ghi `ExecStart = <quaqua-bin> auditor run` (resolve `command -v quaqua` lúc cài); launchd tương tự.
- Tests: `tests/test_auditor.sh` → điều khiển binary qua `QUAQUA_BIN` (stub harness như hiện tại); `tests/test_install_auditor.sh` → assert ExecStart/ProgramArguments mới.
- Acceptance: 2 suite xanh (cả `/bin/bash` 3.2 lẫn bash mới như T2 hiện tại); slow gate không tệ hơn baseline.
- Xoá khi xanh: `scripts/auditor/auditor.sh`.

### P2 — usage.py → `quaqua usage`
- Subcommand: `quaqua usage [--days N] [--until <ms>] [--json]`: port nguyên hành vi bản python — console request-logs (paged; token lấy runtime qua `opencode auth export`, KHÔNG bao giờ in), meter `/inference/go/v1/usage`, attribution local (opencode API / DB `mode=ro`), outcomes từ `runs/`+git, bảng text + `--json`, markers `[UNAVAILABLE]/[PARTIAL]`, exit `0` khi có logs / `2` khi request-logs mất hẳn; giữ retry (attempts/backoff, 4xx fail-fast).
- Dep: **`ureq`** (pin cụ thể) + rustls; cập nhật `Cargo.lock`; không dep nào khác.
- Tests offline: dùng CHÍNH fixtures cũ → chuyển `scripts/audit/tests/fixtures/{meter.json,request_logs.json}` sang `crates/quaqua/tests/fixtures/usage/`; golden 1 lần: so output Rust vs python trên cùng fixtures (ghi report).
- Acceptance parity: live `--days 1` cả 2 bản → cost lệch ≤2%; `grep -cE 'st_|rt_'` = 0.
- Xoá khi xanh: `scripts/audit/` (sau khi fixtures đã chuyển).

### P3 — pre-commit → `quaqua init` sinh template
- Nhúng template (giữ nguyên logic 65 dòng portable-paths) trong binary; `quaqua init` khi chạy: tạo hook dir do init quản (**đề xuất `<root>/.git/quaqua-hooks/`** — không tracked; nếu có lý do muốn khác thì ghi trong report), ghi `pre-commit` (+x), set `core.hooksPath` = dir mới; idempotent (refresh khi nội dung khác); migrate legacy: nếu `hooksPath` đang là `<root>/scripts/hooks` → chuyển + log rõ.
- `scripts/hooks/pre-commit` xoá khỏi repo; `tests/test_pre_commit.sh` đổi: init sandbox → chạy hook sinh ra → giữ nguyên toàn bộ case hiện có.
- Acceptance: `test_pre_commit.sh` xanh; workspace thật `core.hooksPath` trỏ dir mới; commit thử vi phạm bị chặn.
- Edge: `herdr-gate.sh` vẫn ở `scripts/hooks/` (tracked) — không nhầm với hooksPath.

## 3. Guardrails
- Mỗi phase 1 nhánh `quaqua/<worker>`; không push; merge theo quy trình duyệt của anh iu.
- Script cũ chỉ xoá trong CÙNG phase khi suite tương ứng xanh — không xoá sớm.
- Hermetic: tests không gọi mạng/harness/herdr thật (stub + fixtures); bằng chứng "0 rò rỉ" như sude-w3m đã làm.
- **Định dạng report** (mọi worker): heading đúng `## Blockers` / `## Lessons`; `## Blockers` body đúng chữ `none` khi không có.

## 4. Open
- Không còn quyết định mở (3 câu đã chốt). Ghi chú thực thi P3: vị trí hook dir — `.git/quaqua-hooks/` là đề xuất; ai làm thấy vướng thì ghi lý do + phương án thay trong report.

## 5. Team & thứ tự (đề xuất)
- P1, P2, P3 **file-area khác nhau** (auditor+units · usage+Cargo · init+tests) → **3 worker song song** (cap 8 thoải mái), verify từng cái.
- Thứ tự merge: P3 (nhỏ) → P1 → P2 (lớn nhất); hoặc theo thời điểm xanh.
- Sau 3 phase: `scripts/` còn đúng 2 file + units; chạy lại full gate + slow gate trên main.

## 6. Prompt dán cho executor (khi mở team)
```text
Đọc trước: ~/quaqua/docs/plans/active/quaqua-poll.md (đích, phase, acceptance, guardrails).
Repo ~/quaqua, base = main sau merge quaqua-clean. Mỗi phase 1 nhánh quaqua/<worker>, không push.
Làm ĐÚNG 1 phase theo brief riêng (P1 auditor / P2 usage / P3 pre-commit) — không gộp, không làm phase khác.
Script cũ chỉ được xoá trong phase của nó, khi suite tương ứng đã xanh.
Report: runs/<name>.md với ## Evidence (command + kết quả), ## Blockers (đúng chữ none nếu không có), ## Lessons.
```
