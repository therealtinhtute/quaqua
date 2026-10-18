//! `quaqua init` boots the per-workspace personal state a fresh clone needs
//! (`memory/`, `projects/`, `raw/`, `runs/`, `reports/`, seeded files), wires
//! `core.hooksPath`, tags the real CoS pane in herdr, and prints the install line.
//!
//! Idempotent: an existing target is never overwritten — every file is copied
//! only when missing. The repo root is resolved from the shim's
//! `QUAQUA_SCRIPT_DIR` (the shim lives at `<root>/scripts/workspace/`), else
//! from cwd, exactly as the bash script resolves it from its own location.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INSTALL_LINE: &str = "cargo install --path crates/quaqua";

const PRE_COMMIT_TEMPLATE: &str = r#"#!/usr/bin/env bash
# pre-commit — rules/_all/portable-paths.md: reject a commit whose ADDED lines
# carry a machine-specific absolute path (/home/<name>/…, /Users/<name>/…).
# The repo syncs across machines, so committed scripts, docs, settings and run
# reports must use repo-relative paths, ~/, $HOME or $(dirname "$0").
#
# Scope: only lines this commit adds. Existing tracked content is never
# re-judged — a checkout that already violates the rule still commits, which
# keeps the hook from bricking a legacy tree. A rename-with-modification adds
# its content, so a moved file is re-checked under its new name; a pure rename
# adds nothing and passes. Binary diffs carry no lines and pass.
#
# Allowlist: rules/_all/portable-paths.md (it names the pattern) and tests/**
# (fixtures must spell real machine paths). Excluded via pathspec, so the
# allowlist is exact — "tests-helper.sh" is still checked.
#
# Heuristic guardrail, like scripts/hooks/herdr-gate.sh: not a security
# boundary. Known holes — a user component outside [A-Za-z0-9._-], a path that
# only appears through a file marked `-diff` in .gitattributes (git then calls
# it binary), and commits made with --no-verify.
#
# The diff is parsed rather than diffed per file: `git diff -- <path>` re-adds
# every line when a rename is detected, which would re-judge untouched content.
# `--no-prefix` + `core.quotePath=false` make the "+++" line the literal path
# (a trailing TAB is added when the name contains a space, hence the strip).
#
# Installed by `quaqua init` via `core.hooksPath`.
# Exit 1 with each offending file:line on stderr; 0 when clean.
set -uo pipefail

path_re='/(home|Users)/[A-Za-z0-9._-]+'

diff=$(git -c core.quotePath=false diff --cached -U0 --no-prefix --no-ext-diff \
         --no-textconv --diff-filter=ACMR -- . \
         ':(exclude,literal)tests' \
         ':(exclude,literal)rules/_all/portable-paths.md' || true)
[[ -n $diff ]] || exit 0

bad=0
cur=
while IFS= read -r line; do
  case $line in
    '+++ '*)
      cur=${line#+++ }
      cur=${cur%$'\t'}
      cur=${cur% }
      [[ $cur == /dev/null ]] && cur=
      ;;
    '+'*)
      [[ -n $cur ]] || continue
      if [[ $line =~ $path_re ]]; then
        printf '%s: %s\n' "$cur" "${line#+}" >&2
        bad=1
      fi
      ;;
  esac
done <<<"$diff"

(( bad )) || exit 0
{
  echo "pre-commit: machine-specific absolute path in the added lines above."
  echo "Use a repo-relative path, ~/, \$HOME or \$(dirname \"\$0\") instead"
  echo "— rules/_all/portable-paths.md"
} >&2
exit 1
"#;

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

    wire_hooks(&root)?;
    tag_cos_pane(&root);

    println!("workspace ready in {}", root.display());
    println!("{INSTALL_LINE}");
    Ok(())
}

