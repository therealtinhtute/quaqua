//! `quaqua doctor` — check that the `quaqua` a shell resolves is the binary
//! worth using: installed on PATH, not older than the checkout it was built
//! from, and not shadowed by a shell function/alias (issue #55 item 3).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// Interactive shells can be slow with a large rc; never let the probe (or a
/// check command) hang `quaqua doctor`.
const PROBE_TIMEOUT: Duration = Duration::from_millis(5000);

pub struct Report {
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// `quaqua doctor`: print what was found, one `error:` per problem, and exit
/// non-zero if anything is wrong.
pub fn run() -> i32 {
    let report = diagnose();
    for line in &report.notes {
        println!("{line}");
    }
    if report.problems.is_empty() {
        println!(
            "ok: quaqua is installed, not older than the checkout, and not shadowed by a shell function/alias"
        );
        0
    } else {
        for problem in &report.problems {
            eprintln!("error: {problem}");
        }
        1
    }
}

/// The real checks: PATH lookup, checkout discovery, and `$SHELL`.
pub fn diagnose() -> Report {
    let path = std::env::var_os("PATH");
    let binary = find_on_path("quaqua", path.as_deref());
    let checkout = checkout_root();
    let shell = shell_path();
    evaluate(binary.as_deref(), checkout.as_deref(), shell.as_deref())
}

/// The check core with explicit inputs, so tests can drive every branch
/// without changing PATH or the user's shell.
pub fn evaluate(binary: Option<&Path>, checkout: Option<&Path>, shell: Option<&Path>) -> Report {
    let mut report = Report {
        problems: Vec::new(),
        notes: Vec::new(),
    };
    match binary {
        Some(bin) => {
            report.notes.push(format!("binary: {}", bin.display()));
            if let Some(root) = checkout {
                report.notes.push(format!("checkout: {}", root.display()));
                if let Some(newest) = stale_against(bin, root) {
                    report.problems.push(format!(
                        "{} is older than the checkout ({}) — rebuild/install it: cargo install --path crates/quaqua --locked",
                        bin.display(),
                        newest.display()
                    ));
                }
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
/// `crates/quaqua/src`, else `~/quaqua`.
pub fn checkout_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("QUAQUA_CHECKOUT") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if let Ok(mut dir) = std::env::current_dir() {
        loop {
            if is_checkout(&dir) {
                return Some(dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let candidate = home.join("quaqua");
    is_checkout(&candidate).then_some(candidate)
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
        let report = evaluate(None, None, None);
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
        let report = evaluate(Some(&bin), None, Some(&shell));
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
        let report = evaluate(Some(&bin), Some(&checkout), None);
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
        let report = evaluate(Some(&fresh), Some(&checkout), None);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
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
}
