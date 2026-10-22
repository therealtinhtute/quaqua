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
    extract_principle_citations, load_playbook, load_playbooks, parse_playbook, read_principle,
    resolve_principle_path, run_list, run_show, Playbook, PlaybookError,
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
fn p2a_acceptance_1_fixture_test_loads_all_6_playbooks_and_17_leaves() {
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

    assert!(
        !playbooks_dir.join("fixture.md").exists(),
        "fixture.md must be removed from templates/workspace/playbooks"
    );
    assert!(
        !principles_dir.join("fixture.md").exists(),
        "fixture.md must be removed from templates/workspace/principles"
    );

    let expected_leaves = [
        "attack-the-premise",
        "encode-lessons-in-structure",
        "explain-the-number",
        "fix-root-causes",
        "foundational-thinking",
        "guard-the-context-window",
        "laziness-protocol",
        "make-operations-idempotent",
        "migrate-callers-then-delete-legacy-apis",
        "minimize-reader-load",
        "model-the-domain",
        "never-block-on-the-human",
        "prove-it-works",
        "redesign-from-first-principles",
        "sequence-verifiable-units",
        "subtract-before-you-add",
        "test-behavior-not-implementation",
    ];
    for leaf in expected_leaves {
        let leaf_path = principles_dir.join(format!("{leaf}.md"));
        assert!(leaf_path.is_file(), "missing principle leaf: {leaf}.md");
        let content = read_principle(leaf, &principles_dir)
            .unwrap_or_else(|e| panic!("failed to read principle {leaf}: {e}"));
        assert!(!content.is_empty(), "principle {leaf} content is empty");
    }

    let playbooks = load_playbooks(&playbooks_dir, &principles_dir)
        .expect("all playbooks must load and validate against principles");
    assert_eq!(playbooks.len(), 6, "must load exactly 6 playbooks");
    let names: Vec<&str> = playbooks.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "bug-fix",
            "feature",
            "figure-it-out",
            "multi-phase-plan",
            "perf-issue",
            "refactoring"
        ]
    );
}

#[test]
fn s6_acceptance_2_every_cited_leaf_resolves_none_over_cap() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    let playbooks = load_playbooks(&playbooks_dir, &principles_dir)
        .expect("all 3 playbooks must load and validate");

    for pb in &playbooks {
        for citation in &pb.cited_principles {
            let path = resolve_principle_path(citation, &principles_dir);
            assert!(
                path.is_file(),
                "cited principle '{citation}' in playbook '{}' does not resolve at {}",
                pb.name,
                path.display()
            );
        }
        let non_exempt: Vec<&String> = pb
            .cited_principles
            .iter()
            .filter(|c| c.as_str() != "principle-prove-it-works")
            .collect();
        assert!(
            non_exempt.len() <= 3,
            "playbook '{}' cites {} non-exempt leaves (max 3): {:?}",
            pb.name,
            non_exempt.len(),
            non_exempt
        );
        // Orchestrator citations must also resolve against principles_dir
        let orch_citations = extract_principle_citations(&pb.orchestrator);
        for citation in &orch_citations {
            let path = resolve_principle_path(citation, &principles_dir);
            assert!(
                path.is_file(),
                "cited principle '{citation}' in playbook '{}' Orchestrator does not resolve at {}",
                pb.name,
                path.display()
            );
        }
    }
}

#[test]
fn s6_acceptance_3_every_worker_ends_with_verified_step() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    let playbooks = load_playbooks(&playbooks_dir, &principles_dir)
        .expect("all playbooks must load and validate");

    for pb in &playbooks {
        assert_worker_contract(pb);
    }
}

