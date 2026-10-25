//! The one workspace root.
//!
//! Every workspace path the binary resolves (rules, runs, team.toml) goes
//! through [`workspace_root`]: `QUAQUA_HOME` first (with `QUAQUA_ROOT` kept as
//! an alias for one release), else `$HOME/quaqua`. There is no discovery from
//! the cwd or the executable path. Code that needs a real workspace calls
//! [`workspace_root_checked`], which requires `projects/INDEX.md`.

use std::path::{Path, PathBuf};

/// Pure resolver, no process env — testable without `set_var`.
/// `QUAQUA_HOME` wins, else `$HOME/quaqua`, else the current directory.
pub fn resolve_workspace_root(quaqua_home: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(p) = quaqua_home {
        p.to_path_buf()
    } else if let Some(h) = home {
        h.join("quaqua")
    } else {
        PathBuf::from(".")
    }
}

/// Returns true if the process is running in a test environment.
pub fn is_test_environment() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::var("QUAQUA_TEST").as_deref() == Ok("1")
        || is_running_in_cargo_test()
}

fn is_running_in_cargo_test() -> bool {
    if std::env::var_os("CARGO_TARGET_TMPDIR").is_some() {
        return true;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.ends_with("deps") {
                return true;
            }
        }
    }
    false
}

static INITIAL_HOME: std::sync::LazyLock<Option<PathBuf>> =
    std::sync::LazyLock::new(|| std::env::var_os("HOME").map(PathBuf::from));

/// Returns the initial HOME directory captured at startup.
pub fn initial_home() -> Option<PathBuf> {
    INITIAL_HOME.clone()
}

/// Returns the live workspace DB path at `$HOME/quaqua/runs/quaqua.db` if initial HOME is set.
pub fn live_workspace_db() -> Option<PathBuf> {
    initial_home().map(|h| h.join("quaqua/runs/quaqua.db"))
}

/// Returns true if `path` points to the live workspace DB at `$HOME/quaqua/runs/quaqua.db`.
pub fn is_live_workspace_db(path: &Path) -> bool {
    // Any path inside the system temp directory is a test path, never the live workspace.
    let tmp = std::env::temp_dir();
    if path.starts_with(&tmp) {
        return false;
    }
    if let (Ok(p_canon), Ok(tmp_canon)) = (path.canonicalize(), tmp.canonicalize()) {
        if p_canon.starts_with(&tmp_canon) {
            return false;
        }
    }
    if path.starts_with("/tmp") || path.starts_with("/private/tmp") || path.starts_with("/private/var") {
        return false;
    }

    if let Some(live) = live_workspace_db() {
        if path == live {
            return true;
        }
        if let (Ok(p1), Ok(p2)) = (path.canonicalize(), live.canonicalize()) {
            if p1 == p2 {
                return true;
            }
        }
    }
    false
}

/// Returns true if `path` points to the live workspace runs directory at `$HOME/quaqua/runs`.
pub fn is_live_workspace_runs(path: &Path) -> bool {
    let db = path.join("quaqua.db");
    is_live_workspace_db(&db)
}
/// A dedicated per-process temporary runs directory under test.
pub fn test_default_runs_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("quaqua-test-runs-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A non-empty env var as a path; an empty value counts as unset.
pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// [`workspace_root`] from an already-read env snapshot: `QUAQUA_HOME`, then
/// its alias `QUAQUA_ROOT`, then `$HOME/quaqua`. Pure — testable without
/// `set_var`.
pub(crate) fn workspace_root_from_env(
    quaqua_home: Option<&Path>,
    quaqua_root: Option<&Path>,
    home: Option<&Path>,
) -> PathBuf {
    resolve_workspace_root(quaqua_home.or(quaqua_root), home)
}

/// Read the process env once: `QUAQUA_HOME`, then its alias `QUAQUA_ROOT`,
/// then `$HOME/quaqua`.
pub fn workspace_root() -> PathBuf {
    workspace_root_from_env(
        env_path("QUAQUA_HOME").as_deref(),
        env_path("QUAQUA_ROOT").as_deref(),
        env_path("HOME").as_deref(),
    )
}

/// [`workspace_root`], required to be an actual workspace: `projects/INDEX.md`
/// must exist. A missing index is an error — never a cwd fallback.
pub fn workspace_root_checked() -> Result<PathBuf, (i32, String)> {
    ensure_workspace_root(workspace_root())
}

fn ensure_workspace_root(root: PathBuf) -> Result<PathBuf, (i32, String)> {
    if root.join("projects/INDEX.md").is_file() {
        Ok(root)
    } else {
        Err((
            1,
            format!(
                "no quaqua workspace at {}: projects/INDEX.md is missing (set QUAQUA_HOME or QUAQUA_ROOT)\n",
                root.display()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quaqua_home_wins_over_home() {
        assert_eq!(
            resolve_workspace_root(Some(Path::new("target/qa")), Some(Path::new("target/user"))),
            PathBuf::from("target/qa"),
        );
    }

    #[test]
    fn default_is_home_quaqua() {
        assert_eq!(
            resolve_workspace_root(None, Some(Path::new("target/user"))),
            PathBuf::from("target/user/quaqua"),
        );
    }

    #[test]
    fn no_env_falls_back_to_cwd() {
        assert_eq!(resolve_workspace_root(None, None), PathBuf::from("."));
    }

    #[test]
    fn checked_root_requires_projects_index() {
        let dir =
            std::env::temp_dir().join(format!("quaqua-workspace-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let err = ensure_workspace_root(dir.clone()).unwrap_err();
        assert_eq!(err.0, 1);
        assert!(err.1.contains("projects/INDEX.md"), "{}", err.1);

        std::fs::create_dir_all(dir.join("projects")).unwrap();
        std::fs::write(dir.join("projects/INDEX.md"), "# Projects\n").unwrap();
        assert_eq!(ensure_workspace_root(dir.clone()).unwrap(), dir);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
