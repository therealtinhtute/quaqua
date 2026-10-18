# sude-pollaud

Port `scripts/auditor/auditor.sh` (235 lines bash) to Rust subcommand `quaqua auditor run` with exact behavioral parity, update scheduling units and test suites, and remove `scripts/auditor/auditor.sh`.

## Changes
- `crates/quaqua/src/auditor.rs`: Implemented `quaqua auditor run` subcommand covering:
  - Deterministic pre-checks: pipeline runs (`runs/pipeline-*.md` status clean/blocked/disputed, commit resolution via `git rev-parse`, reviewer `Verdict:`, recorded repo resolution), unresolved challenges (`runs/*.md`), live-worker sandbox checks (`herdr agent list`), instruction doc-rot line counts (>100 lines).
  - CLI options: `--dry-run`, `--out <path>`, `--model <model>`, `--harness <harness>`, `--repo <path>`, plus env overrides (`AUDITOR_OUT`, `SUPERVISOR_OUT`, `AUDITOR_MODEL`, `AUDITOR_HARNESS`, `AUDITOR_REPO`, `QUAQUA_ROOT`).
  - Headless harness invocation: `opencode run --standalone --auto -m <model>` and `claude -p --model <model> --permission-mode dontAsk --setting-sources "" --allowedTools ... --disallowedTools ... -- <prompt>`.
  - Atomic report file generation with truncated stdout redirection behavior and failure status stubs.
- `crates/quaqua/src/lib.rs`: Registered `pub mod auditor;`.
- `crates/quaqua/src/main.rs`: Minimal add-only registration of `Command::Auditor` and dispatch to `quaqua::auditor::handle_auditor`.
- `scripts/auditor/launchd/com.quaqua.auditor.plist.in`: Updated `ProgramArguments` to `@QUAQUA@ auditor run`.
- `scripts/auditor/systemd/quaqua-auditor.service`: Updated `ExecStart` to `%h/.cargo/bin/quaqua auditor run`.
- `scripts/auditor/install-auditor.sh`: Resolved `quaqua` binary path via `command -v quaqua`, substituted `@QUAQUA@`, and included `dirname "$quaqua_bin"` in launchd `job_path`.
- `tests/test_auditor.sh`: Driven binary via `QUAQUA_BIN` for `quaqua auditor run --repo "$T/repo"`, updated unit assertions.
- `tests/test_install_auditor.sh`: Updated assertions for ProgramArguments `[quaqua_bin, 'auditor', 'run']`, added test case ensuring `quaqua` must be on PATH before installing.
- `tests/test_docs.sh` & `README.md`: Updated entrypoint assertions and documentation to reflect `quaqua auditor run`.
- `scripts/auditor/auditor.sh`: Deleted legacy bash script after all suites passed green.

## Evidence
- Commit SHA: `ae9a7ef`
- `git diff --stat HEAD~1..HEAD`:
```
 README.md                                          |   4 +-
 crates/quaqua/src/auditor.rs                       | 865 +++++++++++++++++++++
 crates/quaqua/src/lib.rs                           |   1 +
 crates/quaqua/src/main.rs                          |  14 +
 scripts/auditor/auditor.sh                         | 235 ------
 scripts/auditor/install-auditor.sh                 |   9 +-
 .../auditor/launchd/com.quaqua.auditor.plist.in    |   5 +-
 scripts/auditor/systemd/quaqua-auditor.service     |   4 +-
 tests/test_auditor.sh                              |  23 +-
 tests/test_docs.sh                                 |   8 +-
 tests/test_install_auditor.sh                      |  30 +-
 11 files changed, 937 insertions(+), 261 deletions(-)
```
- `env -u HERDR_BIN_PATH PATH=/usr/bin:/bin:$HOME/.cargo/bin cargo test --locked`: PASS (172 passed across 11 suites).
- `bash tests/test_auditor.sh`: PASS
- `/bin/bash tests/test_auditor.sh`: PASS (macOS /bin/bash 3.2 compatibility confirmed)
- `bash tests/test_install_auditor.sh`: PASS
- `/bin/bash tests/test_install_auditor.sh`: PASS
- `bash tests/test_docs.sh`: PASS
- Real run / dry-run smoke test:
  `./target/debug/quaqua auditor run --dry-run --out /tmp/smoke-audit.md`
  Verified generated report contains expected headings (`## Drift`, `## Stale docs`, `## Unhandled`, `## Pipeline`, `## Workers`, `Verdict: 25 findings`).
- Grep verification:
  - `grep -rn 'auditor run' scripts/auditor/systemd/ scripts/auditor/launchd/`: units correctly point to binary subcommand `auditor run`.
  - `test -f scripts/auditor/auditor.sh`: returncode 1 (file deleted).
  - Preserved files: `install-auditor.sh`, `systemd/quaqua-auditor.service`, `systemd/quaqua-auditor.timer`, `launchd/com.quaqua.auditor.plist.in`.

## Risks
- Low risk: Subcommand flags and environment variables mirror `auditor.sh` exactly; headless flags for `opencode` and `claude -p` match the pinned argv.
- On macOS, launchctl uses the resolved absolute path to the `quaqua` binary recorded at install time; users moving the `quaqua` binary must re-run `install-auditor.sh`.

## Blockers
none

## Lessons
- In bash, redirection `> "$out"` truncates output files prior to process launch; mimicking this truncation in Rust ensures failure fallback checks (`-s $out`) behave identically when the process produces no stdout.
