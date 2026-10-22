//! Warn-only check of an accepted builder report's `## Playbook` block (§ S5).
//!
//! A builder prompt pins the playbook name and the step ids the worker actually
//! received in `runs/.<name>.playbook` (§ S4, written by `brief::pin_playbook`
//! from both `worker prompt` and `quaqua pipeline`). A builder report must
//! account for every pinned id, either as `<id>: done` or as
//! `<id>: skip: <reason>`; a step its harness cannot run is reported as
//! `<id>: skip: unsupported on <harness>, <reason>` and counts as a valid skip.
//!
//! A gap warns and journals one `playbook_gap {name, missing, unreasoned}`
//! event. It never changes an exit code, and a journal failure only warns: the
//! report is already accepted by the time this runs.

use std::collections::HashMap;
use std::path::Path;

use crate::engine::journal::{EventRecord, Journal};
use crate::engine::journal_sqlite::SqliteJournal;
use crate::util::now_sec;

/// The pinned playbook: the name and the step ids the worker was given.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pin {
    name: String,
    ids: Vec<u32>,
}

/// How one report line accounts for a step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatus {
    Done,
    /// `skip: <reason>`; `None` when no reason followed the `skip:`.
    Skip(Option<String>),
}

/// Pinned steps the report does not account for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybookGap {
    /// Playbook name from the pin.
    pub name: String,
    /// Pinned ids with no `done` and no reasoned `skip` (or no report line).
    pub missing: Vec<u32>,
    /// Pinned ids reported as `skip:` without a reason.
    pub unreasoned: Vec<u32>,
}

impl PlaybookGap {
    fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unreasoned.is_empty()
    }
}

/// § S5: check an accepted builder report against `runs/.<name>.playbook`.
///
/// Returns without a word when there is no pin (the worker was never given a
/// playbook, or the pin write failed after the prompt landed), when the report
/// accounts for every pinned step, and for reviewer reports. Otherwise prints
/// one `warning: playbook gap` line and appends a `playbook_gap` event; the
/// caller's exit code is untouched.
pub fn warn_playbook_gaps(
    journal: &mut SqliteJournal,
    runs_dir: &Path,
    name: &str,
    report: &str,
    expect_verdict: bool,
) {
    // Reviewer reports carry a `Verdict:` line, not a `## Playbook` block, and
    // `worker prompt` never injects a playbook into them.
    if expect_verdict {
        return;
    }
    let Some(pin) = read_pin(&runs_dir.join(format!(".{name}.playbook"))) else {
        return;
    };
    let gap = gap(report, &pin);
    if gap.is_empty() {
        return;
    }
    eprintln!(
        "warning: playbook gap | name={name} playbook={} missing={} unreasoned={}",
        gap.name,
        id_list(&gap.missing),
        id_list(&gap.unreasoned)
    );
    let payload = serde_json::json!({
        "name": gap.name,
        "missing": gap.missing,
        "unreasoned": gap.unreasoned,
    })
    .to_string();
    let event = EventRecord::new(0, now_sec().saturating_mul(1000), "playbook_gap", payload);
    if let Err(e) = journal.append_event(&format!("worker:{name}"), &event) {
        eprintln!("warning: cannot record playbook_gap in the journal: {e}");
    }
}

fn id_list(ids: &[u32]) -> String {
    if ids.is_empty() {
        "none".to_string()
    } else {
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// `runs/.<name>.playbook`, written by `brief::pin_playbook`: `name=<n>\nids=1,2,3`.
/// An absent, unreadable or id-less file means the worker received no steps, so
/// there is nothing to check.
fn read_pin(path: &Path) -> Option<Pin> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut ids: Vec<u32> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name=") {
            name = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("ids=") {
            ids = value
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .collect();
            ids.sort_unstable();
            ids.dedup();
        }
    }
    if ids.is_empty() {
        return None;
    }
    Some(Pin { name, ids })
}

/// The gap of one report against one pin.
fn gap(report: &str, pin: &Pin) -> PlaybookGap {
    let mut seen: HashMap<u32, StepStatus> = HashMap::new();
    if let Some(lines) = playbook_block_lines(report) {
        for line in lines {
            if let Some((id, status)) = parse_step_line(line) {
                seen.insert(id, status);
            }
        }
    }
    let mut missing = Vec::new();
    let mut unreasoned = Vec::new();
    for id in &pin.ids {
        match seen.get(id) {
            Some(StepStatus::Done) | Some(StepStatus::Skip(Some(_))) => {}
            Some(StepStatus::Skip(None)) => unreasoned.push(*id),
            None => missing.push(*id),
        }
    }
    PlaybookGap {
        name: pin.name.clone(),
        missing,
        unreasoned,
    }
}

