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

/// A non-empty env var as a path; an empty value counts as unset.
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Read the process env once: `QUAQUA_HOME`, then its alias `QUAQUA_ROOT`,
/// then `$HOME/quaqua`.
pub fn workspace_root() -> PathBuf {
    let quaqua_home = env_path("QUAQUA_HOME").or_else(|| env_path("QUAQUA_ROOT"));
    let home = env_path("HOME");
    resolve_workspace_root(quaqua_home.as_deref(), home.as_deref())
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
