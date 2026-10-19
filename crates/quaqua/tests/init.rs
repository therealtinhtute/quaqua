//! `quaqua init` smoke tests: boot temp skeleton roots through the built binary
//! with `QUAQUA_SCRIPT_DIR` pointing at the shim layout
//! (`<root>/scripts/workspace`). `PATH` is pointed at a controlled dir so no real
//! `git`/`herdr` is ever touched unless a test deliberately links `git` in; an
//! ambient `HERDR_BIN_PATH` cannot leak either — every invocation pins it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Stub `herdr` for `HERDR_BIN_PATH`: logs argv, answers the pane queries as an
/// unregistered `claude` agent in this checkout (the pane the CoS tag is for).
const HERDR_STUB: &str = r#"#!/bin/sh
echo "$*" >> @LOG@
case "$1 $2" in
  "pane current") echo '{"result":{"pane":{"pane_id":"w1:p1"}}}' ;;
  "pane get") echo '{"result":{"pane":{"pane_id":"w1:p1","agent":"claude","cwd":"@CWD@"}}}' ;;
  "agent list") echo '{"result":{"agents":[{"pane_id":"w1:p1","agent":"claude"}]}}' ;;
esac
exit 0
"#;

/// Decoy `herdr` for `PATH`: records that it ran and reports an agent, so a
/// PATH lookup is visible in the audit prompt instead of silently passing.
const HERDR_DECOY: &str = r#"#!/bin/sh
: > "@MARKER@"
echo '{"result":{"agents":[{"name":"sude-decoy","agent_status":"idle","pane_id":"p9"}]}}'
exit 0
"#;

fn write_script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

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
        let mut inv = Invocation {
            script_dir: root.join("scripts/workspace"),
            cwd: PathBuf::from("/"),
            path: root.join("nobin"),
            extra: Vec::new(),
        };
        // Pin herdr resolution to a path that does not exist, so an ambient
        // HERDR_BIN_PATH (a real herdr on the driving machine) is never exec'd
        // by a test that is not about herdr resolution.
        inv.extra.push((
            "HERDR_BIN_PATH".to_string(),
            root.join("nobin/herdr").to_string_lossy().to_string(),
        ));
        inv
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

#[test]
fn init_wires_embedded_hook_and_migrates_legacy() {
    let Some(git) = find_git() else {
        return;
    };
    let root = temp_root("wirehook");
    skeleton(&root);
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let ok = Command::new(&git)
        .args(["init", "-q"])
        .arg(&root)
        .status()
        .expect("git init");
    assert!(ok.success(), "git init failed");

    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&git, bin.join("git")).unwrap();

    // 1. Fresh clone: wires .git/quaqua-hooks/pre-commit (+exec) and sets core.hooksPath
    let mut inv = Invocation::new(&root);
    inv.path = bin.clone();
    let out = inv.run();
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));

    let hook = root.join(".git/quaqua-hooks/pre-commit");
    assert!(hook.is_file(), "hook was not created");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "hook is not executable");
    }

    let hook_content = std::fs::read_to_string(&hook).unwrap();
    assert!(hook_content.contains("portable-paths.md"));

    let hooks_path = Command::new(&git)
        .arg("-C")
        .arg(&root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&hooks_path.stdout).trim(),
        root.join(".git/quaqua-hooks").to_string_lossy().trim()
    );

    // 2. Modified hook is refreshed + logged
    std::fs::write(&hook, "tampered").unwrap();
    let out = inv.run();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hooks: refreshed"), "no refresh log: {stdout}");
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), hook_content);

    // 3. Legacy migration: core.hooksPath set to scripts/hooks
    let legacy_path = root.join("scripts/hooks");
    let ok = Command::new(&git)
        .arg("-C")
        .arg(&root)
        .args(["config", "core.hooksPath", &legacy_path.to_string_lossy()])
        .status()
        .unwrap();
    assert!(ok.success());

    let out = inv.run();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("migrated core.hooksPath"), "no migration log: {stdout}");

    let hooks_path = Command::new(&git)
        .arg("-C")
        .arg(&root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&hooks_path.stdout).trim(),
        root.join(".git/quaqua-hooks").to_string_lossy().trim()
    );

    // 4. Idempotent re-run is quiet
    let out = inv.run();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("migrated"), "re-run logged migration: {stdout}");
    assert!(!stdout.contains("core.hooksPath="), "re-run logged core.hooksPath: {stdout}");

    let _ = std::fs::remove_dir_all(&root);
}

/// Every herdr exec `quaqua init` makes — the pane queries (pane current/get,
/// agent list) and the CoS tag (`pane report-metadata`) — must run the binary
/// `HERDR_BIN_PATH` names, not one found through `PATH`.
#[test]
fn init_resolves_herdr_through_herdr_bin_path_only() {
    let root = temp_root("herdrbin");
    skeleton(&root);
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    let stub = root.join("stub/herdr");
    std::fs::create_dir_all(stub.parent().unwrap()).unwrap();
    let log = root.join("stub.calls");
    write_script(
        &stub,
        &HERDR_STUB
            .replace("@LOG@", &log.display().to_string())
            .replace("@CWD@", &root.display().to_string()),
    );

    // 1. No herdr on PATH at all: only HERDR_BIN_PATH can answer, so every
    //    query and the tag itself must have gone through the stub.
    let mut inv = Invocation::new(&root);
    inv.extra
        .push(("HERDR_BIN_PATH".to_string(), stub.display().to_string()));
    let out = inv.run();
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    for want in [
        "pane current",
        "pane get w1:p1",
        "agent list",
        "pane report-metadata w1:p1 --source herdr-quaqua \
         --token grid_role=orchestrator --token grid_owner=cos",
    ] {
        assert!(
            calls.contains(want),
            "HERDR_BIN_PATH binary missed '{want}': {calls}",
        );
    }

    // 2. A decoy herdr FIRST on PATH: with HERDR_BIN_PATH set it must never be
    //    executed, and the tag must still come from the stub.
    let marker = root.join("decoy.ran");
    let decoy_dir = root.join("decoy");
    std::fs::create_dir_all(&decoy_dir).unwrap();
    write_script(
        &decoy_dir.join("herdr"),
        &HERDR_DECOY.replace("@MARKER@", &marker.display().to_string()),
    );
    std::fs::remove_file(&log).unwrap();
    let mut inv = Invocation::new(&root);
    inv.path = decoy_dir;
    inv.extra
        .push(("HERDR_BIN_PATH".to_string(), stub.display().to_string()));
    let out = inv.run();
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        !marker.exists(),
        "the PATH herdr was executed despite HERDR_BIN_PATH",
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("pane report-metadata w1:p1"),
        "the HERDR_BIN_PATH binary did not tag the CoS pane: {calls}",
    );

    let _ = std::fs::remove_dir_all(&root);
}
