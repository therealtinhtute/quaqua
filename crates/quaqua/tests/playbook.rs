//! Playbook acceptance tests (§ S3 — Playbook module).
//!
//! Acceptance cases:
//! 1. `list` built from fixture frontmatter
//! 2. `show` prints only Orchestrator
//! 3. step ids parsed
//! 4. unknown `principle-<name>` fails
//! 5. a 4th cited leaf fails
//!
//! Verify command: `cargo test --locked -p quaqua playbook`

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use quaqua::playbook::{
    extract_principle_citations, load_playbook, parse_playbook, run_list, run_show, PlaybookError,
};

/// Helper RAII temporary directory for test isolation.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("quaqua-playbook-{tag}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn sub(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Helper to run the built quaqua CLI binary.
fn run_cli(args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to execute quaqua binary");
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (code, stdout, stderr)
}

// ─────────────────────────────────────────────────────────────────────────────
// Acceptance Case 1: `list` built from fixture frontmatter
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_list_built_from_fixture_frontmatter() {
    let sandbox = TempDir::new("list");
    let pb_dir = sandbox.sub("playbooks");
    let pr_dir = sandbox.sub("principles");

    sandbox.write_file(
        "principles/fixture.md",
        "# Fixture principle\n\nFor testing.\n",
    );
    sandbox.write_file(
        "playbooks/fixture.md",
        r#"---
name: fixture
when: Used for engine tests and verification.
---

## Orchestrator

1. Plan the task.

## Worker

1. Step one citing principle-fixture.
"#,
    );

    let output = run_list(&pb_dir, &pr_dir).expect("run_list should succeed");
    assert_eq!(
        output, "fixture — Used for engine tests and verification.\n",
        "output must match '<name> — <when>'"
    );

    // Also verify multiple playbooks sorted alphabetically
    sandbox.write_file(
        "playbooks/alpha.md",
        r#"---
name: alpha
when: First in the alphabet.
---

## Orchestrator

Plan alpha.

## Worker

1. Execute alpha.
"#,
    );

    let output_multi = run_list(&pb_dir, &pr_dir).expect("run_list multi should succeed");
    let lines: Vec<&str> = output_multi.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "alpha — First in the alphabet.");
    assert_eq!(
        lines[1],
        "fixture — Used for engine tests and verification."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Acceptance Case 2: `show` prints only Orchestrator
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_show_prints_only_orchestrator() {
    let sandbox = TempDir::new("show");
    let pb_dir = sandbox.sub("playbooks");
    let pr_dir = sandbox.sub("principles");

    sandbox.write_file(
        "principles/fixture.md",
        "# Fixture principle\n\nFor testing.\n",
    );
    sandbox.write_file(
        "playbooks/fixture.md",
        r#"---
name: fixture
when: Used for engine tests and verification.
---

## Orchestrator

1. Plan the fixture task.
2. Delegate execution to worker.

## Worker

1. Execute fixture step one.
2. Execute fixture step two citing principle-fixture.
"#,
    );

    let output = run_show("fixture", &pb_dir, &pr_dir).expect("run_show should succeed");

    // Output must contain the Orchestrator section
    assert!(
        output.contains("## Orchestrator"),
        "must contain '## Orchestrator'"
    );
    assert!(
        output.contains("1. Plan the fixture task."),
        "must contain orchestrator step 1"
    );
    assert!(
        output.contains("2. Delegate execution to worker."),
        "must contain orchestrator step 2"
    );

    // Output must NOT contain Worker section or frontmatter
    assert!(
        !output.contains("## Worker"),
        "must NOT contain '## Worker'"
    );
    assert!(
        !output.contains("Execute fixture step one"),
        "must NOT contain worker step 1"
    );
    assert!(
        !output.contains("Execute fixture step two"),
        "must NOT contain worker step 2"
    );
    assert!(
        !output.contains("name: fixture"),
        "must NOT contain frontmatter name"
    );
    assert!(
        !output.contains("when:"),
        "must NOT contain frontmatter when"
    );
    assert!(
        !output.contains("---"),
        "must NOT contain frontmatter delimiter"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Acceptance Case 3: step ids parsed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_step_ids_parsed() {
    let sandbox = TempDir::new("steps");
    let pr_dir = sandbox.sub("principles");

    sandbox.write_file("principles/leaf.md", "# Leaf\n");

    let content = r#"---
name: step-test
when: Testing step parsing.
---

## Orchestrator

1. Plan.

## Worker

1. First step text.
2. Second step text citing principle-leaf.
3. Third step with multiple
   lines of body text.
10. Tenth step with gap in numbering.
"#;

    let playbook = parse_playbook(content, &pr_dir).expect("playbook should parse");
    assert_eq!(playbook.step_ids(), vec![1, 2, 3, 10]);

    assert_eq!(playbook.steps.len(), 4);
    assert_eq!(playbook.steps[0].id, 1);
    assert_eq!(playbook.steps[0].text, "First step text.");
    assert_eq!(playbook.steps[0].cited_principles, Vec::<String>::new());

    assert_eq!(playbook.steps[1].id, 2);
    assert_eq!(playbook.steps[1].cited_principles, vec!["principle-leaf"]);

    assert_eq!(playbook.steps[2].id, 3);
    assert!(playbook.steps[2].text.contains("multiple\n   lines"));

    assert_eq!(playbook.steps[3].id, 10);
    assert_eq!(playbook.steps[3].text, "Tenth step with gap in numbering.");
}

// ─────────────────────────────────────────────────────────────────────────────
// Acceptance Case 4: unknown `principle-<name>` fails
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_unknown_principle_fails() {
    let sandbox = TempDir::new("unknown-principle");
    let pr_dir = sandbox.sub("principles");

    // Only "existing.md" exists in principles/
    sandbox.write_file("principles/existing.md", "# Existing\n");

    let content = r#"---
name: bad-principle
when: Testing unknown principle.
---

## Orchestrator

1. Plan.

## Worker

1. Step one citing principle-nonexistent.
"#;

    let err = parse_playbook(content, &pr_dir).expect_err("must fail on unknown principle");
    match err {
        PlaybookError::UnknownPrinciple { principle, path } => {
            assert_eq!(principle, "principle-nonexistent");
            assert!(
                path.ends_with("nonexistent.md"),
                "path should point to nonexistent.md"
            );
        }
        other => panic!("expected UnknownPrinciple, got: {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Acceptance Case 5: a 4th cited leaf fails
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_fourth_cited_leaf_fails() {
    let sandbox = TempDir::new("leaf-cap");
    let pr_dir = sandbox.sub("principles");

    sandbox.write_file("principles/leaf-a.md", "# Leaf A\n");
    sandbox.write_file("principles/leaf-b.md", "# Leaf B\n");
    sandbox.write_file("principles/leaf-c.md", "# Leaf C\n");
    sandbox.write_file("principles/leaf-d.md", "# Leaf D\n");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    // Case 5a: exactly 4 leaves besides principle-prove-it-works -> FAILS
    let four_leaves_content = r#"---
name: four-leaves
when: Testing 4th leaf cap failure.
---

## Orchestrator

1. Plan.

## Worker

1. Step one citing principle-leaf-a.
2. Step two citing principle-leaf-b.
3. Step three citing principle-leaf-c.
4. Step four citing principle-leaf-d.
"#;

    let err = parse_playbook(four_leaves_content, &pr_dir)
        .expect_err("Worker section citing 4 non-exempt leaves must fail");
    match err {
        PlaybookError::LeafCapExceeded { count, max, leaves } => {
            assert_eq!(count, 4);
            assert_eq!(max, 3);
            assert_eq!(leaves.len(), 4);
        }
        other => panic!("expected LeafCapExceeded, got: {other:?}"),
    }

    // Case 5b: 3 leaves besides principle-prove-it-works + principle-prove-it-works itself -> SUCCEEDS
    let three_plus_prove_content = r#"---
name: three-plus-prove
when: Testing 3 leaves + prove-it-works succeeds.
---

## Orchestrator

1. Plan.

## Worker

1. Step one citing principle-leaf-a.
2. Step two citing principle-leaf-b.
3. Step three citing principle-leaf-c.
4. Step four citing principle-prove-it-works.
"#;

    let pb = parse_playbook(three_plus_prove_content, &pr_dir)
        .expect("3 leaves + principle-prove-it-works must succeed");
    assert_eq!(pb.cited_principles.len(), 4);

    // Case 5c: 4 leaves besides principle-prove-it-works + principle-prove-it-works -> FAILS
    let four_plus_prove_content = r#"---
name: four-plus-prove
when: Testing 4 leaves + prove-it-works fails.
---

## Orchestrator

1. Plan.

## Worker

1. Step one citing principle-leaf-a.
2. Step two citing principle-leaf-b.
3. Step three citing principle-leaf-c.
4. Step four citing principle-leaf-d.
5. Step five citing principle-prove-it-works.
"#;

    let err2 = parse_playbook(four_plus_prove_content, &pr_dir)
        .expect_err("4 non-exempt leaves + prove-it-works must still fail");
    match err2 {
        PlaybookError::LeafCapExceeded { count, .. } => {
            assert_eq!(count, 4);
        }
        other => panic!("expected LeafCapExceeded, got: {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Workspace Fixture Tests: verifying templates/workspace/ fixture files
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn workspace_fixture_playbook_and_leaf_exist_and_validate() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    assert!(
        playbooks_dir.is_dir(),
        "templates/workspace/playbooks must exist"
    );
    assert!(
        principles_dir.is_dir(),
        "templates/workspace/principles must exist"
    );

    let fixture_pb_path = playbooks_dir.join("fixture.md");
    let fixture_pr_path = principles_dir.join("fixture.md");

    assert!(
        fixture_pb_path.is_file(),
        "templates/workspace/playbooks/fixture.md must exist"
    );
    assert!(
        fixture_pr_path.is_file(),
        "templates/workspace/principles/fixture.md must exist"
    );

    let pb = load_playbook(&fixture_pb_path, &principles_dir)
        .expect("fixture playbook must load and validate");
    assert_eq!(pb.name, "fixture");
    assert_eq!(pb.when, "Used for engine tests and verification.");
    assert_eq!(pb.step_ids(), vec![1, 2]);
    assert_eq!(pb.cited_principles, vec!["principle-fixture"]);

    let list_out =
        run_list(&playbooks_dir, &principles_dir).expect("run_list on template fixtures");
    assert!(
        list_out.contains("fixture — Used for engine tests and verification."),
        "list output must contain fixture entry"
    );

    let show_out =
        run_show("fixture", &playbooks_dir, &principles_dir).expect("run_show on fixture");
    assert!(show_out.contains("## Orchestrator"));
    assert!(!show_out.contains("## Worker"));
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI Integration Tests: driving the built binary
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cli_playbook_list_and_show() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let pb_dir = repo_root.join("templates/workspace/playbooks");
    let pr_dir = repo_root.join("templates/workspace/principles");

    let pb_str = pb_dir.to_string_lossy();
    let pr_str = pr_dir.to_string_lossy();

    // 1. Test quaqua playbook list with explicit dirs
    let (code, stdout, stderr) = run_cli(&[
        "playbook",
        "list",
        "--dir",
        &pb_str,
        "--principles-dir",
        &pr_str,
    ]);
    assert_eq!(code, 0, "playbook list failed: {stderr}");
    assert!(
        stdout.contains("fixture — Used for engine tests and verification."),
        "cli stdout: {stdout}"
    );

    // 2. Test quaqua playbook show fixture
    let (code, stdout, stderr) = run_cli(&[
        "playbook",
        "show",
        "fixture",
        "--dir",
        &pb_str,
        "--principles-dir",
        &pr_str,
    ]);
    assert_eq!(code, 0, "playbook show failed: {stderr}");
    assert!(
        stdout.contains("## Orchestrator"),
        "must print Orchestrator section"
    );
    assert!(
        stdout.contains("1. Plan the fixture task."),
        "must print orchestrator step"
    );
    assert!(
        !stdout.contains("## Worker"),
        "must NOT print Worker section"
    );
    assert!(
        !stdout.contains("Execute fixture step"),
        "must NOT print Worker steps"
    );

    // 3. Test quaqua playbook show nonexistent -> exit 1
    let (code, _stdout, stderr) = run_cli(&[
        "playbook",
        "show",
        "nonexistent",
        "--dir",
        &pb_str,
        "--principles-dir",
        &pr_str,
    ]);
    assert_eq!(code, 1, "unknown playbook show must exit 1");
    assert!(
        stderr.contains("playbook 'nonexistent' not found"),
        "stderr should state playbook not found: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Additional Unit Tests: citation extraction and orchestrator-only playbooks
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn playbook_extract_principle_citations_handles_various_punctuation() {
    let text = "Citing principle-first, then `principle-second` and (principle-third). Also [principle-fourth] and principle-prove-it-works.";
    let citations = extract_principle_citations(text);
    assert_eq!(
        citations,
        vec![
            "principle-first",
            "principle-second",
            "principle-third",
            "principle-fourth",
            "principle-prove-it-works",
        ]
    );
}

#[test]
fn orchestrator_only_playbook_parses_without_worker_steps() {
    let sandbox = TempDir::new("orch-only");
    let pr_dir = sandbox.sub("principles");

    let content = r#"---
name: session-pickup
when: Resume an in-flight session from previous artifacts.
---

## Orchestrator

1. Read the prior report.
2. Resume the unit.

## Worker
"#;

    let pb = parse_playbook(content, &pr_dir).expect("orchestrator-only playbook should parse");
    assert_eq!(pb.name, "session-pickup");
    assert!(pb.steps.is_empty());
    assert_eq!(pb.step_ids(), Vec::<u32>::new());
}
