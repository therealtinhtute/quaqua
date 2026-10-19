# sude-auditor

## Summary
- **Commit**: `e69c1eb` on branch `quaqua/sude-auditor` (base `a708fcd`).
- **Bash 3.2 heredoc bug fix**: In `scripts/auditor/auditor.sh`, replaced `prompt=$(cat <<'EOF' ... EOF)` with `read -r -d '' prompt <<'EOF' || true`. In Bash 3.2, heredocs inside `$()` terminate prematurely when seeing unescaped `)` parentheses in the prompt text, exiting with a syntax error and leaving no report.
- **Claude -p prompt argument bug fix**: In `scripts/auditor/auditor.sh`, added `--` before `"$prompt"`. In Claude Code CLI (Commander.js), `--disallowedTools` takes variadic arguments `<tools...>`, so `"$prompt"` was swallowed as a tool pattern, triggering `Error: Input must be provided either through stdin or as a prompt argument when using --print`.
- **Opencode headless harness support**: Added `AUDITOR_HARNESS` support to `scripts/auditor/auditor.sh`. When `AUDITOR_HARNESS=opencode` (or auto-detected when `claude` is absent), the auditor executes `opencode run --standalone --auto -m "${AUDITOR_MODEL:-opencode-go/deepseek-v4.1-flash#low}" "$prompt"`, which runs headless without Claude, writing the final report to stdout (`reports/audit-YYYY-MM-DD-HH.md`) and diagnostic output to stderr.
- **Installer & test suite updates**:
  - `scripts/auditor/install-auditor.sh`: Supported detecting either `claude` or `opencode` for `job_path`, retaining diagnostic error when neither is present.
  - `tests/test_auditor.sh`: Pinned `--` in expected argv for the Claude read-only sandbox, and added sandbox test verifying `AUDITOR_HARNESS=opencode`.
  - `tests/test_install_auditor.sh`: Resolved a modern Bash (>=4.4) on the host for `$T/bin/bash` when the test runner is `/bin/bash` 3.2, ensuring the Darwin happy path passes while preserving the test verifying refusal of system Bash 3.2.
- **Live headless audit report**: Executed `AUDITOR_HARNESS=opencode bash scripts/auditor/auditor.sh` without Claude. Successfully generated `reports/audit-2026-10-18-12.md` (6.7 KB, 5 sections, `Verdict: 59 findings`).

## Test Results
- `bash tests/test_auditor.sh`: PASS (exit 0)
- `bash tests/test_install_auditor.sh`: PASS (exit 0)
- `/bin/bash tests/test_auditor.sh` (macOS system Bash 3.2): PASS (exit 0)
- `/bin/bash tests/test_install_auditor.sh` (macOS system Bash 3.2): PASS (exit 0)
- `cargo test --locked`: PASS (144 passed)
- Live audit: `reports/audit-2026-10-18-12.md` generated cleanly.

## Harness Comparison (opencode vs claude)
- `claude -p`: Strongly sandboxed via `--allowedTools` and `--disallowedTools "Bash(git *--output*)"`, with `--setting-sources ""`. The root cause of the previous failure was variadic argument consumption by `--disallowedTools`, fixed cleanly with `-- "$prompt"`.
- `opencode run`: Operates headlessly via `--standalone --auto`. `opencode-go/deepseek-v4.1-flash#low` completed in 90s. When tested with `#max`, DeepSeek looped extensively over unbounded shell commands (>300s timeout) because `opencode run` does not support tool whitelist filtering (`--allowedTools`). Therefore `#low` is chosen as the default for opencode auditor runs.

## Blockers
- Running `tests/run.sh --slow` all-in-one in this subshell exceeds the 300s command execution timeout because of the two unrelated slow suites (`test_prompt_worker.sh` and `test_pipeline.sh`), each taking several minutes. Both target auditor suites (`test_auditor.sh` and `test_install_auditor.sh`) pass 100% on both modern bash and `/bin/bash` 3.2.

## Lessons
- In Bash 3.2, heredocs inside `$(cat <<'EOF' ... EOF)` break if the text contains `)`; use `read -r -d '' var <<'EOF' || true` instead.
