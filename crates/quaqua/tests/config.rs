//! R1 integration tests: `quaqua config validate` and `quaqua config get` against
//! the repo-root `team.toml` and the invalid fixtures. Each case shells out to the
//! built binary (CARGO_BIN_EXE_quaqua), so the CLI wiring is under test, not just
//! the library.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_team() -> String {
    repo_root().join("team.toml").to_string_lossy().into_owned()
}

fn repo_models() -> String {
    repo_root().join("models.md").to_string_lossy().into_owned()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quaqua"))
        .args(args)
        .output()
        .expect("failed to spawn the quaqua binary")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn config_repo_team_toml_validates_ok() {
    let out = run(&["config", "validate", "--file", &repo_team()]);
    assert!(out.status.success(), "validate failed: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "ok");
}

#[test]
fn config_repo_kinds_validate_against_models_md() {
    // The real drift check: every [kinds.*] must equal its models.md routing cell.
    let out = run(&[
        "config",
        "validate",
        "--file",
        &repo_team(),
        "--models",
        &repo_models(),
    ]);
    assert!(out.status.success(), "drift check failed: {}", stderr(&out));
}

#[test]
fn config_base_fixture_validates_ok() {
    let base = fixtures_dir().join("base.toml");
    let out = run(&["config", "validate", "--file", &base.to_string_lossy()]);
    assert!(out.status.success(), "base fixture rejected: {}", stderr(&out));
}

/// Review round 2, finding 1: `--project X` must not re-check the other projects
/// (or the weights) against X's resolved cap.
#[test]
fn config_validate_project_does_not_recheck_other_projects() {
    let f = fixtures_dir().join("two-projects.toml");
    let f = f.to_string_lossy().into_owned();

    // plain validate and every project selection must pass
    for args in [
        vec!["config", "validate", "--file", f.as_str()],
        vec!["config", "validate", "--file", f.as_str(), "--project", "a"],
        vec!["config", "validate", "--file", f.as_str(), "--project", "b"],
        vec!["config", "validate", "--file", f.as_str(), "--project", "c"],
    ] {
        let out = run(&args);
        assert!(
            out.status.success(),
            "{args:?} rejected: {}",
            stderr(&out)
        );
    }

    // an unknown project is still an error
    let ghost = run(&[
        "config", "validate", "--file", &f, "--project", "ghost",
    ]);
    assert!(!ghost.status.success());
    assert!(stderr(&ghost).contains("unknown project 'ghost'"), "{}", stderr(&ghost));
}

/// Review round 2, finding 2: an explicit `--models <path>` that does not exist
/// is an error; only the implicit sibling lookup may be missing.
#[test]
fn config_explicit_models_missing_is_error() {
    let missing = repo_root().join("does-not-exist-models.md");
    let out = run(&[
        "config",
        "validate",
        "--file",
        &repo_team(),
        "--models",
        &missing.to_string_lossy(),
    ]);
    assert!(!out.status.success(), "explicit missing --models was accepted");
    assert!(stderr(&out).contains("cannot read"), "stderr: {}", stderr(&out));
}

#[test]
fn config_get_default_reviewer_is_a_list_one_per_line() {
    let out = run(&[
        "config",
        "get",
        "strategy.default_reviewer",
        "--file",
        &repo_team(),
    ]);
    assert!(out.status.success(), "get failed: {}", stderr(&out));
    assert_eq!(stdout(&out), "quansu\nsuhuynh\n");
}

#[test]
fn config_get_cap_applies_project_override() {
    let base = run(&[
        "config",
        "get",
        "limits.max_workers_per_orchestrator",
        "--file",
        &repo_team(),
    ]);
    assert_eq!(stdout(&base).trim(), "4");

    let zimage = run(&[
        "config",
        "get",
        "limits.max_workers_per_orchestrator",
        "--file",
        &repo_team(),
        "--project",
        "zimage",
    ]);
    assert!(zimage.status.success(), "override get failed: {}", stderr(&zimage));
    assert_eq!(stdout(&zimage).trim(), "3");
}

#[test]
fn config_get_can_review_and_kind_fields() {
    let can = run(&["config", "get", "kinds.quansu.can_review", "--file", &repo_team()]);
    assert_eq!(stdout(&can).trim(), "true");
    let cannot = run(&["config", "get", "kinds.tieude.can_review", "--file", &repo_team()]);
    assert_eq!(stdout(&cannot).trim(), "false");
    let harness = run(&["config", "get", "kinds.sude.harness", "--file", &repo_team()]);
    assert_eq!(stdout(&harness).trim(), "omp");
    let effort = run(&["config", "get", "kinds.sude.effort", "--file", &repo_team()]);
    assert_eq!(stdout(&effort).trim(), "high");
}

#[test]
fn config_get_unknown_key_fails_clearly() {
    let out = run(&["config", "get", "nope.nope", "--file", &repo_team()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown key 'nope.nope'"), "stderr: {}", stderr(&out));
}

#[test]
fn config_get_unknown_project_fails_clearly() {
    let out = run(&[
        "config",
        "get",
        "limits.max_workers_per_orchestrator",
        "--file",
        &repo_team(),
        "--project",
        "ghost",
    ]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown project 'ghost'"), "stderr: {}", stderr(&out));
}

/// Each invalid fixture is rejected, exits non-zero, and the message names the
/// rule that fired. The `models` argument is passed only for the drift fixture.
#[test]
fn config_invalid_fixtures_are_rejected() {
    let cases: &[(&str, &str, bool)] = &[
        ("cap-over.toml", "over the cap", false),
        ("bad-weights.toml", "exceeds cap", false),
        ("project-cap-over.toml", "exceeds global cap", false),
        ("unknown-kind.toml", "unknown kind 'ghost'", false),
        ("unknown-class.toml", "unknown role-class 'ghostrole'", false),
        ("unknown-role.toml", "unknown role 'ghost'", false),
        ("same-harness.toml", "share harness", false),
        ("reviewer-cannot-review.toml", "can_review = false", false),
        ("model-drift.toml", "drift", true),
        ("missing-kind.toml", "absent from team.toml", true),
        ("bad-schema.toml", "unsupported schema_version", false),
        ("integ-branch.toml", "{slug}", false),
        ("differ-policy.toml", "unknown differ policy", false),
        ("typo-key.toml", "unknown field `can_reveiw`", false),
        ("typo-section.toml", "unknown field `project`", false),
        ("empty-reviewer.toml", "default_reviewer is empty", false),
        ("project-cap-zero.toml", "is below 1", false),
    ];

    for (name, needle, needs_models) in cases {
        let path = fixtures_dir().join("invalid").join(name);
        let path_s = path.to_string_lossy().into_owned();
        let mut args = vec!["config", "validate", "--file", path_s.as_str()];
        let models_s;
        if *needs_models {
            models_s = repo_models();
            args.push("--models");
            args.push(&models_s);
        }
        let out = run(&args);
        let err = stderr(&out);
        assert!(
            !out.status.success(),
            "{name}: expected rejection, got success (stdout={:?})",
            stdout(&out)
        );
        assert!(
            err.contains(needle),
            "{name}: stderr {err:?} does not contain {needle:?}"
        );
    }
}