/// Lines of the report's `## Playbook` section, or `None` when it has none.
/// The section ends at the next level-1/2 heading; deeper headings stay inside.
fn playbook_block_lines(report: &str) -> Option<Vec<&str>> {
    let mut inside = false;
    let mut found = false;
    let mut lines = Vec::new();
    for line in report.lines() {
        if let Some((level, text)) = heading(line) {
            if level <= 2 {
                if found && inside {
                    break;
                }
                inside = level == 2 && text.eq_ignore_ascii_case("playbook");
                found |= inside;
                continue;
            }
        }
        if inside {
            lines.push(line);
        }
    }
    if found {
        Some(lines)
    } else {
        None
    }
}

/// `(level, text)` for a markdown heading line.
fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    Some((level, rest.trim()))
}

/// One step line in the four accepted shapes: `1: done`, `1. done`, `1) done`
/// and the same after a markdown bullet (`- 1: done`).
fn parse_step_line(line: &str) -> Option<(u32, StepStatus)> {
    let mut rest = line.trim();
    for bullet in ["- ", "* ", "+ "] {
        if let Some(after) = rest.strip_prefix(bullet) {
            rest = after.trim_start();
            break;
        }
    }
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let id: u32 = rest[..digits].parse().ok()?;
    let rest = rest[digits..].trim_start();
    let body = rest
        .strip_prefix(':')
        .or_else(|| rest.strip_prefix('.'))
        .or_else(|| rest.strip_prefix(')'))?
        .trim();
    Some((id, step_status(body)?))
}

fn step_status(body: &str) -> Option<StepStatus> {
    if starts_with_word(body, "done") {
        return Some(StepStatus::Done);
    }
    if starts_with_word(body, "skip") {
        let rest = body["skip".len()..].trim_start();
        let reason = rest.strip_prefix(':').unwrap_or(rest).trim();
        return Some(StepStatus::Skip(if reason.is_empty() {
            None
        } else {
            Some(reason.to_string())
        }));
    }
    None
}

