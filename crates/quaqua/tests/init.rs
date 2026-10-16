//! `quaqua init` smoke tests: boot temp skeleton roots through the built binary
//! with `QUAQUA_SCRIPT_DIR` pointing at the shim layout
//! (`<root>/scripts/workspace`). `PATH` is pointed at a controlled dir so no real
//! `git`/`herdr` is ever touched unless a test deliberately links `git` in.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

struct Invocation {
    script_dir: PathBuf,
    cwd: PathBuf,
    path: PathBuf,
    extra: Vec<(String, String)>,
}

impl Invocation {
    fn new(root: &Path) -> Self {
        Invocation {
            script_dir: root.join("scripts/workspace"),
            cwd: PathBuf::from("/"),
            path: root.join("nobin"),
            extra: Vec::new(),
        }
    }
    fn run(&self) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quaqua"));
        cmd.arg("init")
            .env("QUAQUA_SCRIPT_DIR", &self.script_dir)
            .env("PATH", &self.path)
            .env_remove("HERDR_PANE_ID")
            .current_dir(&self.cwd);
        for (k, v) in &self.extra {
            cmd.env(k, v);
        }
        cmd.output().expect("failed to spawn the quaqua binary")
    }
}

fn find_git() -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let cand = Path::new(dir).join("git");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[test]
fn init_boots_a_skeleton_and_is_idempotent() {
    let root = temp_root("boot");
    skeleton(&root);

    let out = Invocation::new(&root).run();
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
    let out = Invocation::new(&root).run();
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

    let out = Invocation::new(&root).run();
    assert!(
        !out.status.success(),
        "expected failure on a missing template"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing template"),
        "unclear failure: {stderr}",
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Finding 3 (runs/suhuynh-r5r1.md): the root is canonicalized (`pwd -P`), so a
/// symlinked checkout path seeds the real dir and reports the real path.
#[test]
fn init_uses_the_canonical_root_for_a_symlinked_path() {
    let real = temp_root("symlink-real");
    skeleton(&real);
    let link = temp_root("symlink-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mut inv = Invocation::new(&real);
    inv.script_dir = link.join("scripts/workspace");
    let out = inv.run();
    assert!(
        out.status.success(),
        "init failed through a symlink: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        real.join("memory/rules.md").is_file(),
        "seeded files not written to the canonical root",
    );
    let canonical = std::fs::canonicalize(&real).unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("workspace ready in {}", canonical.display())),
        "root not canonicalized: {stdout}",
    );

    let _ = std::fs::remove_dir_all(&link);
    let _ = std::fs::remove_dir_all(&real);
}

/// Finding 4 (runs/suhuynh-r5r1.md): a bad `QUAQUA_SCRIPT_DIR` hint has exactly
/// one candidate (`<hint>/../..`) and fails there — it never silently falls back
/// to a different checkout found from cwd.
#[test]
fn init_fails_when_the_script_dir_hint_has_no_templates() {
    let cwd_root = temp_root("hint-cwd");
    skeleton(&cwd_root);
    let hint = temp_root("hint-bad");
    std::fs::create_dir_all(hint.join("scripts/workspace")).unwrap();

    let mut inv = Invocation::new(&hint);
    inv.cwd = cwd_root.clone();
    let out = inv.run();
    assert!(
        !out.status.success(),
        "expected failure for a hint with no templates/",
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no templates/ in"),
        "unclear failure: {stderr}",
    );
    assert!(
        !cwd_root.join("memory").exists(),
        "init fell back to a different checkout found from cwd",
    );

    let _ = std::fs::remove_dir_all(&cwd_root);
    let _ = std::fs::remove_dir_all(&hint);
}

/// Finding 5 (runs/suhuynh-r5r1.md): a failed `git config core.hooksPath` write
/// fails the run (bash `set -e`), instead of printing `workspace ready` with
/// unwired hooks.
#[test]
fn init_fails_when_it_cannot_wire_hooks() {
    let Some(git) = find_git() else {
        return; // no git on PATH: nothing to assert
    };
    let root = temp_root("hookfail");
    skeleton(&root);
    let ok = Command::new(&git)
        .args(["init", "-q"])
        .arg(&root)
        .status()
        .expect("git init");
    assert!(ok.success(), "git init failed");

    // A controlled PATH with only git, so no real herdr is reachable.
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&git, bin.join("git")).unwrap();
    // Point git's config file at a directory: reads report "unset", the write fails.
    let configdir = root.join("configdir");
    std::fs::create_dir_all(&configdir).unwrap();

    let mut inv = Invocation::new(&root);
    inv.path = bin;
    inv.extra.push(("GIT_CONFIG".to_string(), configdir.to_string_lossy().to_string()));
    let out = inv.run();
    assert!(
        !out.status.success(),
        "expected failure when core.hooksPath cannot be written",
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to set core.hooksPath"),
        "unclear failure: {stderr}",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("workspace ready"),
        "run reported success despite unwired hooks: {stdout}",
    );

    let _ = std::fs::remove_dir_all(&root);
}
