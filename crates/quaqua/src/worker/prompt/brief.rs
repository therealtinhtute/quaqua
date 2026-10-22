//! Brief contract for `worker prompt` (§ S4): the advisory `Playbook:` and
//! `Context:` headers, the playbook injection appended to the submitted prompt,
//! and the `runs/.<name>.playbook` pin (name + step ids).
//!
//! Everything here warns except one case: a named playbook missing from a
//! resolvable index is rejected, because the worker would otherwise get a
//! prompt that names a playbook whose steps were never delivered.

use std::path::Path;

use crate::playbook::{
    find_playbook, read_principle, resolve_playbooks_dir, resolve_principles_dir, Playbook,
    PlaybookError,
};

/// What the brief's `Playbook:` header says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybookHeader {
    /// No `Playbook:` line at all.
    Missing,
    /// `Playbook: none[, <reason>]` — the unit runs without a playbook.
    None { reason: String },
    /// `Playbook: <name>`.
    Named(String),
}

/// Value of an advisory brief header (`Playbook`, `Context`), matched at the
/// start of a line so prose that mentions the header does not count.
pub fn header_value<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    for line in text.lines() {
        let Some((key, rest)) = line.trim().split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(rest.trim());
        }
    }
    None
}

/// `Some(reason)` when the value names `none`; the reason is empty for a bare
/// `none` and for `none,` with nothing after it.
fn none_reason(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("none")?;
    if rest.is_empty() {
        return Some(String::new());
    }
    let separator = rest
        .chars()
        .next()
        .is_some_and(|c| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '—' | '('));
    if !separator {
        // A name that merely starts with `none`, e.g. `nonexistent`.
        return None;
    }
    let reason = trimmed["none".len()..]
        .trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '—' | '(')
        })
        .trim();
    Some(reason.to_string())
}

/// Parse the brief's `Playbook:` header.
pub fn playbook_header(text: &str) -> PlaybookHeader {
    let Some(value) = header_value(text, "Playbook") else {
        return PlaybookHeader::Missing;
    };
    if let Some(reason) = none_reason(value) {
        return PlaybookHeader::None { reason };
    }
    match value.split_whitespace().next() {
        Some(name) => PlaybookHeader::Named(name.trim_end_matches([',', ';', ':']).to_string()),
        None => PlaybookHeader::None {
            reason: String::new(),
        },
    }
}

/// The playbook named by the brief, if the header names one (§ S4: the engine
/// never classifies; it only resolves what the coordinator wrote).
pub fn named_playbook(text: &str) -> Option<String> {
    match playbook_header(text) {
        PlaybookHeader::Named(name) => Some(name),
        _ => None,
    }
}

/// Warnings for the two advisory headers. Empty when both are present and a
/// `none` carries a reason. Never an error: briefs without them pass as before.
pub fn brief_header_warnings(text: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    match playbook_header(text) {
        PlaybookHeader::Missing => warnings.push(
            "warning: brief has no 'Playbook:' header — add 'Playbook: <name>' from `quaqua playbook list`, or 'Playbook: none, <reason>' for a trivial one-command unit (rules/_all/brief.md)".to_string(),
        ),
        PlaybookHeader::None { reason } if reason.is_empty() => warnings.push(
            "warning: 'Playbook: none' needs a reason: 'Playbook: none, <why this unit has no playbook>'".to_string(),
        ),
        _ => {}
    }
    match header_value(text, "Context") {
        None => warnings.push(
            "warning: brief has no 'Context:' header — name the files/PRs, or paste the upstream report in full (rules/_all/brief.md)".to_string(),
        ),
        Some(value) if value.is_empty() => warnings.push(
            "warning: 'Context:' is empty; name the files/PRs, or paste the upstream report in full".to_string(),
        ),
        Some(value) if none_reason(value).is_some_and(|r| r.is_empty()) => warnings.push(
            "warning: 'Context: none' needs a reason: 'Context: none, <why nothing upstream matters>'"
                .to_string(),
        ),
        Some(_) => {}
    }
    warnings
}

/// The text appended to a submitted prompt for one playbook: the `## Worker`
/// steps, each cited leaf once, and the pre-filled `## Playbook` block. The
/// `## Orchestrator` text is never included — that is the coordinator's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybookInjection {
    /// Playbook name from the file's frontmatter.
    pub name: String,
    /// Step ids the worker was given, in order.
    pub ids: Vec<u32>,
    /// The block appended to the brief, starting with a newline.
    pub text: String,
}