/// `done`, `done.`, `done (abc1234)` match; `doneish` does not.
fn starts_with_word(s: &str, word: &str) -> bool {
    match s.get(..word.len()) {
        Some(prefix) if prefix.eq_ignore_ascii_case(word) => s[word.len()..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Helper RAII temporary directory for test isolation.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "quaqua-playbook-check-{tag}-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pin(name: &str, ids: &[u32]) -> Pin {
        Pin {
            name: name.to_string(),
            ids: ids.to_vec(),
        }
    }

    fn report(block: &str) -> String {
        format!("# Report\n\n## Changes\n\nsomething\n\n{block}\n## Lessons\nnone\n")
    }

    fn events(journal: &SqliteJournal, name: &str) -> Vec<EventRecord> {
        journal
            .load_events(&format!("worker:{name}"), 1)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.kind == "playbook_gap")
            .collect()
    }

    fn payload(event: &EventRecord) -> serde_json::Value {
        serde_json::from_str(&event.payload_json).unwrap()
    }

    #[test]
    fn playbook_check_parses_the_four_line_formats() {
        let report =
            report("## Playbook\n\n1: done\n2. done\n3) done\n- 4: done\n- 5. done\n- 6) done\n");
        assert_eq!(
            gap(&report, &pin("feature", &[1, 2, 3, 4, 5, 6])),
            PlaybookGap {
                name: "feature".into(),
                missing: vec![],
                unreasoned: vec![],
            }
        );
        // `done` may carry a note, and only the `## Playbook` block counts.
        let noted = "# Report\n\n1: done\n\n## Playbook\n7: done (cafe1234)\n\n## Lessons\nnone\n";
        assert!(gap(noted, &pin("feature", &[7])).is_empty());
        assert_eq!(
            gap(noted, &pin("feature", &[1])).missing,
            vec![1],
            "a step line outside `## Playbook` is not counted"
        );
    }

    #[test]
    fn playbook_check_missing_section_warns_and_records_the_event() {
        let tmp = TempDir::new("pbgap-section");
        let runs = tmp.path();
        let mut journal = SqliteJournal::open(&runs.join("quaqua.db")).unwrap();
        fs::write(runs.join(".sude-s5a.playbook"), "name=feature\nids=1,2,3\n").unwrap();

        warn_playbook_gaps(
            &mut journal,
            runs,
            "sude-s5a",
            "# Report\n\n## Lessons\nnone\n",
            false,
        );

        let recorded = events(&journal, "sude-s5a");
        assert_eq!(recorded.len(), 1, "one gap event");
        let p = payload(&recorded[0]);
        assert_eq!(p["name"], "feature");
        assert_eq!(p["missing"], serde_json::json!([1, 2, 3]));
        assert_eq!(p["unreasoned"], serde_json::json!([]));
    }

    #[test]
    fn playbook_check_missing_step_and_unreasoned_skip_are_separate() {
        let tmp = TempDir::new("pbgap-steps");
        let runs = tmp.path();
        let mut journal = SqliteJournal::open(&runs.join("quaqua.db")).unwrap();
        fs::write(runs.join(".sude-s5b.playbook"), "name=feature\nids=1,2,3\n").unwrap();

        warn_playbook_gaps(
            &mut journal,
            runs,
            "sude-s5b",
            &report("## Playbook\n1: done\n3: skip:\n"),
            false,
        );

        let recorded = events(&journal, "sude-s5b");
        assert_eq!(recorded.len(), 1);
        let p = payload(&recorded[0]);
        assert_eq!(p["missing"], serde_json::json!([2]));
        assert_eq!(p["unreasoned"], serde_json::json!([3]));
    }

    #[test]
    fn playbook_check_accepts_unsupported_harness_skip() {
        let block = report(
            "## Playbook\n1: done\n2: skip: unsupported on omp, cannot open a browser\n3: skip: unsupported on claude\n",
        );
        assert_eq!(
            gap(&block, &pin("feature", &[1, 2, 3])),
            PlaybookGap {
                name: "feature".into(),
                missing: vec![],
                unreasoned: vec![],
            },
            "an unsupported-on-harness skip is a valid skip"
        );
        // A bare `skip:` stays unreasoned even though it names no status.
        assert_eq!(
            gap(&report("## Playbook\n1: skip: \n"), &pin("feature", &[1])).unreasoned,
            vec![1]
        );
        // An unrecognized status is not a `done` and not a `skip`.
        assert_eq!(
            gap(&report("## Playbook\n1: partial\n"), &pin("feature", &[1])).missing,
            vec![1]
        );
    }

    #[test]
    fn playbook_check_without_pin_is_silent() {
        let tmp = TempDir::new("pbgap-nopin");
        let runs = tmp.path();
        let mut journal = SqliteJournal::open(&runs.join("quaqua.db")).unwrap();

        warn_playbook_gaps(
            &mut journal,
            runs,
            "sude-s5c",
            "# Report\n\n## Lessons\nnone\n",
            false,
        );

        assert!(events(&journal, "sude-s5c").is_empty(), "no pin, no event");
        // An id-less pin is a pin the worker never got: also no check.
        fs::write(runs.join(".sude-s5c.playbook"), "name=feature\nids=\n").unwrap();
        warn_playbook_gaps(
            &mut journal,
            runs,
            "sude-s5c",
            "# Report\n\n## Lessons\nnone\n",
            false,
        );
        assert!(events(&journal, "sude-s5c").is_empty());
    }

    #[test]
    fn playbook_check_skips_reviewer_reports() {
        let tmp = TempDir::new("pbgap-reviewer");
        let runs = tmp.path();
        let mut journal = SqliteJournal::open(&runs.join("quaqua.db")).unwrap();
        fs::write(
            runs.join(".suhuynh-s5d.playbook"),
            "name=feature\nids=1,2\n",
        )
        .unwrap();

        warn_playbook_gaps(
            &mut journal,
            runs,
            "suhuynh-s5d",
            "# Review\n\nVerdict: clean 1 findings\n\n## Lessons\nnone\n",
            true,
        );

        assert!(events(&journal, "suhuynh-s5d").is_empty());
    }

    #[test]
    fn playbook_check_complete_report_is_silent() {
        let tmp = TempDir::new("pbgap-clean");
        let runs = tmp.path();
        let mut journal = SqliteJournal::open(&runs.join("quaqua.db")).unwrap();
        fs::write(runs.join(".sude-s5e.playbook"), "name=feature\nids=1,2\n").unwrap();

        warn_playbook_gaps(
            &mut journal,
            runs,
            "sude-s5e",
            &report("## Playbook\n1: done\n2: skip: unsupported on omp, no browser\n"),
            false,
        );
        assert!(events(&journal, "sude-s5e").is_empty());
    }
}
