# Worker roster

Anh gọi theo biệt danh → em spawn đúng `name` / kind / model / effort. Split pane với `--cwd <repo>`.

| Biệt danh | name | kind | Launch args (sau `--`) | Dùng cho |
|---|---|---|---|---|
| tiểu đệ | `tieude` | `pi` | `--model antigravity/gemini-3.8-flash` | Việc vặt: sửa nhỏ, docs, rename, chạy lệnh, tra cứu nhanh |
| sư đệ | `sude` | `pi` | `--model devin/deepseek-v4-1-flash:max` | Việc rẻ cần suy luận sâu: tra cứu, audit/plan read-only, sửa nhỏ |
| đệ tử ("đám worker") | `worker` | `pi` | `--model devin/swe-2:max` | Implementation mặc định; fan-out nhiều đệ tử |
| hộ pháp | `hophap` | `claude` | `--model claude-sonnet-5-5 --effort medium` | Feature vừa, viết test, refactor có phạm vi |
| mưu sĩ | `muusi` | `claude` | `--model claude-opus-5-5 --effort low` | Sketch, brainstorm, sửa dễ, cần trả lời nhanh |
| cao thủ | `caothu` | `claude` | `--model claude-opus-5-5 --effort medium` | Làm feature, việc hằng ngày. **Chỉ spawn khi anh gọi** |
| tông sư | `tongsu` | `claude` | `--model claude-opus-5-5 --effort high` | Sửa bug, đuổi edge case, verify quan trọng, bug codebase cũ. **Chỉ spawn khi anh gọi** |
| chưởng môn | `chuongmon` | `claude` | `--model claude-opus-5-5 --effort max` | Việc khó chạy trọn từ build đến verify, săn lỗ hổng. **Chỉ spawn khi anh gọi** |
| đấng cứu thế | `dangcuuthe` | `codex` | `-m <astra-id> -c model_reasoning_effort=medium` | Second opinion, review nhanh, logic hóc |
| lão sư | `reviewer` | `codex` | `-m gpt-6.1-sol -c model_reasoning_effort=high` | Review diff/PR kỹ, tìm bug; read-only. Báo repo path. `cd ~/agents/reviewer` |

Spawn: `herdr agent start <name> --kind <kind> --pane <id> -- <launch args>`.
- Sư đệ (pi): sau `agent start` gửi `/advisor` để bật advisor gates (executor trùng deepseek nên không đổi model). Đệ tử/tiểu đệ: KHÔNG bật (advisor ép model về executor).
- pi (pi-mono) qua Herdr integration v9; extension: pi-advisor-flow (advisor swe-2, gates plan/failure/completion/loop, `~/.pi/agent/advisor.json`), pi-supervisor (`/supervise <goal>`, model per-cwd `.pi/supervisor-config.json`). `agent_prompt_stalled` → vẫn `agent read`, không resend.
- `<astra-id>`: TODO anh xác nhận model id chính xác của Astra (và `gemini-3.8-flash`).

Rules:
- Implementation and review go to different kinds when possible.
- Keep 1–3 workers alive at once per project (up to 4 per project during fan-out, see `herdr.md`). Close idle ones.
