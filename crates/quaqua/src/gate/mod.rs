//! `quaqua gate` — the PreToolUse(Bash) hook that replaced
//! `scripts/hooks/herdr-gate.sh`: a stateless tokenizer + policy check over the
//! hook JSON on stdin. A blocked command prints the message below to stderr and
//! exits 2; anything else exits 0.

pub mod policy;
pub mod tokenizer;

pub use policy::{check, check_segments, Verdict};

const BLOCK_MESSAGE: &str = "blocked: spawn/move panes and workspaces only via quaqua worker spawn / quaqua lead spawn / quaqua worker cleanup / quaqua lead cleanup or `quaqua grid reconcile` — rules/_all/fanout-agent-grid.md";

/// Entry point for `quaqua gate`: read the Claude Code hook JSON on stdin,
/// check `.tool_input.command`, print the block message to stderr and return
/// the process exit code (2 = blocked, 0 = allow).
pub fn run() -> i32 {
    let mut buf = String::new();
    {
        use std::io::Read;
        let _ = std::io::stdin().read_to_string(&mut buf);
    }
    let cmd = match extract_command(&buf) {
        Ok(cmd) => cmd,
        Err(()) => {
            // Non-empty input we cannot parse: the gate cannot evaluate the
            // command, so it fails closed instead of silently allowing it.
            // (bash's `jq` aborts with its own non-zero status here — rc 5 for
            // jq 1.8.1 — which Claude Code treats as a non-blocking error.)
            eprintln!("{BLOCK_MESSAGE}");
            return 2;
        }
    };
    if cmd.is_empty() {
        return 0;
    }
    match check(&cmd) {
        Verdict::Block => {
            eprintln!("{BLOCK_MESSAGE}");
            2
        }
        Verdict::Allow => 0,
    }
}

/// The hook contract: `{"tool_name":"Bash","tool_input":{"command":"…"}}`.
/// Missing/empty input yields an empty command (bash reads
/// `.tool_input.command // ""`; `jq` on empty/whitespace-only stdin produces no
/// output and exits 0). `Err` means non-empty input that is not valid JSON.
fn extract_command(json: &str) -> Result<String, ()> {
    if json.trim().is_empty() {
        return Ok(String::new());
    }
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => Ok(v
            .get("tool_input")
            .and_then(|t| t.get("command"))
            .and_then(|c| c.as_str())
            .map(String::from)
            .unwrap_or_default()),
        Err(_) => Err(()),
    }
}
