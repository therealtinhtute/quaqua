//! Pure gate decision logic for pipeline build-to-review transitions.
//!
//! Evaluates builder outcome facts against review strategy, git commit presence,
//! and diff thresholds without any side effects or I/O.

use serde::{Deserialize, Serialize};

pub use crate::pipeline::state::ReviewStrategy;

/// Result of the build gate check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildGate {
    /// Builder produced no commits and did not declare 'Outcome: no changes'.
    /// Pipeline should pause/block with the given message.
    Blocked(String),
    /// Builder produced no commits and declared 'Outcome: no changes'.
    /// Pipeline completes clean without review.
    NoChanges,
    /// Commits produced, but review is skipped per strategy or auto thresholds.
    Skip(String),
    /// Commits produced, review should run.
    Review,
}

impl BuildGate {
    /// Returns the wire gate identifier sent to child output and journals.
    pub fn wire_gate(&self) -> &'static str {
        match self {
            BuildGate::NoChanges => "no_changes",
            BuildGate::Skip(_) => "skip",
            BuildGate::Review => "review",
            BuildGate::Blocked(_) => "blocked",
        }
    }

    /// Returns the pipeline log (pfile) entry for this outcome, if any.
    pub fn pfile_line(&self) -> Option<String> {
        match self {
            BuildGate::NoChanges => Some(
                "- build: no commits — builder declared 'Outcome: no changes'; nothing to review\n"
                    .to_string(),
            ),
            BuildGate::Skip(reason) => Some(format!("- review: skipped ({reason})\n")),
            _ => None,
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, BuildGate::Blocked(_))
    }

    pub fn is_no_changes(&self) -> bool {
        matches!(self, BuildGate::NoChanges)
    }

    pub fn is_skip(&self) -> bool {
        matches!(self, BuildGate::Skip(_))
    }

    pub fn is_review(&self) -> bool {
        matches!(self, BuildGate::Review)
    }

    pub fn skip_reason(&self) -> Option<&str> {
        match self {
            BuildGate::Skip(reason) => Some(reason.as_str()),
            _ => None,
        }
    }

    pub fn blocked_message(&self) -> Option<&str> {
        match self {
            BuildGate::Blocked(msg) => Some(msg.as_str()),
            _ => None,
        }
    }
}

/// Observed facts gathered from git and build reports needed to decide the build gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFacts {
    /// HEAD commit sha of the builder worktree.
    pub head: String,
    /// Base commit sha the builder started from.
    pub base_sha: String,
    /// Whether the builder report declared 'Outcome: no changes'.
    pub declared_no_changes: bool,
    /// Configured review strategy (auto, always, skip).
    pub strategy: ReviewStrategy,
    /// Total added + deleted lines from diff against base_sha.
    pub changed_lines: usize,
    /// Number of distinct top-level directories touched.
    pub top_dirs: usize,
    /// Line threshold for auto review (runs review if changed_lines > review_lines).
    pub review_lines: usize,

    // Context facts read by the existing code (mod.rs:1511-1514) for the Blocked error message:
    /// Builder worker name (e.g. "sude").
    pub builder: String,
    /// Task slug/identifier.
    pub task: String,
    /// Repository name.
    pub repo: String,
}

impl BuildFacts {
    pub fn new(
        head: impl Into<String>,
        base_sha: impl Into<String>,
        declared_no_changes: bool,
        strategy: ReviewStrategy,
        changed_lines: usize,
        top_dirs: usize,
        review_lines: usize,
    ) -> Self {
        Self {
            head: head.into(),
            base_sha: base_sha.into(),
            declared_no_changes,
            strategy,
            changed_lines,
            top_dirs,
            review_lines,
            builder: String::new(),
            task: String::new(),
            repo: String::new(),
        }
    }

    pub fn with_context(
        mut self,
        builder: impl Into<String>,
        task: impl Into<String>,
        repo: impl Into<String>,
    ) -> Self {
        self.builder = builder.into();
        self.task = task.into();
        self.repo = repo.into();
        self
    }
}

impl Default for BuildFacts {
    fn default() -> Self {
        Self {
            head: String::new(),
            base_sha: String::new(),
            declared_no_changes: false,
            strategy: ReviewStrategy::Auto,
            changed_lines: 0,
            top_dirs: 0,
            review_lines: 50,
            builder: String::new(),
            task: String::new(),
            repo: String::new(),
        }
    }
}

