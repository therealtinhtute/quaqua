//! `quaqua gate` — the PreToolUse(Bash) hook that replaced
//! `scripts/hooks/herdr-gate.sh`: a stateless tokenizer + policy check over the
//! hook JSON on stdin.
//!
//! Three outcomes, two wire forms:
//! - herdr spawn/move (`Block`): the message below on stderr, exit 2;
//! - outward actions (`Ask`/`Deny`): PreToolUse JSON on stdout, exit 0, so
//!   Claude Code shows its native confirm prompt (`ask`) or refuses the call
//!   (`deny`) — AGENTS.md rule 3;
//! - everything else: exit 0.

pub mod drift;
pub mod policy;
pub mod tokenizer;

pub use policy::{
    check, check_ctx, check_segments, check_with_pr, OfflinePrLookup, PrFacts, PrLookup, RealGh,
    Verdict,
};

const BLOCK_MESSAGE: &str = "blocked: spawn/move panes and workspaces only via quaqua worker spawn / quaqua lead spawn / quaqua worker cleanup / quaqua lead cleanup or `quaqua grid reconcile` — rules/_all/fanout-agent-grid.md";

/// Entry point for `quaqua gate`: read the Claude Code hook JSON on stdin,
/// check `.tool_input.command`, and print the decision.
pub fn run() -> i32 {
    let mut buf = String::new();
    {
        use std::io::Read;
        let _ = std::io::stdin().read_to_string(&mut buf);
    }
    let (cmd, cwd) = match extract(&buf) {
        Ok(x) => x,
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
    // The hook is the only caller that may run `gh`: a merge whose PR state
    // cannot be verified asks, and the lookup is timeout-bounded.
    match check_with_pr(&cmd, cwd.as_deref(), &RealGh::from_env()) {
        Verdict::Block => {
            eprintln!("{BLOCK_MESSAGE}");
            2
        }
        Verdict::Ask(reason) => {
            print_decision("ask", &reason);
            0
        }
        Verdict::Deny(reason) => {
            print_decision("deny", &reason);
            0
        }
        Verdict::Allow => {
            warn_on_drift(&buf, &cmd);
            0
        }
    }
}

/// The hook's `session_id`, when present.
fn session_id(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("session_id")?.as_str().map(String::from)
}

/// Allowed `quaqua worker|lead` call from a session whose rule files changed:
/// tell the agent which files to re-read (issue #72). Never blocks.
fn warn_on_drift(buf: &str, cmd: &str) {
    if !drift::is_dispatch(&tokenizer::tokenize(cmd)) {
        return;
    }
    let Some(sid) = session_id(buf) else { return };
    let changed = drift::drift(&crate::workspace::workspace_root(), &sid);
    if changed.is_empty() {
        return;
    }
    let msg = drift::drift_message(&changed);
    let out = serde_json::json!({
        "systemMessage": msg,
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": msg,
        }
    });
    println!("{out}");
}

/// `quaqua gate --session-start`: the SessionStart hook. Records the session's
/// rule baseline; after a compaction prints the re-read list (stdout becomes
/// session context). Always exits 0.
pub fn run_session_start() -> i32 {
    let mut buf = String::new();
    {
        use std::io::Read;
        let _ = std::io::stdin().read_to_string(&mut buf);
    }
    let v: serde_json::Value = serde_json::from_str(&buf).unwrap_or_default();
    let sid = v.get("session_id").and_then(|x| x.as_str()).unwrap_or("");
    let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("");
    if let Some(text) = drift::session_start(&crate::workspace::workspace_root(), sid, source) {
        println!("{text}");
    }
    0
}

/// The hook contract: `{"tool_name":"Bash","tool_input":{"command":"…"},"cwd":"…"}`.
/// Missing/empty input yields an empty command (bash reads
/// `.tool_input.command // ""`; `jq` on empty/whitespace-only stdin produces no
/// output and exits 0). `Err` means non-empty input that is not valid JSON.
/// `cwd` is only used to resolve the branch a bare `git push` would push.
fn extract(json: &str) -> Result<(String, Option<String>), ()> {
    if json.trim().is_empty() {
        return Ok((String::new(), None));
    }
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => {
            let cmd = v
                .get("tool_input")
                .and_then(|t| t.get("command"))
                .and_then(|c| c.as_str())
                .map(String::from)
                .unwrap_or_default();
            let cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
            Ok((cmd, cwd))
        }
        Err(_) => Err(()),
    }
}

/// The PreToolUse decision JSON Claude Code reads on stdout (exit 0). Built
/// through `serde_json` so a reason carrying a quote or a backslash — e.g. a
/// branch name in the deny reason — cannot break the JSON.
fn print_decision(decision: &str, reason: &str) {
    let out = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }
    });
    println!("{out}");
}