/// Root = the shim's `<QUAQUA_SCRIPT_DIR>/../..` (bash: `dirname($0)/../..`)
/// when the shim sets the variable, else the nearest cwd ancestor holding
/// `templates/`. The result is canonicalized (`pwd -P`), so a symlinked
/// checkout compares equal to herdr's real cwd.
fn resolve_root() -> Result<PathBuf, String> {
    if let Ok(sd) = std::env::var("QUAQUA_SCRIPT_DIR") {
        if !sd.is_empty() {
            let p = PathBuf::from(&sd);
            let cand = p
                .parent()
                .and_then(|x| x.parent())
                .map(PathBuf::from)
                .unwrap_or(p);
            return Ok(canonical(cand));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(r) = find_root_up(&cwd) {
            return Ok(canonical(r));
        }
        return Ok(canonical(cwd));
    }
    Err("init-workspace: no templates/ in repo root".to_string())
}

fn canonical(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
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

/// Wire `core.hooksPath` -> `<root>/.git/quaqua-hooks` on a real git repo, generating
/// the embedded `pre-commit` hook if missing or refreshing it if changed.
/// An existing different value (local or global) is left in place with a warning.
/// If `core.hooksPath` is legacy `<root>/scripts/hooks`, it is migrated to the new dir.
/// A failed write fails the run, like the bash script's `set -e`.
fn wire_hooks(root: &Path) -> Result<(), String> {
    if !git_ok(root, &["rev-parse", "--git-dir"]) {
        return Ok(());
    }

    let hooks_dir = if root.join(".git").is_file() {
        git_output(root, &["rev-parse", "--git-dir"])
            .map(PathBuf::from)
            .map(|p| if p.is_absolute() { p } else { root.join(p) })
            .map(|p| p.join("quaqua-hooks"))
            .unwrap_or_else(|| root.join(".git/quaqua-hooks"))
    } else {
        root.join(".git/quaqua-hooks")
    };

    std::fs::create_dir_all(&hooks_dir).map_err(|e| {
        format!("init-workspace: cannot create {}: {e}", hooks_dir.display())
    })?;

    let hook_file = hooks_dir.join("pre-commit");
    if !hook_file.exists() {
        std::fs::write(&hook_file, PRE_COMMIT_TEMPLATE).map_err(|e| {
            format!("init-workspace: cannot write {}: {e}", hook_file.display())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_file, std::fs::Permissions::from_mode(0o755)).map_err(|e| {
                format!("init-workspace: failed to chmod {}: {e}", hook_file.display())
            })?;
        }
    } else {
        let existing = std::fs::read_to_string(&hook_file).unwrap_or_default();
        if existing != PRE_COMMIT_TEMPLATE {
            std::fs::write(&hook_file, PRE_COMMIT_TEMPLATE).map_err(|e| {
                format!("init-workspace: cannot refresh {}: {e}", hook_file.display())
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&hook_file, std::fs::Permissions::from_mode(0o755)).map_err(|e| {
                    format!("init-workspace: failed to chmod {}: {e}", hook_file.display())
                })?;
            }
            println!("hooks: refreshed {}", hook_file.display());
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&hook_file, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    let want_str = hooks_dir.to_string_lossy().to_string();
    let legacy = root.join("scripts/hooks");
    let legacy_str = legacy.to_string_lossy().to_string();

    match git_output(root, &["config", "--get", "core.hooksPath"]) {
        Some(have)
            if have == want_str
                || have == format!("{want_str}/")
                || have == ".git/quaqua-hooks"
                || have == ".git/quaqua-hooks/"
                || Path::new(&have) == hooks_dir
                || root.join(&have) == hooks_dir =>
        {
            // already ours — silent, like the rest of this command
        }
        Some(have)
            if have == legacy_str
                || have == "scripts/hooks"
                || have == format!("{legacy_str}/")
                || Path::new(&have) == legacy
                || root.join(&have) == legacy =>
        {
            if !git_ok(root, &["config", "core.hooksPath", &want_str]) {
                return Err(format!(
                    "init-workspace: failed to set core.hooksPath to {want_str}"
                ));
            }
            println!("hooks: migrated core.hooksPath from '{have}' to {want_str}");
        }
        Some(have) if !have.is_empty() => {
            eprintln!(
                "init-workspace: core.hooksPath already set to '{have}' — left unchanged \
                 (unset it to use {want_str})"
            );
        }
        _ => {
            if !git_ok(root, &["config", "core.hooksPath", &want_str]) {
                return Err(format!(
                    "init-workspace: failed to set core.hooksPath to {want_str}"
                ));
            }
            println!("hooks: core.hooksPath={want_str}");
        }
    }
    Ok(())
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
