//! `quaqua doctor` end-to-end: missing on PATH, a shell function/alias
//! shadowing it, a stale binary, and a healthy setup (issue #55 item 3).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
    bin_dir: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    checkout: PathBuf,
    shell: PathBuf,
}

impl Fixture {
    fn new(tag: &str, shell_line: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("quaqua-doctor-cli-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let f = Fixture {
            bin_dir: root.join("bin"),
            home: root.join("home"),
            cwd: root.join("cwd"),
            checkout: root.join("checkout"),
            shell: root.join("shell"),
            root,
        };
        for d in [&f.bin_dir, &f.home, &f.cwd] {
            fs::create_dir_all(d).unwrap();
        }
        fs::create_dir_all(f.checkout.join("crates/quaqua/src")).unwrap();
        fs::write(
            &f.shell,
            format!("#!/bin/sh\nprintf '%s\\n' '{shell_line}'\n"),
        )
        .unwrap();
        fs::set_permissions(&f.shell, fs::Permissions::from_mode(0o755)).unwrap();
        f
    }

    fn install_binary(&self) {
        let dest = self.bin_dir.join("quaqua");
        fs::copy(env!("CARGO_BIN_EXE_quaqua"), &dest).unwrap();
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, path: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_quaqua"))
            .arg("doctor")
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("QUAQUA_CHECKOUT", &self.checkout)
            .env("QUAQUA_SHELL", &self.shell)
            .current_dir(&self.cwd)
            .output()
            .expect("run quaqua doctor")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn missing_binary_exits_nonzero() {
    let f = Fixture::new("missing", "quaqua not found");
    let empty = f.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    let out = f.run(&empty);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not on PATH"), "{stderr}");
}

#[test]
fn shadowed_by_a_shell_function_exits_nonzero() {
    let f = Fixture::new("shadow", "quaqua is a shell function");
    f.install_binary();
    let out = f.run(&f.bin_dir);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("function/alias"), "{stderr}");
}

#[test]
fn a_stale_binary_exits_nonzero() {
    let f = Fixture::new("stale", "quaqua is /tmp/bin/quaqua");
    f.install_binary();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(
        f.checkout.join("crates/quaqua/src/newer.rs"),
        "fn newer() {}",
    )
    .unwrap();
    let out = f.run(&f.bin_dir);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("older than the checkout"), "{stderr}");
}

#[test]
fn a_healthy_setup_exits_zero() {
    let f = Fixture::new("healthy", "quaqua is /tmp/bin/quaqua");
    f.install_binary();
    let out = f.run(&f.bin_dir);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok:"), "{stdout}");
}

#[test]
fn a_workspace_that_is_also_an_engine_checkout_warns_and_still_exits_zero() {
    let f = Fixture::new("overlap", "quaqua is /tmp/bin/quaqua");
    f.install_binary();
    // `$HOME/quaqua` before the split: the workspace root held the engine
    // sources and the engine `AGENTS.md`, so the persona never loaded.
    let workspace = f.home.join("quaqua");
    fs::create_dir_all(workspace.join("crates/quaqua/src")).unwrap();
    fs::write(workspace.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(workspace.join("AGENTS.md"), "# quaqua engine\n").unwrap();

    let out = f.run(&f.bin_dir);
    // A broken layout warns; it never fails the run.
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("warning:"), "{stderr}");
    assert!(stderr.contains("Cargo.toml"), "{stderr}");
    assert!(stderr.contains("@identity.md"), "{stderr}");
}

#[test]
fn a_split_workspace_is_quiet() {
    let f = Fixture::new("split", "quaqua is /tmp/bin/quaqua");
    f.install_binary();
    let workspace = f.home.join("quaqua");
    fs::create_dir_all(workspace.join("projects")).unwrap();
    fs::write(workspace.join("projects/INDEX.md"), "").unwrap();
    fs::write(workspace.join("AGENTS.md"), "@identity.md\n@SOUL.md\n").unwrap();

    let out = f.run(&f.bin_dir);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("warning:"), "{stderr}");
}