impl PlaybookInjection {
    /// Content of `runs/.<name>.playbook`: the name and the step ids, so a
    /// playbook edited between prompt and report still checks against the ids
    /// the worker actually received.
    pub fn state_content(&self) -> String {
        let ids: Vec<String> = self.ids.iter().map(|id| id.to_string()).collect();
        format!("name={}\nids={}\n", self.name, ids.join(","))
    }
}

/// Render the injection for an already-parsed playbook, reading each cited
/// leaf once from `principles_dir`.
pub fn render_injection(
    playbook: &Playbook,
    principles_dir: &Path,
) -> Result<PlaybookInjection, PlaybookError> {
    let ids = playbook.step_ids();
    let mut text = format!(
        "\n## Worker (playbook: {})\n\nReport every step in your report's `## Playbook` block: `<id>: done`, or `<id>: skip: <reason>`.\n",
        playbook.name
    );
    for step in &playbook.steps {
        text.push_str(&format!("\n{}. {}", step.id, step.text));
    }
    text.push('\n');
    for citation in &playbook.cited_principles {
        let leaf = read_principle(citation, principles_dir)?;
        text.push_str(&format!("\n### {}\n\n{}\n", citation, leaf.trim()));
    }
    text.push_str("\n## Playbook\n");
    for id in &ids {
        text.push_str(&format!("{id}: \n"));
    }
    Ok(PlaybookInjection {
        name: playbook.name.clone(),
        ids,
        text,
    })
}

/// Resolve the brief's playbook in an explicit index and build the injection.
/// `None` when the brief names no playbook. An unknown name is rejected with
/// the brief-error exit code; a broken or unreadable index is not the brief's
/// fault and is left to [`prepare_injection_in_workspace`] to downgrade.
pub fn prepare_injection(
    task: &str,
    playbooks_dir: &Path,
    principles_dir: &Path,
) -> Result<Option<PlaybookInjection>, (i32, String)> {
    let Some(name) = named_playbook(task) else {
        return Ok(None);
    };
    let playbook = find_playbook(&name, playbooks_dir, principles_dir).map_err(|e| match e {
        PlaybookError::NotFound(_) => (
            1,
            format!("error: unknown playbook '{name}' (run `quaqua playbook list`; rules/_all/brief.md)\n"),
        ),
        other => (
            1,
            format!("error: cannot load playbook '{name}': {other}\n"),
        ),
    })?;
    let injection = render_injection(&playbook, principles_dir)
        .map_err(|e| (1, format!("error: cannot render playbook '{name}': {e}\n")))?;
    Ok(Some(injection))
}

/// [`prepare_injection`] against the workspace's `playbooks/` and
/// `principles/`. An unresolvable or missing index only warns: the brief is
/// not wrong because the workspace is.
pub fn prepare_injection_in_workspace(
    task: &str,
) -> Result<Option<PlaybookInjection>, (i32, String)> {
    if named_playbook(task).is_none() {
        return Ok(None);
    }
    let (playbooks_dir, principles_dir) =
        match (resolve_playbooks_dir(None), resolve_principles_dir(None)) {
            (Ok(playbooks), Ok(principles)) => (playbooks, principles),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!(
                    "warning: cannot resolve the playbook index ({e}); 'Playbook:' not checked"
                );
                return Ok(None);
            }
        };
    if !playbooks_dir.is_dir() || !principles_dir.is_dir() {
        eprintln!(
            "warning: playbook index missing ({} / {}); 'Playbook:' not checked",
            playbooks_dir.display(),
            principles_dir.display()
        );
        return Ok(None);
    }
    prepare_injection(task, &playbooks_dir, &principles_dir)
}

/// Print the advisory header warnings for a brief on stderr.
pub fn warn_brief_headers(task: &str) {
    for warning in brief_header_warnings(task) {
        eprintln!("{warning}");
    }
}

/// The prompt body handed to the agent: the brief, the injection when there is
/// one, then the report instruction. Without an injection it is byte-identical
/// to the body before § S4.
pub fn compose_prompt_body(name: &str, task: &str, injection: Option<&str>) -> String {
    let tail = format!(
        "When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/{name}.md (publish it atomically, never a partial file); reply only the path."
    );
    match injection {
        Some(block) => format!("{task}\n{block}\n{tail}"),
        None => format!("{task}. {tail}"),
    }
}