#[test]
fn s6_acceptance_4_forbidden_terms_grep_clean() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    let forbidden = ["cursor", "task tool", "/loop", "brainstorm", "to-plan"];

    for dir in [&playbooks_dir, &principles_dir] {
        for entry in fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let content = fs::read_to_string(&path).unwrap();
                let lower = content.to_lowercase();
                for term in &forbidden {
                    assert!(
                        !lower.contains(term),
                        "forbidden term '{term}' found in {}",
                        path.display()
                    );
                }
            }
        }
    }
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
        stdout.contains(
            "bug-fix — A reported defect to reproduce, root-cause, and fix with runtime evidence."
        ),
        "cli stdout missing bug-fix: {stdout}"
    );
    assert!(
        stdout.contains("feature — New or changed behavior, built from a named data shape."),
        "cli stdout missing feature: {stdout}"
    );
    assert!(
        stdout.contains("figure-it-out — Large migrations, multi-part changes, or work when no narrower playbook fits."),
        "cli stdout missing figure-it-out: {stdout}"
    );
    assert!(
        !stdout.contains("fixture"),
        "cli stdout must NOT contain fixture: {stdout}"
    );

    // 2. Test quaqua playbook show bug-fix
    let (code, stdout, stderr) = run_cli(&[
        "playbook",
        "show",
        "bug-fix",
        "--dir",
        &pb_str,
        "--principles-dir",
        &pr_str,
    ]);
    assert_eq!(code, 0, "playbook show failed: {stderr}");
    assert!(
        stdout.contains("## Orchestrator"),
        "must print Orchestrator section: {stdout}"
    );
    assert!(
        !stdout.contains("## Worker"),
        "must NOT print Worker section: {stdout}"
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

#[test]
fn f2_default_path_with_no_workspace_dir_errors() {
    let sandbox = TempDir::new("no-ws");
    let empty_home = sandbox.sub("empty-home");

    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .args(&["playbook", "list"])
        .env("QUAQUA_HOME", &empty_home)
        .env_remove("QUAQUA_ROOT")
        .env_remove("QUAQUA_PLAYBOOKS_DIR")
        .env_remove("QUAQUA_PRINCIPLES_DIR")
        .output()
        .expect("failed to execute quaqua binary");

    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(code, 1, "default path with no workspace must exit 1");
    assert!(
        stderr.contains("no quaqua workspace at")
            || stderr.contains("projects/INDEX.md is missing"),
        "stderr should state workspace error: {stderr}"
    );
}

#[test]
fn default_path_missing_playbooks_dir_in_valid_workspace_errors() {
    let sandbox = TempDir::new("ws-no-playbooks");
    let ws = sandbox.sub("ws");
    sandbox.write_file("ws/projects/INDEX.md", "# Projects\n");
    sandbox.write_file("ws/principles/prove-it-works.md", "# Prove it works\n");

    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .args(&["playbook", "list"])
        .env("QUAQUA_HOME", &ws)
        .env_remove("QUAQUA_ROOT")
        .env_remove("QUAQUA_PLAYBOOKS_DIR")
        .env_remove("QUAQUA_PRINCIPLES_DIR")
        .output()
        .expect("failed to execute quaqua binary");

    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(
        code, 1,
        "missing playbooks dir in valid workspace must exit 1"
    );
    assert!(
        stderr.contains("playbooks directory not found"),
        "stderr should state playbooks directory not found: {stderr}"
    );
}

#[test]
fn finding_3_default_path_pins_valid_workspace_and_rejects_non_workspace() {
    let sandbox = TempDir::new("f3-workspace-resolution");

    // 1. Non-workspace directory: has playbooks/ and principles/ dirs, but NO projects/INDEX.md
    let non_ws = sandbox.sub("non-ws");
    sandbox.write_file(
        "non-ws/playbooks/test.md",
        "---\nname: test\nwhen: Test.\n---\n## Orchestrator\nPlan.\n## Worker\n1. Step citing principle-prove-it-works.\n",
    );
    sandbox.write_file("non-ws/principles/prove-it-works.md", "# Prove it works\n");

    let bin = env!("CARGO_BIN_EXE_quaqua");
    let out = Command::new(bin)
        .args(&["playbook", "list"])
        .env("QUAQUA_HOME", &non_ws)
        .env_remove("QUAQUA_ROOT")
        .env_remove("QUAQUA_PLAYBOOKS_DIR")
        .env_remove("QUAQUA_PRINCIPLES_DIR")
        .output()
        .expect("failed to execute quaqua binary");

    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(
        code, 1,
        "non-workspace dir must exit 1 even if playbooks/ exists"
    );
    assert!(
        stderr.contains("no quaqua workspace at")
            || stderr.contains("projects/INDEX.md is missing"),
        "stderr must report workspace error, got: {stderr}"
    );

    // 2. Valid workspace: has projects/INDEX.md, playbooks/, principles/
    let valid_ws = sandbox.sub("valid-ws");
    sandbox.write_file("valid-ws/projects/INDEX.md", "# Projects\n");
    sandbox.write_file(
        "valid-ws/playbooks/sample.md",
        "---\nname: sample\nwhen: Sample playbook.\n---\n## Orchestrator\nPlan.\n## Worker\n1. Step citing principle-prove-it-works.\n",
    );
    sandbox.write_file(
        "valid-ws/principles/prove-it-works.md",
        "# Prove it works\n",
    );

    let out_valid = Command::new(bin)
        .args(&["playbook", "list"])
        .env("QUAQUA_HOME", &valid_ws)
        .env_remove("QUAQUA_ROOT")
        .env_remove("QUAQUA_PLAYBOOKS_DIR")
        .env_remove("QUAQUA_PRINCIPLES_DIR")
        .output()
        .expect("failed to execute quaqua binary");

    let code_valid = out_valid.status.code().unwrap_or(-1);
    let stdout_valid = String::from_utf8_lossy(&out_valid.stdout).to_string();
    let stderr_valid = String::from_utf8_lossy(&out_valid.stderr).to_string();
    assert_eq!(code_valid, 0, "valid workspace must exit 0: {stderr_valid}");
    assert!(stdout_valid.contains("sample — Sample playbook."));
}

#[test]
fn f3_preamble_principle_typo_fails() {
    let sandbox = TempDir::new("f3-typo");
    let pr_dir = sandbox.sub("principles");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    let content = r#"---
name: preamble-typo
when: Testing preamble principle typo.
---

## Orchestrator

1. Plan.

## Worker

Read principle-typo before beginning.

1. Step one citing principle-prove-it-works.
"#;

    let err = parse_playbook(content, &pr_dir).expect_err("preamble typo must fail parser");
    match err {
        PlaybookError::UnknownPrinciple { principle, .. } => {
            assert_eq!(principle, "principle-typo");
        }
        other => panic!("expected UnknownPrinciple, got: {other:?}"),
    }
}

#[test]
fn f3_preamble_four_leaves_fail() {
    let sandbox = TempDir::new("f3-four-leaves");
    let pr_dir = sandbox.sub("principles");
    sandbox.write_file("principles/leaf-a.md", "# Leaf A\n");
    sandbox.write_file("principles/leaf-b.md", "# Leaf B\n");
    sandbox.write_file("principles/leaf-c.md", "# Leaf C\n");
    sandbox.write_file("principles/leaf-d.md", "# Leaf D\n");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    let content = r#"---
name: preamble-four-leaves
when: Testing 4 leaves in preamble fails cap.
---

## Orchestrator

1. Plan.

## Worker

Read principle-leaf-a, principle-leaf-b, principle-leaf-c, and principle-leaf-d.

1. Step one citing principle-prove-it-works.
"#;

    let err = parse_playbook(content, &pr_dir)
        .expect_err("preamble with 4 non-exempt leaves must fail cap");
    match err {
        PlaybookError::LeafCapExceeded { count, max, .. } => {
            assert_eq!(count, 4);
            assert_eq!(max, 3);
        }
        other => panic!("expected LeafCapExceeded, got: {other:?}"),
    }
}

#[test]
fn orchestrator_principle_typo_fails() {
    let sandbox = TempDir::new("orch-typo");
    let pr_dir = sandbox.sub("principles");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    let content = r#"---
name: orch-typo
when: Testing orchestrator principle typo.
---

## Orchestrator

1. Step citing principle-orch-typo.

## Worker

1. Step one citing principle-prove-it-works.
"#;

    let err = parse_playbook(content, &pr_dir).expect_err("orchestrator typo must fail parser");
    match err {
        PlaybookError::UnknownPrinciple { principle, .. } => {
            assert_eq!(principle, "principle-orch-typo");
        }
        other => panic!("expected UnknownPrinciple, got: {other:?}"),
    }
}

/// Allowlist of playbooks permitted to be orchestrator-only per plan § P2.
const ALLOWED_ORCHESTRATOR_ONLY: &[&str] = &["multi-phase-plan", "session-pickup", "pause-safely"];

/// Helper validating that a playbook conforms to the worker contract (§ S6 / § P2).
/// Narrows empty steps skip to explicit allowlist AND empty worker_text per finding 3.
fn assert_worker_contract(pb: &Playbook) {
    if pb.steps.is_empty() {
        assert!(
            ALLOWED_ORCHESTRATOR_ONLY.contains(&pb.name.as_str()),
            "playbook '{}' has empty worker steps but is not in orchestrator-only allowlist",
            pb.name
        );
        assert!(
            pb.worker_text.is_empty(),
            "orchestrator-only playbook '{}' must have empty worker_text, but has: '{}'",
            pb.name,
            pb.worker_text
        );
        return;
    }
    let last_step = pb.steps.last().unwrap();
    assert!(
        last_step
            .text
            .contains("Check every Evidence line, then write `Verified: <level> <sha>`")
            || last_step
                .text
                .contains("check every Evidence line, then write `Verified: <level> <sha>`"),
        "playbook '{}' last step text does not end with required Verified step text: '{}'",
        pb.name,
        last_step.text
    );
    assert!(
        last_step
            .cited_principles
            .contains(&"principle-prove-it-works".to_string()),
        "playbook '{}' last step must cite principle-prove-it-works",
        pb.name
    );
}

#[test]
fn finding_1_multi_phase_plan_orchestrator_skeleton_intact() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    let output = run_show("multi-phase-plan", &playbooks_dir, &principles_dir)
        .expect("run_show multi-phase-plan should succeed");
    assert!(
        output.contains("## How to read this"),
        "multi-phase-plan Orchestrator output must contain '## How to read this'"
    );
    assert!(
        output.contains("## Program checklist"),
        "multi-phase-plan Orchestrator output must contain '## Program checklist'"
    );
    assert!(
        output.ends_with("```\n") || output.ends_with("```"),
        "multi-phase-plan Orchestrator output must end with closing code fence"
    );
}

