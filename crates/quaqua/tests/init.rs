//! `quaqua init` smoke test: boot a temp skeleton root through the built binary
//! with `QUAQUA_SCRIPT_DIR` pointing at the shim layout
//! (`<root>/scripts/workspace`), and check the booted state. `PATH` is pointed
//! at an empty dir so no real `git`/`herdr` is ever touched.

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_root(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("quaqua-init-test-{tag}-{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn skeleton(root: &Path) {
    std::fs::create_dir_all(root.join("scripts/workspace")).unwrap();
    std::fs::create_dir_all(root.join("templates")).unwrap();
    std::fs::write(root.join("templates/memory-rules.md"), "# rules\n").unwrap();
    std::fs::write(
        root.join("templates/projects-INDEX.md"),
        "| slug | repo | owner |\n",
    )
    .unwrap();
    std::fs::write(
        root.join("templates/projects-_template.md"),
        "# p\n\n## Now\n\n## Vision\n",
    )
    .unwrap();
    std::fs::write(root.join("USER.example.md"), "# user\n").unwrap();
}

fn run_init(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_quaqua"))
        .arg("init")
        .env("QUAQUA_SCRIPT_DIR", root.join("scripts/workspace"))
        .env("PATH", root.join("nobin"))
        .env_remove("HERDR_PANE_ID")
        .current_dir("/")
        .output()
        .expect("failed to spawn the quaqua binary")
}

#[test]
fn init_boots_a_skeleton_and_is_idempotent() {
    let root = temp_root("boot");
    skeleton(&root);

    let out = run_init(&root);
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    for d in ["memory", "projects", "raw", "runs", "reports"] {
        assert!(root.join(d).is_dir(), "missing dir {d}/");
    }
    for f in [
        "memory/rules.md",
        "projects/INDEX.md",
        "projects/_template.md",
        "USER.md",
    ] {
        assert!(root.join(f).is_file(), "missing seeded file {f}");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("cargo install --path crates/quaqua"),
        "install line missing: {stdout}",
    );

    // re-run must not clobber local content
    std::fs::write(root.join("memory/rules.md"), "# mine\n").unwrap();
    let out = run_init(&root);
    assert!(out.status.success(), "re-run failed");
    assert_eq!(
        std::fs::read_to_string(root.join("memory/rules.md")).unwrap(),
        "# mine\n",
        "re-run clobbered memory/rules.md",
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn init_fails_loudly_on_a_missing_template() {
    let root = temp_root("missing");
    skeleton(&root);
    std::fs::remove_file(root.join("templates/memory-rules.md")).unwrap();

    let out = run_init(&root);
    assert!(!out.status.success(), "expected failure on a missing template");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing template"),
        "unclear failure: {stderr}",
    );

    let _ = std::fs::remove_dir_all(&root);
}
