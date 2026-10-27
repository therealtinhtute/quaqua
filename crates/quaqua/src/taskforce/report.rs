//! Council merged report generation and verdict parsing.

use super::workflow::{MemberStatus, TaskforceOutput, TaskforceState};
use std::io;
use std::path::{Path, PathBuf};

/// Parse a member report content string, extracting verdict string, findings count, and challenge status.
pub fn parse_member_verdict(content: &str) -> Result<(String, u32, bool), String> {
    let (findings, is_challenge) = crate::pipeline::parse_report_content(content)?;
    let verdict_str = if is_challenge {
        "challenge".to_string()
    } else if findings == 0 {
        "clean".to_string()
    } else if findings == 1 {
        "1 finding".to_string()
    } else {
        format!("{findings} findings")
    };
    Ok((verdict_str, findings, is_challenge))
}

/// Render the markdown contents of a council merged report.
pub fn render_council_report(
    task: &str,
    role: &str,
    state: &TaskforceState,
    output: &TaskforceOutput,
) -> String {
    let completed_count = output.report_paths.len();
    let quorum_status = if output.quorum_met { "MET" } else { "UNMET" };

    let mut out = String::new();
    out.push_str(&format!("# Council report: {task}\n"));
    out.push_str(&format!(
        "Role: {role} · Quorum: {}/{} ({quorum_status})\n",
        completed_count,
        state.members.len()
    ));
    out.push_str(&format!("Chair: {}\n\n", state.chair_owner));

    out.push_str("## Tally\n");
    if output.tally.is_empty() {
        out.push_str("none\n\n");
    } else {
        for (verdict, ids) in &output.tally {
            out.push_str(&format!(
                "- {verdict}: {} ({})\n",
                ids.len(),
                ids.join(", ")
            ));
        }
        out.push('\n');
    }

    let maj_str = output.majority.as_deref().unwrap_or("none");
    out.push_str(&format!("Majority verdict: {maj_str}\n"));

    let diss_str = if output.dissent.is_empty() {
        "none".to_string()
    } else {
        output.dissent.join(", ")
    };
    out.push_str(&format!("Dissenters: {diss_str}\n\n"));

    out.push_str("## Members\n");
    for m in &state.members {
        match &m.status {
            MemberStatus::Completed {
                report_path,
                verdict,
                ..
            } => {
                let v = verdict.as_deref().unwrap_or("clean");
                out.push_str(&format!("- {}: {v} · {}\n", m.id, report_path.display()));
            }
            MemberStatus::Failed { code, reason } => {
                out.push_str(&format!("- {}: FAILED (code {code}) - {reason}\n", m.id));
            }
            MemberStatus::Started | MemberStatus::Pending => {
                out.push_str(&format!("- {}: NOT RUN\n", m.id));
            }
        }
    }
    out.push_str("\n## Lessons\nnone\n");

    out
}

/// Atomically write the council merged report into `runs_dir/council-{task}.md`.
pub fn write_council_report(
    runs_dir: &Path,
    task: &str,
    role: &str,
    state: &TaskforceState,
    output: &TaskforceOutput,
) -> io::Result<PathBuf> {
    let report_path = runs_dir.join(format!("council-{task}.md"));
    let rendered = render_council_report(task, role, state, output);
    crate::state::atomic_write(&report_path, rendered)?;
    Ok(report_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::child::SingleWorkerInput;
    use crate::taskforce::workflow::TaskforceInput;

    #[test]
    fn test_parse_member_verdict_clean() {
        let content = "# Report\nSome findings text\nVerdict: clean\n## Lessons\nnone\n";
        let (v, f, ch) = parse_member_verdict(content).unwrap();
        assert_eq!(v, "clean");
        assert_eq!(f, 0);
        assert!(!ch);
    }

    #[test]
    fn test_parse_member_verdict_findings() {
        let content = "# Report\nVerdict: 2 findings\n## Lessons\nnone\n";
        let (v, f, ch) = parse_member_verdict(content).unwrap();
        assert_eq!(v, "2 findings");
        assert_eq!(f, 2);
        assert!(!ch);
    }

    #[test]
    fn test_parse_member_verdict_challenge() {
        let content = "# Report\n## Challenge\nPremise invalid.\n## Lessons\nnone\n";
        let (v, _f, ch) = parse_member_verdict(content).unwrap();
        assert_eq!(v, "challenge");
        assert!(ch);
    }

    #[test]
    fn test_render_council_report_majority_and_dissent() {
        let input = TaskforceInput::for_council(
            "t1",
            "repo",
            "brief",
            vec![
                SingleWorkerInput::new("c1", "k1", "repo", "b", 100, None),
                SingleWorkerInput::new("c2", "k2", "repo", "b", 100, None),
                SingleWorkerInput::new("c3", "k3", "repo", "b", 100, None),
            ],
            Some(2),
            Some("council".to_string()),
            Some("cos".to_string()),
        );
        let mut state = TaskforceState::from_input(&input);
        state.members[0].status = MemberStatus::Completed {
            report_path: PathBuf::from("runs/c1.md"),
            verdict: Some("clean".to_string()),
            findings_count: Some(0),
            is_challenge: Some(false),
        };
        state.members[1].status = MemberStatus::Completed {
            report_path: PathBuf::from("runs/c2.md"),
            verdict: Some("clean".to_string()),
            findings_count: Some(0),
            is_challenge: Some(false),
        };
        state.members[2].status = MemberStatus::Completed {
            report_path: PathBuf::from("runs/c3.md"),
            verdict: Some("1 finding".to_string()),
            findings_count: Some(1),
            is_challenge: Some(false),
        };

        let output = super::super::workflow::build_taskforce_output(&state);
        assert!(output.quorum_met);
        assert_eq!(output.majority.as_deref(), Some("clean"));
        assert_eq!(output.dissent, vec!["c3"]);

        let rendered = render_council_report("t1", "council", &state, &output);
        assert!(rendered.contains("# Council report: t1"));
        assert!(rendered.contains("Role: council · Quorum: 3/3 (MET)"));
        assert!(rendered.contains("Chair: cos"));
        assert!(rendered.contains("Majority verdict: clean"));
        assert!(rendered.contains("Dissenters: c3"));
        assert!(rendered.contains("- c1: clean · runs/c1.md"));
        assert!(rendered.contains("- c3: 1 finding · runs/c3.md"));
        assert!(rendered.contains("## Lessons\nnone"));
    }
}
