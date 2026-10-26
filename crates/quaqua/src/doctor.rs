//! `quaqua doctor` — check that the `quaqua` a shell resolves is the binary
//! worth using: installed on PATH, not older than the checkout it was built
//! from (when one can be found), and not shadowed by a shell function/alias
//! (issue #55 item 3).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// Interactive shells can be slow with a large rc; never let the probe (or a
/// check command) hang `quaqua doctor`.
const PROBE_TIMEOUT: Duration = Duration::from_millis(5000);

pub struct Report {
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    /// Whether a binary was compared against a checkout: `run()` may only claim
    /// the staleness check when this is true.
    pub checkout_checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftProblem {
    pub drift: String,
    pub fix: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceDriftReport {
    pub problems: Vec<DriftProblem>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

/// `quaqua doctor`: print what was found, one `error:` per problem, and exit
/// non-zero if anything is wrong.
pub fn run() -> i32 {
    let report = diagnose();
    for line in &report.notes {
        println!("{line}");
    }
    for line in &report.warnings {
        eprintln!("warning: {line}");
    }
    if report.problems.is_empty() {
        let ok = if report.checkout_checked {
            "ok: quaqua is installed, not older than the checkout, and not shadowed by a shell function/alias"
        } else {
            "ok: quaqua is installed and not shadowed by a shell function/alias — the staleness check was skipped (no checkout found)"
        };
        if report.warnings.is_empty() {
            println!("{ok}");
        } else {
            println!("{ok} (layout warnings above)");
        }
        0
    } else {
        for problem in &report.problems {
            eprintln!("error: {problem}");
        }
        1
    }
}

/// `quaqua doctor --workspace`: check workspace rules and herdr plugin event hooks
/// for drift against engine templates and manifest.
pub fn run_workspace(
    workspace: Option<&Path>,
    checkout: Option<&Path>,
    plugins_json: Option<&Path>,
) -> i32 {
    let report = evaluate_workspace(workspace, checkout, plugins_json);
    for line in &report.notes {
        println!("{line}");
    }
    for line in &report.warnings {
        eprintln!("warning: {line}");
    }
    if report.problems.is_empty() {
        println!(
            "ok: workspace rules and herdr plugin event hooks are in sync with engine templates"
        );
        0
    } else {
        for problem in &report.problems {
            eprintln!("error: {}", problem.drift);
            eprintln!("  fix: {}", problem.fix);
        }
        1
    }
}

/// The real checks: PATH lookup, checkout discovery, and `$SHELL`.
pub fn diagnose() -> Report {
    let path = std::env::var_os("PATH");
    let binary = find_on_path("quaqua", path.as_deref());
    let checkout = checkout_root();
    let workspace = crate::workspace::workspace_root();
    let shell = shell_path();
    evaluate(
        binary.as_deref(),
        checkout.as_deref(),
        Some(&workspace),
        shell.as_deref(),
    )
}

/// The check core with explicit inputs, so tests can drive every branch
/// without changing PATH or the user's shell.
pub fn evaluate(
    binary: Option<&Path>,
    checkout: Option<&Path>,
    workspace: Option<&Path>,
    shell: Option<&Path>,
) -> Report {
    let mut report = Report {
        problems: Vec::new(),
        warnings: workspace_warnings(workspace),
        notes: Vec::new(),
        checkout_checked: false,
    };
    match binary {
        Some(bin) => {
            report.notes.push(format!("binary: {}", bin.display()));
            if let Some(root) = checkout {
                report.notes.push(format!("checkout: {}", root.display()));
                report.checkout_checked = true;
                if let Some(newest) = stale_against(bin, root) {
                    report.problems.push(format!(
                        "{} is older than the checkout ({}) — rebuild/install it: cargo install --path crates/quaqua --locked",
                        bin.display(),
                        newest.display()
                    ));
                }
            } else {
                report.notes.push(
                    "checkout: not found — the staleness check was skipped (set QUAQUA_CHECKOUT=<engine checkout> or run inside one)"
                        .to_string(),
                );
            }
        }
        None => report.problems.push(
            "quaqua is not on PATH — install it (cargo install --path crates/quaqua --locked) or add its bin directory to PATH"
                .to_string(),
        ),
    }
    if let Some(shell) = shell {
        report.notes.push(format!("shell: {}", shell.display()));
        match shadow_probe(shell) {
            Ok(output) => {
                if let Some(line) = is_shadowed(&output) {
                    report.problems.push(format!(
                        "the shell resolves `quaqua` to a function/alias, not the binary: {line} — remove it from your shell rc"
                    ));
                }
            }
            Err(e) => report.notes.push(format!("shell probe skipped: {e}")),
        }
    }
    report
}

/// The first executable `name` on `path` (a `None` PATH finds nothing).
pub fn find_on_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    for dir in std::env::split_paths(path?) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `QUAQUA_CHECKOUT`, else the current directory or an ancestor containing
/// `crates/quaqua/src`, else nothing. There is no `$HOME` guess: the workspace
/// root is not the engine checkout (`workspace_warnings` reports the overlap
/// instead).
pub fn checkout_root() -> Option<PathBuf> {
    let explicit = std::env::var_os("QUAQUA_CHECKOUT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let cwd = std::env::current_dir().ok();
    let workspace = crate::workspace::workspace_root();
    checkout_root_from(explicit.as_deref(), cwd.as_deref(), Some(&workspace))
}

/// Layout warnings for the workspace `quaqua doctor` runs against: a workspace
/// must not also be an engine checkout, and its root `AGENTS.md` must import
/// the persona. Both are warnings — the run still exits 0.
pub fn workspace_warnings(workspace: Option<&Path>) -> Vec<String> {
    let Some(root) = workspace else {
        return Vec::new();
    };
    if !root.is_dir() {
        return Vec::new();
    }
    let mut warnings = Vec::new();
    let mut engine = Vec::new();
    if root.join("Cargo.toml").is_file() {
        engine.push("Cargo.toml");
    }
    if root.join("crates").is_dir() {
        engine.push("crates/");
    }
    if !engine.is_empty() {
        warnings.push(format!(
            "{} is also an engine checkout ({}) — keep the workspace and the engine checkout apart, or the engine AGENTS.md replaces the workspace one",
            root.display(),
            engine.join(", ")
        ));
    }
    let agents = root.join("AGENTS.md");
    if let Ok(text) = std::fs::read_to_string(&agents) {
        if !text.lines().any(|line| line.trim() == "@identity.md") {
            warnings.push(format!(
                "{} has no `@identity.md` import — the workspace persona will not load",
                agents.display()
            ));
        }
    }
    warnings
}

/// [`checkout_root`] with its process inputs passed in, so tests can prove a
/// workspace root is never taken for the engine checkout.
fn checkout_root_from(
    explicit: Option<&Path>,
    cwd: Option<&Path>,
    workspace: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        return Some(explicit.to_path_buf());
    }
    let mut dir = cwd?.to_path_buf();
    loop {
        if is_checkout(&dir) {
            // A workspace holding the engine sources is a broken layout, not a
            // checkout to compare a binary against.
            return (Some(dir.as_path()) != workspace).then_some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn is_checkout(path: &Path) -> bool {
    path.join("crates/quaqua/src").is_dir()
}

fn shell_path() -> Option<PathBuf> {
    std::env::var_os("QUAQUA_SHELL")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("SHELL").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

/// Ask the user's interactive shell how it resolves `quaqua`. `-i` loads the
/// rc files where functions and aliases live; `type` is a builtin in bash,
/// zsh, sh and fish.
pub fn shadow_probe(shell: &Path) -> Result<String, String> {
    let mut cmd = Command::new(shell);
    cmd.args(["-ic", "type quaqua 2>&1 || true"]);
    cmd.stdin(Stdio::null());
    let out = run_with_timeout(cmd, PROBE_TIMEOUT)?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The `type` output names a shell function or alias. A binary path does not
/// (and a function's body printout starts with `quaqua ()`).
pub fn is_shadowed(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| {
            let l = line.to_lowercase();
            (l.contains("quaqua")
                && (l.contains("function") || l.contains("alias") || l.contains("aliased")))
                || l.starts_with("quaqua ()")
        })
        .map(str::to_string)
}

/// The checkout file newer than the binary, if any: an installed binary built
/// before the source it claims to run is stale.
pub fn stale_against(binary: &Path, checkout: &Path) -> Option<PathBuf> {
    let newest = newest_source(&checkout.join("crates/quaqua/src"))?;
    let bin_time = std::fs::metadata(binary).ok()?.modified().ok()?;
    let src_time = std::fs::metadata(&newest).ok()?.modified().ok()?;
    (bin_time < src_time).then_some(newest)
}

fn newest_source(dir: &Path) -> Option<PathBuf> {
    newest_file(dir).map(|(_, path)| path)
}

fn newest_file(dir: &Path) -> Option<(SystemTime, PathBuf)> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let found = if meta.is_dir() {
            newest_file(&path)
        } else {
            meta.modified().ok().map(|t| (t, path))
        };
        if let Some((time, candidate)) = found {
            if best.as_ref().map(|(t, _)| time > *t).unwrap_or(true) {
                best = Some((time, candidate));
            }
        }
    }
    best
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", cmd.get_program().to_string_lossy()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} timed out after {}ms",
                        cmd.get_program().to_string_lossy(),
                        timeout.as_millis()
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("waiting for the shell failed: {e}")),
        }
    }
    child
        .wait_with_output()
        .map_err(|e| format!("reading shell output failed: {e}"))
}

/// Compare workspace rule files under `workspace_rules_dir` against template files
/// under `template_dir`. Returns a `DriftProblem` for each missing or differing rule.
pub fn diff_rules(template_dir: &Path, workspace_rules_dir: &Path) -> Vec<DriftProblem> {
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(template_dir) else {
        return problems;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();

    for t_path in files {
        let Some(file_name) = t_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let ws_path = workspace_rules_dir.join(file_name);
        let t_content = match std::fs::read_to_string(&t_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !ws_path.is_file() {
            problems.push(DriftProblem {
                drift: format!(
                    "rule drift: rules/_all/{} is missing (template exists under {})",
                    file_name,
                    t_path.display()
                ),
                fix: format!("cp {} {}", t_path.display(), ws_path.display()),
            });
        } else {
            match std::fs::read_to_string(&ws_path) {
                Ok(ws_content) => {
                    if ws_content != t_content {
                        problems.push(DriftProblem {
                            drift: format!(
                                "rule drift: rules/_all/{} differs from template {}",
                                file_name,
                                t_path.display()
                            ),
                            fix: format!("cp {} {}", t_path.display(), ws_path.display()),
                        });
                    }
                }
                Err(e) => {
                    problems.push(DriftProblem {
                        drift: format!("rule drift: rules/_all/{} cannot be read: {e}", file_name),
                        fix: format!("cp {} {}", t_path.display(), ws_path.display()),
                    });
                }
            }
        }
    }
    problems
}

/// Fallback when no checkout is found: compare against the embedded template table.
pub fn diff_embedded_rules(workspace_rules_dir: &Path) -> Vec<DriftProblem> {
    let mut problems = Vec::new();
    for entry in crate::init::ENTRIES {
        if let Some(rel) = entry.path.strip_prefix("rules/_all/") {
            let ws_file = workspace_rules_dir.join(rel);
            let t_path_disp = format!("templates/workspace/rules/_all/{rel}");
            if !ws_file.is_file() {
                problems.push(DriftProblem {
                    drift: format!("rule drift: rules/_all/{rel} is missing (template exists under {t_path_disp})"),
                    fix: format!("cp {t_path_disp} {}", ws_file.display()),
                });
            } else {
                match std::fs::read(&ws_file) {
                    Ok(bytes) => {
                        if bytes != entry.bytes {
                            problems.push(DriftProblem {
                                drift: format!("rule drift: rules/_all/{rel} differs from template {t_path_disp}"),
                                fix: format!("cp {t_path_disp} {}", ws_file.display()),
                            });
                        }
                    }
                    Err(e) => {
                        problems.push(DriftProblem {
                            drift: format!("rule drift: rules/_all/{rel} cannot be read: {e}"),
                            fix: format!("cp {t_path_disp} {}", ws_file.display()),
                        });
                    }
                }
            }
        }
    }
    problems
}

/// Pure diff function: compare event hooks declared in `manifest_toml` against
/// registrations in `plugins_json`.
pub fn diff_plugin_hooks(
    manifest_toml: &str,
    manifest_path: &Path,
    plugins_json: &str,
) -> Result<Vec<DriftProblem>, String> {
    let mut problems = Vec::new();
    let manifest_val: toml::Value =
        toml::from_str(manifest_toml).map_err(|e| format!("cannot parse manifest TOML: {e}"))?;

    let plugin_id = manifest_val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("herdr-quaqua");

    let mut declared_events: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(events_arr) = manifest_val.get("events").and_then(|v| v.as_array()) {
        for ev in events_arr {
            let on = ev.get("on").and_then(|v| v.as_str()).unwrap_or("");
            let cmd: Vec<String> = ev
                .get("command")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !on.is_empty() && !cmd.is_empty() {
                declared_events.push((on.to_string(), cmd));
            }
        }
    }

    let plugin_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let fix_cmd = format!("herdr plugin link {}", plugin_dir.display());

    let parsed_json: serde_json::Value = match serde_json::from_str(plugins_json) {
        Ok(v) => v,
        Err(e) => {
            problems.push(DriftProblem {
                drift: format!("plugin registration drift: plugins.json cannot be parsed ({e})"),
                fix: fix_cmd,
            });
            return Ok(problems);
        }
    };

    let entries = match parsed_json.as_array() {
        Some(arr) => arr,
        None => {
            problems.push(DriftProblem {
                drift: "plugin registration drift: plugins.json is not an array".to_string(),
                fix: fix_cmd,
            });
            return Ok(problems);
        }
    };

    let matching_plugin = entries.iter().find(|entry| {
        entry.get("plugin_id").and_then(|v| v.as_str()) == Some(plugin_id)
            || entry.get("name").and_then(|v| v.as_str()) == Some(plugin_id)
    });

    let Some(plugin) = matching_plugin else {
        problems.push(DriftProblem {
            drift: format!(
                "plugin hook drift: herdr plugin '{plugin_id}' is not registered in plugins.json"
            ),
            fix: fix_cmd,
        });
        return Ok(problems);
    };

    let registered_events = plugin
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut missing_hooks = Vec::new();
    for (on, cmd) in &declared_events {
        let found = registered_events.iter().any(|ev| {
            let ev_on = ev.get("on").and_then(|v| v.as_str()).unwrap_or("");
            let ev_cmd: Vec<String> = ev
                .get("command")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            ev_on == on && ev_cmd == *cmd
        });
        if !found {
            missing_hooks.push(format!("{on} -> {cmd:?}"));
        }
    }

    if !missing_hooks.is_empty() {
        problems.push(DriftProblem {
            drift: format!(
                "plugin hook drift: herdr plugin '{plugin_id}' lacks event hook(s) declared in {}: {}",
                manifest_path.display(),
                missing_hooks.join(", ")
            ),
            fix: fix_cmd,
        });
    }

    Ok(problems)
}

pub fn resolve_engine_from_workspace(ws: &Path) -> Option<PathBuf> {
    let index = ws.join("projects/INDEX.md");
    if let Ok(content) = std::fs::read_to_string(&index) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('|') {
                let parts: Vec<&str> = trimmed.split('|').map(|s| s.trim()).collect();
                if parts.len() >= 3 && parts[1] != "slug" && !parts[1].starts_with('-') {
                    let r = parts[2];
                    let repo_str = if let Some(stripped) = r.strip_prefix("~/") {
                        format!("{}/{}", home, stripped)
                    } else if r == "~" {
                        home.clone()
                    } else {
                        r.to_string()
                    };
                    let p = PathBuf::from(repo_str);
                    if p.join("crates/quaqua/src").is_dir() {
                        return Some(p);
                    }
                }
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let default_engine = PathBuf::from(home).join("Lab/quaqua");
        if default_engine.join("crates/quaqua/src").is_dir() {
            return Some(default_engine);
        }
    }
    None
}

pub fn resolve_herdr_plugins_json() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("HERDR_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("plugins.json"));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("herdr/plugins.json"));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home).join(".config/herdr/plugins.json"));
    }
    None
}

pub fn evaluate_workspace(
    workspace: Option<&Path>,
    checkout: Option<&Path>,
    plugins_json_path: Option<&Path>,
) -> WorkspaceDriftReport {
    let mut report = WorkspaceDriftReport::default();
    let ws = workspace
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::workspace::workspace_root());

    report.notes.push(format!("workspace: {}", ws.display()));

    let checkout_dir = checkout
        .map(PathBuf::from)
        .or_else(checkout_root)
        .or_else(|| resolve_engine_from_workspace(&ws));

    // 1. Rules drift
    if let Some(co) = &checkout_dir {
        report.notes.push(format!("checkout: {}", co.display()));
        let t_rules = co.join("templates/workspace/rules/_all");
        let ws_rules = ws.join("rules/_all");
        let rule_problems = diff_rules(&t_rules, &ws_rules);
        report.problems.extend(rule_problems);
    } else {
        let ws_rules = ws.join("rules/_all");
        let rule_problems = diff_embedded_rules(&ws_rules);
        report.problems.extend(rule_problems);
    }

    // 2. Herdr plugin hook drift
    let manifest_path = checkout_dir
        .as_ref()
        .map(|c| c.join("plugins/herdr-quaqua/herdr-plugin.toml"))
        .unwrap_or_else(|| PathBuf::from("plugins/herdr-quaqua/herdr-plugin.toml"));

    let manifest_toml = if manifest_path.is_file() {
        std::fs::read_to_string(&manifest_path).ok()
    } else {
        Some(include_str!("../../../plugins/herdr-quaqua/herdr-plugin.toml").to_string())
    };

    let pjson_path = plugins_json_path
        .map(PathBuf::from)
        .or_else(resolve_herdr_plugins_json);

    match (manifest_toml, pjson_path) {
        (Some(toml_str), Some(pj_path)) => {
            if pj_path.is_file() {
                match std::fs::read_to_string(&pj_path) {
                    Ok(json_str) => match diff_plugin_hooks(&toml_str, &manifest_path, &json_str) {
                        Ok(probs) => report.problems.extend(probs),
                        Err(e) => report
                            .warnings
                            .push(format!("plugin hook check error: {e}")),
                    },
                    Err(e) => {
                        let plugin_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
                        report.problems.push(DriftProblem {
                            drift: format!(
                                "plugin hook drift: herdr plugins.json at {} cannot be read: {e}",
                                pj_path.display()
                            ),
                            fix: format!("herdr plugin link {}", plugin_dir.display()),
                        });
                    }
                }
            } else {
                let plugin_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
                report.problems.push(DriftProblem {
                    drift: format!(
                        "plugin hook drift: herdr plugins.json not found at {}",
                        pj_path.display()
                    ),
                    fix: format!("herdr plugin link {}", plugin_dir.display()),
                });
            }
        }
        (None, _) => {
            report
                .warnings
                .push("cannot find herdr-plugin.toml manifest".to_string());
        }
        (_, None) => {
            report
                .warnings
                .push("cannot resolve herdr plugins.json path".to_string());
        }
    }

    // 3. Workspace warnings
    report.warnings.extend(workspace_warnings(Some(&ws)));

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quaqua-doctor-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Deterministic mtime (filesystem timestamp granularity differs).
    fn set_mtime(path: &Path, secs: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn stub_shell(dir: &Path, line: &str) -> PathBuf {
        let path = dir.join("shell");
        fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{line}'\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn missing_binary_is_a_problem() {
        let report = evaluate(None, None, None, None);
        assert_eq!(report.problems.len(), 1);
        assert!(
            report.problems[0].contains("not on PATH"),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn a_shell_function_or_alias_shadowing_quaqua_is_a_problem() {
        assert!(is_shadowed("quaqua is a function\nquaqua () { ... }").is_some());
        assert!(is_shadowed("quaqua is aliased to `~/bin/quaqua-old'").is_some());
        assert!(is_shadowed("quaqua: function").is_some());
        assert!(is_shadowed("quaqua is /usr/local/bin/quaqua").is_none());
        assert!(is_shadowed("").is_none());

        let dir = temp_dir("shadow");
        let shell = stub_shell(&dir, "quaqua is a shell function");
        let bin = dir.join("quaqua");
        fs::write(&bin, "").unwrap();
        let report = evaluate(Some(&bin), None, None, Some(&shell));
        assert!(
            report.problems.iter().any(|p| p.contains("function/alias")),
            "{:?}",
            report.problems
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_binary_older_than_the_checkout_is_stale() {
        let dir = temp_dir("stale");
        let checkout = dir.join("checkout");
        let src = checkout.join("crates/quaqua/src");
        fs::create_dir_all(&src).unwrap();
        let bin = dir.join("quaqua");
        fs::write(&bin, "").unwrap();
        // The source is newer than the binary: the binary is stale.
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        set_mtime(&bin, 1_000);
        set_mtime(&src.join("main.rs"), 2_000);
        let report = evaluate(Some(&bin), Some(&checkout), None, None);
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("older than the checkout")),
            "{:?}",
            report.problems
        );

        // A fresh binary (newer than the source) is accepted.
        let fresh = dir.join("quaqua-fresh");
        fs::write(&fresh, "").unwrap();
        set_mtime(&fresh, 3_000);
        let report = evaluate(Some(&fresh), Some(&checkout), None, None);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A workspace dir with a project index and a persona-importing AGENTS.md:
    /// the layout `quaqua init` is supposed to produce.
    fn workspace_dir(dir: &Path) -> PathBuf {
        let ws = dir.join("workspace");
        fs::create_dir_all(ws.join("projects")).unwrap();
        fs::write(ws.join("projects/INDEX.md"), "").unwrap();
        fs::write(
            ws.join("AGENTS.md"),
            "# CoS workspace\n\n@identity.md\n@SOUL.md\n",
        )
        .unwrap();
        ws
    }

    #[test]
    fn a_workspace_that_also_holds_the_engine_checkout_is_a_warning_not_a_problem() {
        let dir = temp_dir("overlap");
        let ws = workspace_dir(&dir);

        // A clean workspace is quiet (only the missing binary is a problem).
        let report = evaluate(None, None, Some(&ws), None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);

        // `Cargo.toml` at the root: the workspace is also an engine checkout.
        fs::write(ws.join("Cargo.toml"), "[workspace]\n").unwrap();
        let report = evaluate(None, None, Some(&ws), None);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0].contains("Cargo.toml"),
            "{:?}",
            report.warnings
        );
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);

        // `crates/` alone is enough of a signal.
        let other = dir.join("other");
        fs::create_dir_all(other.join("crates/quaqua/src")).unwrap();
        let report = evaluate(None, None, Some(&other), None);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0].contains("crates/"),
            "{:?}",
            report.warnings
        );

        // Neither a missing workspace dir nor no workspace at all warns.
        let report = evaluate(None, None, Some(&dir.join("absent")), None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let report = evaluate(None, None, None, None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_workspace_agents_md_without_the_persona_import_is_a_warning() {
        let dir = temp_dir("agents");
        let ws = workspace_dir(&dir);
        let agents = ws.join("AGENTS.md");

        // Importing the persona (the `quaqua init` layout) is quiet.
        let report = evaluate(None, None, Some(&ws), None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        // No AGENTS.md at all: nothing claims to import anything.
        fs::remove_file(&agents).unwrap();
        let report = evaluate(None, None, Some(&ws), None);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        // The engine AGENTS.md the workspace root used to carry: no persona.
        fs::write(
            &agents,
            "# quaqua engine\n\nEngine-only: crates/, tests/.\n",
        )
        .unwrap();
        let report = evaluate(None, None, Some(&ws), None);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0].contains("@identity.md"),
            "{:?}",
            report.warnings
        );
        // A warning never fails the run.
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkout_is_the_engine_sources_never_the_workspace_root() {
        let dir = temp_dir("checkout");
        let engine = dir.join("Lab/quaqua");
        fs::create_dir_all(engine.join("crates/quaqua/src")).unwrap();
        let ws = dir.join("quaqua");
        fs::create_dir_all(ws.join("projects")).unwrap();

        // Running inside the engine checkout discovers it from the cwd.
        assert_eq!(
            checkout_root_from(None, Some(&engine.join("crates/quaqua")), Some(&ws)),
            Some(engine.clone())
        );
        // An explicit QUAQUA_CHECKOUT wins.
        assert_eq!(
            checkout_root_from(Some(&engine), None, Some(&ws)),
            Some(engine.clone())
        );
        // The workspace root is not the checkout, even when engine sources sit
        // there (`~/quaqua` before the split): that layout is a warning instead.
        let overlapping = dir.join("overlap");
        fs::create_dir_all(overlapping.join("crates/quaqua/src")).unwrap();
        assert_eq!(
            checkout_root_from(None, Some(&overlapping), Some(&overlapping)),
            None
        );
        // Outside any checkout nothing is assumed.
        assert_eq!(checkout_root_from(None, Some(&ws), Some(&ws)), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_on_path_skips_missing_entries() {
        let dir = temp_dir("path");
        let bin = dir.join("quaqua");
        fs::write(&bin, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let missing = format!("/nope/{}", dir.display());
        let path = OsStr::new(&missing);
        assert!(find_on_path("quaqua", Some(path)).is_none());
        let path = dir.as_os_str();
        assert_eq!(find_on_path("quaqua", Some(path)), Some(bin));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_rules_detects_content_difference_and_missing_rules() {
        let dir = temp_dir("diff-rules");
        let template_dir = dir.join("templates/rules");
        let ws_rules_dir = dir.join("ws/rules");
        fs::create_dir_all(&template_dir).unwrap();
        fs::create_dir_all(&ws_rules_dir).unwrap();

        fs::write(
            template_dir.join("lead-inbox.md"),
            "template lead inbox content",
        )
        .unwrap();
        fs::write(template_dir.join("brief.md"), "common brief content").unwrap();
        fs::write(template_dir.join("missing.md"), "template only content").unwrap();

        // ws has differing lead-inbox, identical brief, and missing.md is absent
        fs::write(
            ws_rules_dir.join("lead-inbox.md"),
            "stale workspace lead inbox",
        )
        .unwrap();
        fs::write(ws_rules_dir.join("brief.md"), "common brief content").unwrap();

        let problems = diff_rules(&template_dir, &ws_rules_dir);
        assert_eq!(problems.len(), 2, "{problems:?}");

        let diff_lead = problems
            .iter()
            .find(|p| p.drift.contains("lead-inbox.md"))
            .unwrap();
        assert!(diff_lead.drift.contains("differs from template"));
        assert!(diff_lead.fix.starts_with("cp "));

        let missing = problems
            .iter()
            .find(|p| p.drift.contains("missing.md"))
            .unwrap();
        assert!(missing.drift.contains("is missing"));
        assert!(missing.fix.starts_with("cp "));

        // When workspace is synced: 0 problems
        fs::write(
            ws_rules_dir.join("lead-inbox.md"),
            "template lead inbox content",
        )
        .unwrap();
        fs::write(ws_rules_dir.join("missing.md"), "template only content").unwrap();
        let clean = diff_rules(&template_dir, &ws_rules_dir);
        assert!(clean.is_empty(), "{clean:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_plugin_hooks_detects_missing_event_hooks() {
        let dir = temp_dir("diff-hooks");
        let manifest_path = dir.join("herdr-quaqua/herdr-plugin.toml");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        let manifest_toml = r#"
id = "herdr-quaqua"
name = "herdr-quaqua"

[[events]]
on = "pane.created"
command = ["quaqua", "grid", "hook"]

[[events]]
on = "pane.agent_status_changed"
command = ["quaqua", "events", "hook"]

[[events]]
on = "pane.exited"
command = ["quaqua", "events", "hook"]
"#;

        // plugins.json has only pane.created
        let partial_json = r#"[
  {
    "plugin_id": "herdr-quaqua",
    "name": "herdr-quaqua",
    "events": [
      {
        "on": "pane.created",
        "command": ["quaqua", "grid", "hook"]
      }
    ]
  }
]"#;

        let problems = diff_plugin_hooks(manifest_toml, &manifest_path, partial_json).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].drift.contains("lacks event hook(s)"));
        assert!(problems[0].drift.contains("pane.agent_status_changed"));
        assert!(problems[0].drift.contains("pane.exited"));
        assert!(problems[0].fix.contains("herdr plugin link"));

        // When plugins.json has all hooks: 0 problems
        let full_json = r#"[
  {
    "plugin_id": "herdr-quaqua",
    "name": "herdr-quaqua",
    "events": [
      {
        "on": "pane.created",
        "command": ["quaqua", "grid", "hook"]
      },
      {
        "on": "pane.agent_status_changed",
        "command": ["quaqua", "events", "hook"]
      },
      {
        "on": "pane.exited",
        "command": ["quaqua", "events", "hook"]
      }
    ]
  }
]"#;
        let clean = diff_plugin_hooks(manifest_toml, &manifest_path, full_json).unwrap();
        assert!(clean.is_empty(), "{clean:?}");

        let _ = fs::remove_dir_all(&dir);
    }
}
