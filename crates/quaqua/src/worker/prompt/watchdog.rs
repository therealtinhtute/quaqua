//! Report validation and small shared helpers.

/// Sleep or advance the test-only virtual clock.
pub fn nap(sec: u64) {
    if let Ok(sleep_cmd) = std::env::var("PROMPT_WORKER_SLEEP_CMD") {
        let _ = std::process::Command::new(sleep_cmd).arg(sec.to_string()).status();
    } else {
        std::thread::sleep(std::time::Duration::from_secs(sec));
    }
}

/// Prepend a header line to a report unless it is already present. Returns
/// `None` when the line is already there (idempotent stamping).
pub fn prepend_header(report: &str, line: &str) -> Option<String> {
    if report.lines().any(|l| l == line) {
        return None;
    }
    Some(format!("{line}\n{report}"))
}

/// Matches Lessons headings in markdown reports.
/// Accepts exact `## Lessons` as well as numbered headings like `## 5. Lessons`.
pub fn is_lessons_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(after_hash) = trimmed.strip_prefix("##") else {
        return false;
    };
    // Reject deeper headings (###, ####, ...)
    if after_hash.starts_with('#') {
        return false;
    }
    // Must be followed by space or tab
    if !after_hash.starts_with(' ') && !after_hash.starts_with('\t') {
        return false;
    }
    let rest = after_hash.trim_start();
    if let Some(rem) = rest.strip_prefix("Lessons") {
        return rem.is_empty() || !rem.chars().next().unwrap().is_alphabetic();
    }
    // Check for numbered prefix: e.g. "5. Lessons", "5) Lessons", "5 Lessons"
    let digits_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len > 0 {
        let after_digits = &rest[digits_len..];
        let after_punct = after_digits
            .strip_prefix('.')
            .or_else(|| after_digits.strip_prefix(')'))
            .unwrap_or(after_digits);
        let after_num = after_punct.trim_start();
        if let Some(rem) = after_num.strip_prefix("Lessons") {
            return rem.is_empty() || !rem.chars().next().unwrap().is_alphabetic();
        }
    }
    false
}

/// Validates the structure of the finished run report.
pub fn validate_report_content(
    report_content: &str,
    expect_verdict: bool,
) -> Result<(), &'static str> {
    if expect_verdict {
        let has_verdict = report_content
            .lines()
            .any(|l| l.trim_start().starts_with("Verdict:"));
        if !has_verdict {
            return Err("'Verdict:' line");
        }
    } else {
        let has_lessons = report_content.lines().any(|l| is_lessons_heading(l));
        if !has_lessons {
            return Err("'## Lessons' section");
        }
    }
    Ok(())
}
