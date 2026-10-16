//! `quaqua init` — the port of `scripts/workspace/init-workspace.sh`. It boots
//! the per-workspace personal state a fresh clone needs (`memory/`, `projects/`,
//! `raw/`, `runs/`, `reports/`, seeded files), wires `core.hooksPath`, tags the
//! real CoS pane in herdr, and prints the install line.
//!
//! Idempotent: an existing target is never overwritten — every file is copied
//! only when missing. The repo root is resolved from the shim's
//! `QUAQUA_SCRIPT_DIR` (the shim lives at `<root>/scripts/workspace/`), else
//! from cwd, exactly as the bash script resolves it from its own location.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INSTALL_LINE: &str = "cargo install --path crates/quaqua";

pub fn run() -> Result<(), String> {
    let root = resolve_root()?;
    if !root.join("templates").is_dir() {
        return Err(format!("init-workspace: no templates/ in {}", root.display()));
    }

    // gitignored personal-state dirs (AGENTS.md '## Memory')
    for d in ["memory", "projects", "raw", "runs", "reports"] {
        let p = root.join(d);
        if p.is_dir() {
            continue;
        }
        std::fs::create_dir_all(&p).map_err(|e| format!("init-workspace: cannot create {d}/: {e}"))?;
        println!("created {d}/");
    }

    // seed <repo-relative-source> <repo-relative-target>: copy only when the
    // target is missing, never clobber an existing local file. The source is
    // checked first, so a missing template fails loudly even on a populated
    // workspace.
    for (src, dst) in [
        ("templates/memory-rules.md", "memory/rules.md"),
        ("templates/projects-INDEX.md", "projects/INDEX.md"),
        ("templates/projects-_template.md", "projects/_template.md"),
        ("USER.example.md", "USER.md"),
    ] {
        seed(&root, src, dst)?;
    }

    wire_hooks(&root);
    tag_cos_pane(&root);

    println!("workspace ready in {}", root.display());
    println!("{INSTALL_LINE}");
    Ok(())
}

/// Root = the nearest ancestor of `QUAQUA_SCRIPT_DIR` (shim) or cwd holding
/// `templates/`. In the shim layout the script sits at `<root>/scripts/workspace`,
/// so this equals the bash `dirname($0)/../..`.
fn resolve_root() -> Result<PathBuf, String> {
    if let Ok(sd) = std::env::var("QUAQUA_SCRIPT_DIR") {
        if let Some(r) = find_root_up(&PathBuf::from(sd)) {
            return Ok(r);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(r) = find_root_up(&cwd) {
            return Ok(r);
        }
    }
    Err("init-workspace: no templates/ in repo root".to_string())
}

fn find_root_up(start: &Path) -> Option<PathBuf> {
    for anc in start.ancestors() {
        if anc.join("templates").is_dir() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

fn seed(root: &Path, src_rel: &str, dst_rel: &str) -> Result<(), String> {
    let src = root.join(src_rel);
    let dst = root.join(dst_rel);
    if !src.is_file() {
        return Err(format!("init-workspace: missing template: {src_rel}"));
    }
    if dst.exists() {
        return Ok(());
    }
    std::fs::copy(&src, &dst).map_err(|e| format!("init-workspace: cannot create {dst_rel}: {e}"))?;
    println!("created {dst_rel}");
    Ok(())
}

/// Wire `core.hooksPath` -> `<root>/scripts/hooks` on a real git repo, only when
/// nothing else owns it. An existing different value (local or global) is left
/// in place with a warning.
fn wire_hooks(root: &Path) {
    if !git_ok(root, &["rev-parse", "--git-dir"]) {
        return;
    }
    let want = root.join("scripts/hooks");
    let want_str = want.to_string_lossy().to_string();
    match git_output(root, &["config", "--get", "core.hooksPath"]) {
        Some(have) if have == want_str => {
            // already ours — silent, like the rest of this command
        }
        Some(have) if !have.is_empty() => {
            eprintln!(
                "init-workspace: core.hooksPath already set to '{have}' — left unchanged \
                 (unset it to use {want_str})"
            );
        }
        _ => {
            if git_ok(root, &["config", "core.hooksPath", &want_str]) {
                println!("hooks: core.hooksPath={want_str}");
            }
        }
    }
}

fn git_ok(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// herdr-grid: tag the CoS orchestrator pane (R5) — but ONLY that pane. The
/// caller's pane is checked against herdr, not assumed: the CoS is the unnamed
/// Claude agent in this checkout. Anything else (another harness, a named
/// agent, another directory, a herdr that cannot answer) fails closed and is
/// left untagged.
fn tag_cos_pane(root: &Path) {
    if !herdr_available() {
        return;
    }
    let cos_pane = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            herdr_json(&["pane", "current"]).and_then(|v| {
                v.pointer("/result/pane/pane_id")
                    .and_then(|p| p.as_str())
                    .map(String::from)
            })
        });
    let Some(pane) = cos_pane else {
        return;
    };
    if !is_cos_pane(&pane, root) {
        return;
    }
    let _ = Command::new("herdr")
        .args([
            "pane",
            "report-metadata",
            &pane,
            "--source",
            "herdr-grid",
            "--token",
            "grid_role=orchestrator",
            "--token",
            "grid_owner=cos",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// is_cos_pane <pane>: true only for the CoS pane itself — a `claude` agent, in
/// this checkout root, not registered in `herdr agent list`.
fn is_cos_pane(pane: &str, root: &Path) -> bool {
    let info = match herdr_json(&["pane", "get", pane]) {
        Some(v) => v,
        None => return false,
    };
    let agent = info
        .pointer("/result/pane/agent")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cwd = info
        .pointer("/result/pane/cwd")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if agent != "claude" || cwd.is_empty() {
        return false;
    }
    let real = std::fs::canonicalize(cwd).ok();
    if real.as_deref() != Some(root) {
        return false;
    }
    let list = match herdr_json(&["agent", "list"]) {
        Some(v) => v,
        None => return false,
    };
    let name = list
        .get("result")
        .and_then(|r| r.get("agents"))
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|e| e.get("pane_id").and_then(|p| p.as_str()) == Some(pane))
        })
        .and_then(|e| e.get("name").and_then(|n| n.as_str()))
        .unwrap_or("");
    name.is_empty()
}

fn herdr_json(args: &[&str]) -> Option<serde_json::Value> {
    let out = Command::new("herdr")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// `command -v herdr`, without invoking it (no side effects for a stubbed test
/// `herdr` on PATH).
fn herdr_available() -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if !dir.is_empty() && Path::new(dir).join("herdr").is_file() {
                return true;
            }
        }
    }
    false
}
