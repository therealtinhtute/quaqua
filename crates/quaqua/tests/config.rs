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
    repo_root()
        .join("templates/workspace/team.toml")
        .to_string_lossy()
        .into_owned()
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
fn config_repo_kinds_carry_model_and_effort() {
    // team.toml is the only source of harness/model/effort: the repo config must
    // validate on its own (no models.md drift check anymore).
    let out = run(&["config", "validate", "--file", &repo_team()]);
    assert!(out.status.success(), "validate failed: {}", stderr(&out));
    let effort = run(&["config", "get", "kinds.tieude.effort", "--file", &repo_team()]);
    assert_eq!(stdout(&effort).trim(), "max");
    let fallback = run(&["config", "get", "kinds.sude.fallback", "--file", &repo_team()]);
    assert_eq!(stdout(&fallback).trim(), "suhuynh");
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
    assert_eq!(stdout(&out), "suhuynh\n");
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
    assert_eq!(stdout(&base).trim(), "8");

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
    let can = run(&["config", "get", "kinds.suhuynh.can_review", "--file", &repo_team()]);
    assert_eq!(stdout(&can).trim(), "true");
    let cannot = run(&["config", "get", "kinds.tieude.can_review", "--file", &repo_team()]);
    assert_eq!(stdout(&cannot).trim(), "false");
    let harness = run(&["config", "get", "kinds.sude.harness", "--file", &repo_team()]);
    assert_eq!(stdout(&harness).trim(), "omp");
    let effort = run(&["config", "get", "kinds.sude.effort", "--file", &repo_team()]);
    assert_eq!(stdout(&effort).trim(), "high");
    let spawn = run(&["config", "get", "kinds.truonglao.spawn", "--file", &repo_team()]);
    assert_eq!(stdout(&spawn).trim(), "auto");
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
/// rule that fired.
#[test]
fn config_invalid_fixtures_are_rejected() {
    let cases: &[(&str, &str)] = &[
        ("cap-over.toml", "over the cap"),
        ("bad-weights.toml", "exceeds cap"),
        ("project-cap-over.toml", "exceeds global cap"),
        ("unknown-kind.toml", "unknown kind 'ghost'"),
        ("unknown-class.toml", "unknown role-class 'ghostrole'"),
        ("unknown-role.toml", "unknown role 'ghost'"),
        ("same-harness.toml", "share harness"),
        ("reviewer-cannot-review.toml", "can_review = false"),
        ("missing-effort.toml", "effort is missing"),
        ("bad-effort.toml", "does not take effort 'banana'"),
        ("bad-opencode-variant.toml", "does not take effort 'medium'"),
        ("bad-opencode-openrouter-variant.toml", "does not take effort 'medium'"),
        ("unknown-opencode-model.toml", "is not in allowlist (fix: add model and variants to team.toml [opencode_models])"),
        ("fallback-missing.toml", "fallback 'ghost' is not a kind"),
        ("fallback-self.toml", "fallback names itself"),
        ("fallback-cannot-review.toml", "cannot review"),
        ("bad-schema.toml", "unsupported schema_version"),
        ("integ-branch.toml", "{slug}"),
        ("differ-policy.toml", "unknown differ policy"),
        ("typo-key.toml", "unknown field `can_reveiw`"),
        ("typo-section.toml", "unknown field `project`"),
        ("empty-reviewer.toml", "default_reviewer is empty"),
        ("project-cap-zero.toml", "is below 1"),
        ("bad-timer-duration.toml", "timers.budget_default"),
        ("bad-timer-nudge.toml", "timers.nudge_at"),
        ("bad-timer-grace.toml", "timers.grace"),
        ("unknown-spawn.toml", "unknown spawn policy 'sometimes'"),
    ];

    for (name, needle) in cases {
        let path = fixtures_dir().join("invalid").join(name);
        let path_s = path.to_string_lossy().into_owned();
        let out = run(&["config", "validate", "--file", path_s.as_str()]);
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

#[test]
fn config_get_purposes() {
    let out = run(&["config", "get", "purposes.review", "--file", &repo_team()]);
    assert!(out.status.success(), "get failed: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "suhuynh");
}

#[test]
fn config_get_opencode_models() {
    let out = run(&["config", "get", "opencode_models.deepseek-v4.1-flash", "--file", &repo_team()]);
    assert!(out.status.success(), "get failed: {}", stderr(&out));
    assert!(stdout(&out).contains("max"));
}
