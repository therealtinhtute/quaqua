# Evidence index — bản published cho cross-machine

**Policy:** `raw/`, `runs/`, `reports/`, `memory/` là **local-only by design** (`.gitignore`). Tham chiếu trong file đã track (rules, AGENTS.md, code comments, PR/issue) nên trỏ về `docs/research/**` để máy khác đọc được; nếu chỉ có bản local thì ghi chú rõ tại đây.

**Chuẩn hoá khi publish:** mọi bản copy đã chuẩn hoá đường dẫn home cá nhân (dạng /Users/… hoặc /home/…) → `~` (vừa qua portable-paths hook, vừa tránh lộ path máy). Nội dung khác giữ nguyên byte.

## Published (đợt 18/10)
| Citation nguồn (local-only) | Bản published |
|---|---|
| `runs/quaqua-intake-lead.md` | `evidence/quaqua-intake-lead.md` |
| `runs/sude-guard.md` | `evidence/sude-guard.md` |
| `runs/tieude-gatefix.md` | `evidence/tieude-gatefix.md` |
| `runs/tieude-gridfix.md` | `evidence/tieude-gridfix.md` |
| `raw/2026-10-17-tieude-evtprobe.md` | `probes/evtprobe/REPORT-2026-10-17.md` (PR #50) |
| `raw/2026-10-18-*.md` (39 file — toàn bộ evidence ngày 18/10) | `evidence/2026-10-18-*.md` |
| `docs/plans/active/*.md` (local plan files) | `plans/*.md` (bản copy; bản gốc vẫn ở `docs/plans/`) |

## Cited nhưng đã mất local — không publish được
`suhuynh-r5r1.md` · `tieude-capfix2.md` · `mu-lead.md` · `sude-stale.md` · `sude-rdone.md` · `sude-quick.md` · `baro-lead.rules.md` · `coucou-island-lead.rules.md` · `raw/2026-10-11-x.md` — các lesson trong `rules/` cite tên cũ; file đã bị cleanup theo vòng đời `runs/` (ngoài git history vì `runs/` không track).

## Không phải evidence — fixture/placeholder
`runs/pipeline-*.md` (ok/blocked/dispute…), `sude-quick/sude-slow/sude-noles…`, `runs/test.rules.md`, `some-worker.md` — tên ví dụ trong test/docs, không cần publish.
