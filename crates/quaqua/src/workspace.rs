//! The one workspace root.
//!
//! Every workspace path the binary resolves (rules, runs, team.toml) goes
//! through [`workspace_root`]: `QUAQUA_HOME` first, else `$HOME/quaqua`.
//!
//! Not to be confused with [`crate::lead::spawn::find_quaqua_root`], which
//! discovers the checkout that owns `projects/INDEX.md` (cwd / `QUAQUA_ROOT`
//! / exe walk). This module is the installed workspace directory.

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
