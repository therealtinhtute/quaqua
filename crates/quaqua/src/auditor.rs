//! quaqua auditor — read-only drift and pipeline audit.
//!
//! Replaces the legacy auditor shell script with exact behavioral parity.
//! Runs deterministic pre-checks over pipeline runs, challenges, live workers,
//! and instruction docs, builds the audit prompt, executes the headless harness
//! (`opencode` or `claude`), and writes the final report to `reports/audit-YYYY-MM-DD-HH.md`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "# Usage: quaqua auditor run [--dry-run] [--out <path>] [--model <model>] [--harness <harness>] [--repo <path>]";

pub const PROMPT_TEMPLATE: &str = r#"You are the Auditor for the quaqua repo. READ-ONLY: inspect and report only.
Do not edit, create, or delete any file; do not run mutating commands; do not
contact, prompt, or instruct any agent. If a redirect is needed, flag it in the
report — the CoS routes it.

Audit five things:
1. Drift in CoS activity. Read memory/ files from the last 2 days, projects/*.md,
   and `git log` (last ~30 commits) of each repo listed in projects/INDEX.md.
   Flag: claimed outcomes the code or git history does not back; project
   "Now"/"Log" entries contradicted by repo state; memory rules or lessons that
   rest on stale facts.
2. Stale docs vs real code/scripts. Flag: AGENTS.md, memory/rules.md, models.md,
   rules/ entries that reference removed scripts/behaviors or describe a flow the
   scripts no longer implement; scripts or docs/plans that contradict each other;
   instruction docs over 100 lines (doc-rot: agents trust them over the code).
3. Unhandled memory. Flag: reports/audit-*.md with no
   `- [audit] <report file>:` line in any memory/YYYY-MM-DD.md (the
   handled marker CoS writes, see AGENTS.md); `## Lessons` lines (other than
   `none`) in runs/*.md from the last 2 days that no rules/ file cites as
   `runs/<name>.md`.
4. Pipeline runs (runs/pipeline-*.md, written by `quaqua pipeline`; the
   last `- status:` line is the outcome). Flag:
   - `status: disputed|blocked` with no memory/YYYY-MM-DD.md line naming
     `pipeline-<task>` (the CoS handling note, same convention as the
     `[audit]` report marker);
   - `status: clean` whose `commit` is `none`/`?` or that
     `git rev-parse --verify <sha>^{commit}` cannot resolve in the pipeline's
     recorded repo (the `- repo:` header in runs/pipeline-<task>.md — Peers
     build in project repos, not this checkout; a missing recorded repo is a
     separate flag);
   - `status: clean` where review ran (`- review round <N>:` lines — not
     `review: skipped`) but neither the last review-round line nor the
     reviewer runs file it names carries a `Verdict:` line;
   - a `## Challenge` section in any runs/*.md (except *.rules.md — injected
     rule dumps where the word describes a rule, not a challenge) with no
     memory line naming the worker. Challenges are resolved by the Lead
     before acceptance; no handling line = still open. A Challenge already
     copied into a pipeline file's `## Challenge (from <file>)` section is
     covered by that file's flag — do not report it twice.
   Deterministic pre-check — the script already ran the cheap greps; list
   each finding below under `## Pipeline`, `## Workers` or (doc-rot)
   `## Stale docs` unless repo evidence
   contradicts it, and add any flag the greps missed:
__PRECHECK__

Write your whole report as the final message — it becomes the report file.
Format: `## Drift`, `## Stale docs`, `## Unhandled`, `## Pipeline`, and `## Workers`
sections, each `- file: finding + evidence` (cite path/commit), one line each.
No findings in a section → write `- none`. End with one line
`Verdict: clean | N findings`. Terse, technical."#;

#[derive(Debug, Default, Clone)]
pub struct AuditorRunOptions {
    pub dry_run: bool,
    pub out: Option<PathBuf>,
    pub model: Option<String>,
    pub harness: Option<String>,
    pub repo: Option<PathBuf>,
}

pub fn handle_auditor(args: &[String]) -> Result<i32, (i32, String)> {
    if args.is_empty() {
        return Err((1, format!("{USAGE}\n")));
    }

    match args[0].as_str() {
        "run" => {
            let mut opts = AuditorRunOptions::default();
            let mut i = 1;
            while i < args.len() {
                let arg = &args[i];
                if arg == "--dry-run" {
                    opts.dry_run = true;
                    i += 1;
                } else if arg == "--out" {
                    if i + 1 < args.len() {
                        opts.out = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else {
                        return Err((1, format!("error: --out requires a path\n{USAGE}\n")));
                    }
                } else if let Some(val) = arg.strip_prefix("--out=") {
                    opts.out = Some(PathBuf::from(val));
                    i += 1;
                } else if arg == "--model" {
                    if i + 1 < args.len() {
                        opts.model = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return Err((1, format!("error: --model requires a value\n{USAGE}\n")));
                    }
                } else if let Some(val) = arg.strip_prefix("--model=") {
                    opts.model = Some(val.to_string());
                    i += 1;
                } else if arg == "--harness" {
                    if i + 1 < args.len() {
                        opts.harness = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return Err((1, format!("error: --harness requires a value\n{USAGE}\n")));
                    }
                } else if let Some(val) = arg.strip_prefix("--harness=") {
                    opts.harness = Some(val.to_string());
                    i += 1;
                } else if arg == "--repo" {
                    if i + 1 < args.len() {
                        opts.repo = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    } else {
                        return Err((1, format!("error: --repo requires a path\n{USAGE}\n")));
                    }
                } else if let Some(val) = arg.strip_prefix("--repo=") {
                    opts.repo = Some(PathBuf::from(val));
                    i += 1;
                } else if arg == "--help" || arg == "-h" {
                    println!("{USAGE}");
                    return Ok(0);
                } else {
                    return Err((
                        1,
                        format!("error: unknown auditor option '{arg}'\n{USAGE}\n"),
                    ));
                }
            }
            run_auditor(opts)
        }
        "--help" | "-h" => {
            println!("{USAGE}");
            Ok(0)
        }
        unknown => Err((
            1,
            format!("error: unknown auditor command '{unknown}'\n{USAGE}\n"),
        )),
    }
}

pub fn run_auditor(opts: AuditorRunOptions) -> Result<i32, (i32, String)> {
    let root = resolve_auditor_root(opts.repo.as_deref());

    let out = resolve_out_path(&root, opts.out.as_deref());
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| (1, format!("error creating report dir: {e}\n")))?;
    }

    let flags = run_deterministic_prechecks(&root);

    let precheck_block = if flags.is_empty() {
        "- none".to_string()
    } else {
        flags.join("\n")
    };

    let prompt = PROMPT_TEMPLATE.replace("__PRECHECK__", &precheck_block);

    if opts.dry_run {
        let mut report = String::from("## Drift\n- none\n\n## Stale docs\n");
        let stale_flags: Vec<&str> = flags
            .iter()
            .filter(|f| f.contains("doc-rot risk"))
            .map(|s| s.as_str())
            .collect();
        if stale_flags.is_empty() {
            report.push_str("- none\n\n");
        } else {
            for f in stale_flags {
                report.push_str(f);
                report.push('\n');
            }
            report.push('\n');
        }

        report.push_str("## Unhandled\n- none\n\n## Pipeline\n");
        let pipeline_flags: Vec<&str> = flags
            .iter()
            .filter(|f| {
                f.starts_with("- runs/pipeline-")
                    || (f.starts_with("- runs/") && f.contains("'## Challenge'"))
            })
            .map(|s| s.as_str())
            .collect();
        if pipeline_flags.is_empty() {
            report.push_str("- none\n\n");
        } else {
            for f in pipeline_flags {
                report.push_str(f);
                report.push('\n');
            }
            report.push('\n');
        }

        report.push_str("## Workers\n");
        let worker_flags: Vec<&str> = flags
            .iter()
            .filter(|f| f.contains("herdr") || f.contains("live worker"))
            .map(|s| s.as_str())
            .collect();
        if worker_flags.is_empty() {
            report.push_str("- none\n\n");
        } else {
            for f in worker_flags {
                report.push_str(f);
                report.push('\n');
            }
            report.push('\n');
        }

        if flags.is_empty() {
            report.push_str("Verdict: clean\n");
        } else {
            report.push_str(&format!("Verdict: {} findings\n", flags.len()));
        }

        std::fs::write(&out, &report)
            .map_err(|e| (1, format!("error writing dry-run report: {e}\n")))?;
        println!("{}", out.display());
        return Ok(0);
    }

    let harness = resolve_harness(opts.harness.as_deref());
    let (model, mut cmd) = match harness.as_str() {
        "opencode" => {
            let m = opts
                .model
                .clone()
                .or_else(|| {
                    std::env::var("AUDITOR_MODEL")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "opencode-go/deepseek-v4.1-flash#low".to_string());
            let mut c = Command::new("opencode");
            c.current_dir(&root);
            c.args(["run", "--standalone", "--auto", "-m", &m, &prompt]);
            (m, c)
        }
        _ => {
            let m = opts
                .model
                .clone()
                .or_else(|| {
                    std::env::var("AUDITOR_MODEL")
                        .ok()
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "claude-opus-5-5".to_string());
            let mut c = Command::new("claude");
            c.current_dir(&root);
            c.arg("-p")
                .arg("--model")
                .arg(&m)
                .arg("--permission-mode")
                .arg("dontAsk")
                .arg("--setting-sources")
                .arg("")
                .arg("--allowedTools")
                .arg("Read,Grep,Glob,Bash(git log:*),Bash(git diff:*),Bash(git show:*)")
                .arg("--disallowedTools")
                .arg("Bash(git *--output*)")
                .arg("--")
                .arg(&prompt);
            (m, c)
        }
    };
    let _ = std::fs::File::create(&out);

    let _ = model;

    let res = cmd.output();
    let (rc, stdout, stderr) = match res {
        Ok(output) => (
            output.status.code().unwrap_or(1),
            output.stdout,
            output.stderr,
        ),
        Err(e) => (1, Vec::new(), e.to_string().into_bytes()),
    };

    if rc != 0 {
        if !stderr.is_empty() {
            let _ = std::io::stderr().write_all(&stderr);
        }
        if !stdout.is_empty() {
            let _ = std::fs::write(&out, &stdout);
        }
        let is_empty = match std::fs::metadata(&out) {
            Ok(meta) => meta.len() == 0,
            Err(_) => true,
        };
        if is_empty {
            let fail_msg = format!(
                "# Auditor {} — FAILED\n\n`{}` exited {}; see scheduler stderr logs (journal or Library/Logs/quaqua-auditor.log).\n",
                current_date_minute(), harness, rc
            );
            let _ = std::fs::write(&out, fail_msg);
        }
        return Err((rc, String::new()));
    }

    std::fs::write(&out, &stdout).map_err(|e| (1, format!("error writing report: {e}\n")))?;
    println!("{}", out.display());
    Ok(0)
}

fn resolve_auditor_root(explicit: Option<&Path>) -> PathBuf {
    resolve_auditor_root_from(
        explicit,
        crate::workspace::env_path("AUDITOR_REPO").as_deref(),
        crate::workspace::env_path("QUAQUA_HOME").as_deref(),
        crate::workspace::env_path("QUAQUA_ROOT").as_deref(),
        crate::workspace::env_path("HOME").as_deref(),
    )
}

/// `--repo` first, then `AUDITOR_REPO`, then the one workspace root
/// (`QUAQUA_HOME` > `QUAQUA_ROOT` > `$HOME/quaqua`). Pure — testable without
/// `set_var`.
fn resolve_auditor_root_from(
    explicit: Option<&Path>,
    auditor_repo: Option<&Path>,
    quaqua_home: Option<&Path>,
    quaqua_root: Option<&Path>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Some(p) = auditor_repo {
        return p.to_path_buf();
    }
    crate::workspace::workspace_root_from_env(quaqua_home, quaqua_root, home)
}

fn resolve_out_path(root: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(out) = std::env::var("AUDITOR_OUT").or_else(|_| std::env::var("SUPERVISOR_OUT")) {
        if !out.is_empty() {
            return PathBuf::from(out);
        }
    }
    root.join("reports")
        .join(format!("audit-{}.md", current_date_hour()))
}

fn resolve_harness(explicit: Option<&str>) -> String {
    if let Some(h) = explicit {
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if let Ok(h) = std::env::var("AUDITOR_HARNESS") {
        if !h.is_empty() {
            return h;
        }
    }
    if is_on_path("claude") {
        "claude".to_string()
    } else if is_on_path("opencode") {
        "opencode".to_string()
    } else {
        "claude".to_string()
    }
}

pub fn is_on_path(binary: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary);
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = candidate.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            return true;
                        }
                    }
                }
                #[cfg(not(unix))]
                return true;
            }
        }
    }
    false
}

pub fn current_date_hour() -> String {
    if let Ok(output) = Command::new("date").arg("+%Y-%m-%d-%H").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch_secs_date_hour(secs)
}

pub fn current_date_minute() -> String {
    if let Ok(output) = Command::new("date").arg("+%Y-%m-%d %H:%M").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch_secs_date_minute(secs)
}

fn format_epoch_secs_date_hour(secs: u64) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}-{hour:02}")
}

fn format_epoch_secs_date_minute(secs: u64) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970;
    loop {
        let leap = is_leap_year(year);
        let ydays = if leap { 366 } else { 365 };
        if days < ydays {
            break;
        }
        days -= ydays;
        year += 1;
    }
    let leap = is_leap_year(year);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &m in &mdays {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn mem_mentions(root: &Path, needles: &[&str]) -> bool {
    let mem_dir = root.join("memory");
    let entries = match std::fs::read_dir(&mem_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                for needle in needles {
                    if content.contains(needle) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn git_verify_commit(repo: &Path, sha: &str) -> bool {
    let arg = format!("{sha}^{{commit}}");
    let res = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", &arg])
        .output();
    matches!(res, Ok(output) if output.status.success())
}

pub fn run_deterministic_prechecks(root: &Path) -> Vec<String> {
    let mut flags: Vec<String> = Vec::new();

    // ── 1. pipeline-run pre-checks (runs/pipeline-*.md) ──────────────────────
    let runs_dir = root.join("runs");
    if let Ok(entries) = std::fs::read_dir(&runs_dir) {
        let mut p_files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| name.starts_with("pipeline-") && name.ends_with(".md"))
                    .unwrap_or(false)
            })
            .collect();
        p_files.sort();

        for f in p_files {
            let base = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let content = match std::fs::read_to_string(&f) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let sline = match content
                .lines()
                .filter(|l| l.starts_with("- status: "))
                .last()
            {
                Some(l) => l,
                None => continue,
            };

            let rest = &sline["- status: ".len()..];
            let st_len = rest
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .count();
            let st = &rest[..st_len];

            match st {
                "disputed" | "blocked" => {
                    if !mem_mentions(root, &[base]) {
                        flags.push(format!("- runs/{base}.md: status {st} but no memory/YYYY-MM-DD.md line names {base}"));
                    }
                }
                "clean" => {
                    let sha = if let Some(idx) = sline.find("commit ") {
                        let after = &sline[idx + "commit ".len()..];
                        after.split_whitespace().next().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };

                    let mut prepo_raw = String::new();
                    for line in content.lines() {
                        if line.starts_with("- task: ") {
                            if let Some(idx) = line.find(" · repo: ") {
                                prepo_raw = line[idx + " · repo: ".len()..].trim().to_string();
                                break;
                            }
                        }
                    }

                    let prepo = if prepo_raw.starts_with("~/") {
                        if let Ok(home) = std::env::var("HOME") {
                            PathBuf::from(home).join(&prepo_raw[2..])
                        } else {
                            PathBuf::from(&prepo_raw)
                        }
                    } else if prepo_raw == "~" {
                        if let Ok(home) = std::env::var("HOME") {
                            PathBuf::from(home)
                        } else {
                            PathBuf::from("~")
                        }
                    } else if !prepo_raw.is_empty() {
                        PathBuf::from(&prepo_raw)
                    } else {
                        root.to_path_buf()
                    };

                    if !prepo.is_dir() {
                        flags.push(format!(
                            "- runs/{base}.md: status clean but recorded repo '{}' is missing",
                            prepo.display()
                        ));
                    } else if sha.is_empty()
                        || sha == "none"
                        || sha == "?"
                        || !git_verify_commit(&prepo, &sha)
                    {
                        let sha_disp = if sha.is_empty() { "none" } else { &sha };
                        flags.push(format!("- runs/{base}.md: status clean but commit '{sha_disp}' does not resolve in recorded repo {}", prepo.display()));
                    }

                    let last_rev = content
                        .lines()
                        .filter(|l| {
                            if let Some(idx) = l.find("review round ") {
                                let after = &l[idx + "review round ".len()..];
                                let digits =
                                    after.chars().take_while(|c| c.is_ascii_digit()).count();
                                digits > 0 && after[digits..].starts_with(':')
                            } else {
                                false
                            }
                        })
                        .last()
                        .unwrap_or("");

                    if !last_rev.is_empty() && !last_rev.contains("Verdict:") {
                        let mut revfile = String::new();
                        if let Some(open) = last_rev.rfind('(') {
                            if let Some(close) = last_rev[open..].find(')') {
                                let inner = &last_rev[open + 1..open + close];
                                if inner.ends_with(".md") {
                                    revfile = inner.to_string();
                                }
                            }
                        }
                        if revfile.is_empty() && last_rev.contains(" spawned on ") {
                            if let Some(idx) = last_rev.find(" spawned on ") {
                                let prefix = &last_rev[..idx];
                                if let Some(colon) = prefix.rfind(':') {
                                    let name = prefix[colon + 1..].trim();
                                    if !name.is_empty() && !name.contains(' ') {
                                        revfile = format!("{name}.md");
                                    }
                                }
                            }
                        }
                        let has_verdict = if !revfile.is_empty() {
                            let rev_path = root.join("runs").join(&revfile);
                            if let Ok(rev_content) = std::fs::read_to_string(&rev_path) {
                                rev_content
                                    .lines()
                                    .any(|l| l.trim_start().starts_with("Verdict:"))
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !has_verdict {
                            flags.push(format!("- runs/{base}.md: status clean, review ran, but no 'Verdict:' on the last review-round line or in the reviewer runs file it names"));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // ── 2. challenge pre-checks (runs/*.md) ───────────────────────────────────
    if let Ok(entries) = std::fs::read_dir(&runs_dir) {
        let mut r_files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| name.ends_with(".md") && !name.ends_with(".rules.md"))
                    .unwrap_or(false)
            })
            .collect();
        r_files.sort();

        for f in r_files {
            let content = match std::fs::read_to_string(&f) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !content.lines().any(|l| l.starts_with("## Challenge")) {
                continue;
            }
            let base = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let from_needle = format!("(from {base}.md)");
            let mut copied = false;
            if let Ok(p_entries) = std::fs::read_dir(&runs_dir) {
                for pe in p_entries.flatten() {
                    let p = pe.path();
                    if p != f {
                        if let Some(pname) = p.file_name().and_then(|n| n.to_str()) {
                            if pname.starts_with("pipeline-") && pname.ends_with(".md") {
                                if let Ok(pcontent) = std::fs::read_to_string(&p) {
                                    if pcontent.contains(&from_needle) {
                                        copied = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if copied {
                continue;
            }

            let mut short = base.to_string();
            if let Some(s) = short.strip_suffix(".plan") {
                short = s.to_string();
            } else if let Some(s) = short.strip_suffix(".build") {
                short = s.to_string();
            } else if let Some(idx) = short.rfind(".fix") {
                let suffix = &short[idx + ".fix".len()..];
                if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                    short = short[..idx].to_string();
                }
            }

            if short == base {
                if !mem_mentions(root, &[base]) {
                    flags.push(format!("- runs/{base}.md: '## Challenge' unresolved — no memory/YYYY-MM-DD.md line names {base}"));
                }
            } else if !mem_mentions(root, &[base, &short]) {
                flags.push(format!("- runs/{base}.md: '## Challenge' unresolved — no memory/YYYY-MM-DD.md line names {base} or {short}"));
            }
        }
    }

    // ── 3. doc-rot pre-checks ─────────────────────────────────────────────────
    let mut doc_files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("md") {
                doc_files.push(p);
            }
        }
    }
    let rules_dir = root.join("rules");
    if let Ok(subdirs) = std::fs::read_dir(&rules_dir) {
        for sub in subdirs.flatten() {
            let sub_p = sub.path();
            if sub_p.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&sub_p) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("md") {
                            doc_files.push(p);
                        }
                    }
                }
            }
        }
    }
    let projects_dir = root.join("projects");
    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("md") {
                doc_files.push(p);
            }
        }
    }
    let mem_rules = root.join("memory").join("rules.md");
    if mem_rules.is_file() {
        doc_files.push(mem_rules);
    }

    doc_files.sort();

    for f in doc_files {
        if let Ok(bytes) = std::fs::read(&f) {
            let n = bytes.iter().filter(|&&b| b == b'\n').count();
            if n > 100 {
                let rel = f.strip_prefix(root).unwrap_or(&f);
                flags.push(format!(
                    "- {}: {} lines (>100) — doc-rot risk, trim or split",
                    rel.display(),
                    n
                ));
            }
        }
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quaqua_home_wins_over_quaqua_root() {
        // QUAQUA_HOME=A QUAQUA_ROOT=B must audit A, like every other command
        // (the auditor used to read QUAQUA_ROOT first).
        assert_eq!(
            resolve_auditor_root_from(
                None,
                None,
                Some(Path::new("A")),
                Some(Path::new("B")),
                Some(Path::new("home")),
            ),
            PathBuf::from("A"),
        );
    }

    #[test]
    fn explicit_and_auditor_repo_win_over_the_workspace() {
        assert_eq!(
            resolve_auditor_root_from(
                Some(Path::new("cli")),
                Some(Path::new("env")),
                Some(Path::new("A")),
                None,
                None,
            ),
            PathBuf::from("cli"),
        );
        assert_eq!(
            resolve_auditor_root_from(
                None,
                Some(Path::new("env")),
                Some(Path::new("A")),
                None,
                None,
            ),
            PathBuf::from("env"),
        );
    }

    #[test]
    fn test_prompt_template_has_all_keywords() {
        assert!(PROMPT_TEMPLATE.contains("READ-ONLY"));
        assert!(PROMPT_TEMPLATE.contains("memory/"));
        assert!(PROMPT_TEMPLATE.contains("projects/"));
        assert!(PROMPT_TEMPLATE.contains("git log"));
        assert!(PROMPT_TEMPLATE.contains("each repo listed in projects/INDEX.md"));
        assert!(PROMPT_TEMPLATE.contains("## Unhandled"));
        assert!(PROMPT_TEMPLATE.contains("[audit]"));
        assert!(PROMPT_TEMPLATE.contains("## Lessons"));
        assert!(PROMPT_TEMPLATE.contains("runs/"));
    }
}