#[test]
fn finding_2_perf_issue_steps_have_no_duplicate_ids() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let playbooks_dir = repo_root.join("templates/workspace/playbooks");
    let principles_dir = repo_root.join("templates/workspace/principles");

    let pb = load_playbook(&playbooks_dir.join("perf-issue.md"), &principles_dir)
        .expect("perf-issue should parse");
    assert_eq!(
        pb.step_ids(),
        vec![1, 2, 3, 4, 5],
        "perf-issue step_ids must be exactly [1, 2, 3, 4, 5] without duplicate ids from mantras"
    );
    assert_eq!(pb.steps.len(), 5, "perf-issue must have exactly 5 steps");
}

#[test]
#[should_panic(expected = "not in orchestrator-only allowlist")]
fn finding_3_unallowlisted_empty_worker_rejected() {
    let sandbox = TempDir::new("f3-allowlist");
    let pr_dir = sandbox.sub("principles");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    let content = r#"---
name: unknown-empty
when: Testing unknown empty worker.
---

## Orchestrator

1. Plan.

## Worker
"#;
    let pb = parse_playbook(content, &pr_dir).expect("should parse");
    assert_worker_contract(&pb);
}

#[test]
#[should_panic(expected = "must have empty worker_text")]
fn finding_3_prose_worker_rejected() {
    let sandbox = TempDir::new("f3-prose");
    let pr_dir = sandbox.sub("principles");
    sandbox.write_file("principles/prove-it-works.md", "# Prove it works\n");

    let content = r#"---
name: multi-phase-plan
when: Testing prose worker.
---

## Orchestrator

1. Plan.

## Worker

Prose without numbered steps.
"#;
    let pb = parse_playbook(content, &pr_dir).expect("should parse");
    assert_worker_contract(&pb);
}