/// Pure decision function for the build gate.
///
/// Rules:
/// - `head == base_sha && declared_no_changes` -> `BuildGate::NoChanges`
/// - `head == base_sha && !declared_no_changes` -> `BuildGate::Blocked(msg)`
/// - `head != base_sha`:
///   - `strategy == ReviewStrategy::Always` -> `BuildGate::Review`
///   - `strategy == ReviewStrategy::Skip` -> `BuildGate::Skip("gate: skip")`
///   - `strategy == ReviewStrategy::Auto`:
///     - `changed_lines > review_lines || top_dirs > 1` -> `BuildGate::Review`
///     - otherwise -> `BuildGate::Skip("gate: auto")`
pub fn decide_build_gate(facts: &BuildFacts) -> BuildGate {
    if facts.head == facts.base_sha {
        if facts.declared_no_changes {
            BuildGate::NoChanges
        } else {
            let base7 = if facts.base_sha.len() >= 7 {
                &facts.base_sha[..7]
            } else {
                &facts.base_sha
            };
            let msg = format!(
                "builder produced no commits on quaqua/{}-{} (HEAD == base {base7}) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then 'quaqua pipeline --resume {} {}'",
                facts.builder, facts.task, facts.task, facts.repo
            );
            BuildGate::Blocked(msg)
        }
    } else {
        match facts.strategy {
            ReviewStrategy::Always => BuildGate::Review,
            ReviewStrategy::Skip => BuildGate::Skip("gate: skip".to_string()),
            ReviewStrategy::Auto => {
                if facts.changed_lines > facts.review_lines || facts.top_dirs > 1 {
                    BuildGate::Review
                } else {
                    BuildGate::Skip("gate: auto".to_string())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decide_build_gate_table() {
        struct TestCase {
            name: &'static str,
            head: &'static str,
            base_sha: &'static str,
            declared_no_changes: bool,
            strategy: ReviewStrategy,
            changed_lines: usize,
            top_dirs: usize,
            review_lines: usize,
            expected: BuildGate,
        }

        let cases = vec![
            // 1. head == base + decl -> NoChanges
            TestCase {
                name: "head == base + decl -> NoChanges",
                head: "abc1234",
                base_sha: "abc1234",
                declared_no_changes: true,
                strategy: ReviewStrategy::Auto,
                changed_lines: 0,
                top_dirs: 0,
                review_lines: 50,
                expected: BuildGate::NoChanges,
            },
            // 2. head == base - decl -> Blocked
            TestCase {
                name: "head == base - decl -> Blocked",
                head: "abc1234def5678",
                base_sha: "abc1234def5678",
                declared_no_changes: false,
                strategy: ReviewStrategy::Auto,
                changed_lines: 0,
                top_dirs: 0,
                review_lines: 50,
                expected: BuildGate::Blocked(
                    "builder produced no commits on quaqua/sude-task1 (HEAD == base abc1234) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then 'quaqua pipeline --resume task1 myrepo'".to_string(),
                ),
            },
            // 3. head != base + decl + always -> Review
            TestCase {
                name: "head != base + decl + always -> Review",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: true,
                strategy: ReviewStrategy::Always,
                changed_lines: 10,
                top_dirs: 1,
                review_lines: 50,
                expected: BuildGate::Review,
            },
            // 4. auto small -> Skip
            TestCase {
                name: "auto small -> Skip",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: false,
                strategy: ReviewStrategy::Auto,
                changed_lines: 20,
                top_dirs: 1,
                review_lines: 50,
                expected: BuildGate::Skip("gate: auto".to_string()),
            },
            // 5. skip -> Skip
            TestCase {
                name: "skip -> Skip",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: false,
                strategy: ReviewStrategy::Skip,
                changed_lines: 100,
                top_dirs: 3,
                review_lines: 50,
                expected: BuildGate::Skip("gate: skip".to_string()),
            },
            // 6. auto lines exceeded -> Review
            TestCase {
                name: "auto lines exceeded -> Review",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: false,
                strategy: ReviewStrategy::Auto,
                changed_lines: 51,
                top_dirs: 1,
                review_lines: 50,
                expected: BuildGate::Review,
            },
            // 7. auto top_dirs exceeded -> Review
            TestCase {
                name: "auto top_dirs exceeded -> Review",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: false,
                strategy: ReviewStrategy::Auto,
                changed_lines: 10,
                top_dirs: 2,
                review_lines: 50,
                expected: BuildGate::Review,
            },
            // 8. auto both thresholds exceeded -> Review
            TestCase {
                name: "auto both thresholds exceeded -> Review",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: false,
                strategy: ReviewStrategy::Auto,
                changed_lines: 120,
                top_dirs: 4,
                review_lines: 50,
                expected: BuildGate::Review,
            },
            // 9. head != base + decl + auto small -> Skip
            TestCase {
                name: "head != base + decl + auto small -> Skip",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: true,
                strategy: ReviewStrategy::Auto,
                changed_lines: 5,
                top_dirs: 1,
                review_lines: 50,
                expected: BuildGate::Skip("gate: auto".to_string()),
            },
            // 10. head != base + decl + auto large -> Review
            TestCase {
                name: "head != base + decl + auto large -> Review",
                head: "def5678",
                base_sha: "abc1234",
                declared_no_changes: true,
                strategy: ReviewStrategy::Auto,
                changed_lines: 80,
                top_dirs: 1,
                review_lines: 50,
                expected: BuildGate::Review,
            },
            // 11. head == base + decl + strategy Always -> still NoChanges (no commits to review)
            TestCase {
                name: "head == base + decl + strategy Always -> NoChanges",
                head: "abc1234",
                base_sha: "abc1234",
                declared_no_changes: true,
                strategy: ReviewStrategy::Always,
                changed_lines: 0,
                top_dirs: 0,
                review_lines: 50,
                expected: BuildGate::NoChanges,
            },
            // 12. head == base - decl + strategy Skip -> still Blocked
            TestCase {
                name: "head == base - decl + strategy Skip -> Blocked",
                head: "short",
                base_sha: "short",
                declared_no_changes: false,
                strategy: ReviewStrategy::Skip,
                changed_lines: 0,
                top_dirs: 0,
                review_lines: 50,
                expected: BuildGate::Blocked(
                    "builder produced no commits on quaqua/sude-task1 (HEAD == base short) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then 'quaqua pipeline --resume task1 myrepo'".to_string(),
                ),
            },
        ];

        for case in cases {
            let facts = BuildFacts {
                head: case.head.to_string(),
                base_sha: case.base_sha.to_string(),
                declared_no_changes: case.declared_no_changes,
                strategy: case.strategy,
                changed_lines: case.changed_lines,
                top_dirs: case.top_dirs,
                review_lines: case.review_lines,
                builder: "sude".to_string(),
                task: "task1".to_string(),
                repo: "myrepo".to_string(),
            };

            let outcome = decide_build_gate(&facts);
            assert_eq!(outcome, case.expected, "Case failed: {}", case.name);
        }
    }

    #[test]
    fn test_build_gate_wire_and_pfile_line() {
        assert_eq!(BuildGate::NoChanges.wire_gate(), "no_changes");
        assert_eq!(BuildGate::Review.wire_gate(), "review");
        assert_eq!(
            BuildGate::Skip("gate: auto".into()).wire_gate(),
            "skip"
        );
        assert_eq!(
            BuildGate::Blocked("error".into()).wire_gate(),
            "blocked"
        );

        assert_eq!(
            BuildGate::NoChanges.pfile_line(),
            Some(
                "- build: no commits — builder declared 'Outcome: no changes'; nothing to review\n"
                    .to_string()
            )
        );
        assert_eq!(
            BuildGate::Skip("gate: auto".into()).pfile_line(),
            Some("- review: skipped (gate: auto)\n".to_string())
        );
        assert_eq!(
            BuildGate::Skip("gate: skip".into()).pfile_line(),
            Some("- review: skipped (gate: skip)\n".to_string())
        );
        assert_eq!(BuildGate::Review.pfile_line(), None);
        assert_eq!(BuildGate::Blocked("msg".into()).pfile_line(), None);
    }

    #[test]
    fn test_build_facts_builder_helpers() {
        let facts = BuildFacts::new(
            "head123",
            "base123",
            true,
            ReviewStrategy::Auto,
            15,
            1,
            50,
        )
        .with_context("detu", "fix-bug", "quaqua");

        assert_eq!(facts.head, "head123");
        assert_eq!(facts.base_sha, "base123");
        assert!(facts.declared_no_changes);
        assert_eq!(facts.strategy, ReviewStrategy::Auto);
        assert_eq!(facts.changed_lines, 15);
        assert_eq!(facts.top_dirs, 1);
        assert_eq!(facts.review_lines, 50);
        assert_eq!(facts.builder, "detu");
        assert_eq!(facts.task, "fix-bug");
        assert_eq!(facts.repo, "quaqua");
    }
}
