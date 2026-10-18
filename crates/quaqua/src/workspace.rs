//! The one workspace root.
//!
//! Every workspace path the binary resolves (rules, runs, team.toml) goes
//! through [`workspace_root`]: `QUAQUA_HOME` first, else `$HOME/quaqua`.
//!
/// [`find_quaqua_root`] discovers the checkout that owns `projects/INDEX.md`
/// (via `QUAQUA_ROOT`, `QUAQUA_SCRIPT_DIR`, cwd ancestor walk, exe ancestor walk).
/// In contrast, [`workspace_root`] resolves the installed workspace directory.

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

/// Read the process env once. An empty `QUAQUA_HOME`/`HOME` counts as unset.
pub fn workspace_root() -> PathBuf {
    let quaqua_home = std::env::var_os("QUAQUA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    resolve_workspace_root(quaqua_home.as_deref(), home.as_deref())
}

/// Discover the quaqua repository checkout root containing `projects/INDEX.md`.
/// Checks `QUAQUA_ROOT`, `QUAQUA_SCRIPT_DIR`, current directory ancestors, and current exe ancestors.
pub fn find_quaqua_root() -> Result<PathBuf, (i32, String)> {
    if let Ok(r) = std::env::var("QUAQUA_ROOT") {
        if !r.is_empty() {
            let p = PathBuf::from(r);
            if p.join("projects/INDEX.md").is_file() {
                return Ok(p);
            }
        }
    }
    if let Ok(script_dir) = std::env::var("QUAQUA_SCRIPT_DIR") {
        let p = PathBuf::from(script_dir);
        if let Some(parent2) = p.parent().and_then(|p| p.parent()) {
            if parent2.join("projects/INDEX.md").is_file() {
                return Ok(parent2.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd.clone();
        loop {
            if cur.join("projects/INDEX.md").is_file() {
                return Ok(cur);
            }
            if !cur.pop() {
                break;
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let mut cur = parent.to_path_buf();
            loop {
                if cur.join("projects/INDEX.md").is_file() {
                    return Ok(cur);
                }
                if !cur.pop() {
                    break;
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        return Ok(cwd);
    }
    Ok(PathBuf::from("."))
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
}
