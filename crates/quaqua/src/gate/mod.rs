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
    check, check_ctx, check_segments, check_with_lookups, check_with_pr, OfflinePaneLookup,
    OfflinePrLookup, PaneLookup, PrFacts, PrLookup, RealGh, RealHerdrPaneLookup, Verdict,
};
use std::path::{Path, PathBuf};

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
    match check_with_lookups(
        &cmd,
        cwd.as_deref(),
        &RealGh::from_env(),
        &RealHerdrPaneLookup::from_env(),
    ) {
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
    let herdr = crate::herdr::Herdr::new();
    let ws_root = crate::workspace::workspace_root();
    run_session_start_with(&herdr, &buf, &ws_root, None)
}

/// Testable entrypoint for `run_session_start` with injectable HerdrOps, workspace root, and cwd override.
pub fn run_session_start_with<H: crate::herdr::HerdrOps>(
    herdr: &H,
    buf: &str,
    ws_root: &Path,
    cwd_override: Option<&Path>,
) -> i32 {
    let v: serde_json::Value = serde_json::from_str(buf).unwrap_or_default();
    let sid = v.get("session_id").and_then(|x| x.as_str()).unwrap_or("");
    let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("");
    if let Some(text) = drift::session_start(ws_root, sid, source) {
        println!("{text}");
    }

    auto_tag_cos_pane_if_eligible(herdr, &v, ws_root, cwd_override);

    0
}

fn is_worker_worktree(path: &Path) -> bool {
    if path.components().any(|c| c.as_os_str() == ".wt") {
        return true;
    }
    if path.join(".git").is_file() {
        return true;
    }
    false
}

fn is_same_path(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        ca == cb
    } else {
        a == b
    }
}

fn is_eligible_workspace(cwd: &Path, ws_root: &Path) -> bool {
    if is_worker_worktree(cwd) || is_worker_worktree(ws_root) {
        return false;
    }
    if is_same_path(cwd, ws_root) && ws_root.join("projects/INDEX.md").is_file() {
        return true;
    }
    // Check if cwd is a CoS workspace outside ws_root
    cwd.join("identity.md").is_file()
        || cwd.join(".cos/identity.md").is_file()
        || cwd.join("projects/INDEX.md").is_file()
        || cwd.join(".cos/projects/INDEX.md").is_file()
}

fn auto_tag_cos_pane_if_eligible<H: crate::herdr::HerdrOps>(
    herdr: &H,
    v: &serde_json::Value,
    ws_root: &Path,
    cwd_override: Option<&Path>,
) {
    let cwd = cwd_override
        .map(PathBuf::from)
        .or_else(|| {
            v.get("cwd")
                .and_then(|c| c.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Condition 1: hook cwd/workspace root is the quaqua workspace (not a worker worktree)
    if !is_eligible_workspace(&cwd, ws_root) {
        return;
    }

    // Condition 2: HERDR_PANE (or herdr pane current) resolves
    let pane = match crate::cos::resolve_cos_pane(herdr, None) {
        Ok(p) => p,
        Err(_) => return,
    };

    // Condition 3: Pane does not already carry a grid_role token other than cos
    match crate::cos::pane_grid_role(herdr, &pane) {
        Ok(Some(role)) if !crate::cos::is_cos_pane_token(&role) => {
            // Already carries grid_role other than cos (e.g. worker, orchestrator) -> skip
            return;
        }
        Ok(_) => {
            // No grid_role or grid_role=cos -> proceed to tag
        }
        Err(e) => {
            eprintln!("quaqua: auto-tag CoS pane failed: {e}");
            return;
        }
    }

    // Tag pane as CoS (idempotent / harmless retag)
    if let Err(e) = crate::cos::tag_cos_pane_for_workspace(herdr, &pane, &cwd) {
        eprintln!("quaqua: auto-tag CoS pane failed: {e}");
    }
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
