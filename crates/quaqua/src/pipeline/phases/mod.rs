//! Pipeline execution phases and shared context.

pub mod build;
pub mod fix;
pub mod plan;
pub mod review;
pub mod spawn;

use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;
use crate::util::now_sec;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    Done,
}

pub struct PipelineContext<'a, G: GitOps, H: HerdrOps> {
    pub task: String,
    pub repo_path: PathBuf,
    pub state_file: PathBuf,
    pub pfile: PathBuf,
    pub runs_dir: PathBuf,
    pub brief_text: String,
    pub git: &'a G,
    pub herdr: &'a H,
    pub cfg: crate::config::Config,
    pub inv_start: u64,
    pub plan_budget: u64,
    pub build_budget: u64,
    pub review_budget: u64,
    pub fix_budget: u64,
    pub max_sec: u64,
    pub review_lines: usize,
    pub grid_tab: String,
    pub grid_owner: String,
}

impl<'a, G: GitOps, H: HerdrOps> PipelineContext<'a, G, H> {
    pub fn run_grid_hook(&self) -> bool {
        if std::env::var("GRID_SKIP").as_deref() == Ok("1")
            && std::env::var("QUAQUA_TEST").as_deref() == Ok("1")
        {
            return true;
        }
        match crate::grid::run_grid_repair(&self.grid_tab, Some(&self.grid_owner)) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("warning: grid check failed ({e}); arrange panes manually");
                false
            }
        }
    }

    pub fn finish(
        &self,
        state: &mut PipelineState,
        st: &str,
        code: i32,
        blocker: Option<&str>,
    ) -> Result<StepResult, (i32, String)> {
        let el = state.start_elapsed + (now_sec() - self.inv_start);
        state.start_elapsed = el;

        let mut final_st = st.to_string();
        let mut final_blocker = blocker.map(|s| s.to_string());

        if final_st == "clean" && code == 0 && state.start_elapsed > self.max_sec {
            final_st = "blocked".to_string();
            if final_blocker.is_none() {
                final_blocker = Some(format!(
                    "total budget {}s of work time exceeded — outcome arrived over budget",
                    self.max_sec
                ));
            }
        }

        let mins = state.start_elapsed / 60;
        if pipeline_finished(&final_st, code) {
            state.finished = 1;
        }
        let _ = state.save(&self.state_file);

        let mut pcontent = std::fs::read_to_string(&self.pfile).unwrap_or_default();
        let commit_str = if state.commit.is_empty() {
            "none"
        } else {
            &state.commit
        };
        pcontent.push_str(&format!(
            "- status: {final_st} · commit {commit_str} · elapsed {mins}m\n"
        ));

        if let Some(b) = &final_blocker {
            pcontent.push_str(&format!("\n## Blockers\n- {b}\n"));
        }
        let _ = std::fs::write(&self.pfile, pcontent);

        append_telemetry(
            &self.runs_dir,
            &iso_timestamp(),
            &self.task,
            &state.repo,
            &state.builder,
            &state.reviewer,
            &state.review_run,
            state.rounds_run,
            state.review_changed,
            &final_st,
            mins,
        );

        println!(
            "pipeline-{}: status={final_st} rounds={} commit={commit_str} run={}",
            self.task,
            state.rounds_run,
            self.pfile.display()
        );

        if code == 0 {
            Ok(StepResult::Done)
        } else {
            Err((code, String::new()))
        }
    }

    pub fn check_clock(&self, state: &mut PipelineState) -> Option<Result<StepResult, (i32, String)>> {
        let el = state.start_elapsed + (now_sec() - self.inv_start);
        if el > self.max_sec {
            let b = format!(
                "total budget {}s of work time exceeded — remaining work is in the log above",
                self.max_sec
            );
            Some(self.finish(state, "blocked", 4, Some(&b)))
        } else {
            None
        }
    }

    pub fn archive_stale(&self, f: &Path) {
        let bname = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let stale_path = format!("{}.stale.{}", f.display(), now_sec());
        if std::fs::rename(f, &stale_path).is_ok() {
            let mut pcontent = std::fs::read_to_string(&self.pfile).unwrap_or_default();
            pcontent.push_str(&format!(
                "- archived stale report {bname} (predates this phase's prompt)\n"
            ));
            let _ = std::fs::write(&self.pfile, pcontent);
        }
    }

    pub fn wait_file(
        &self,
        state: &mut PipelineState,
        worker: &str,
        prompt: &str,
        budget_sec: u64,
    ) -> Result<(), (i32, String)> {
        let el = state.start_elapsed + (now_sec() - self.inv_start);
        let rem = if self.max_sec > el {
            self.max_sec - el
        } else {
            0
        };
        let mut budget = if rem < budget_sec { rem } else { budget_sec };
        if budget < 1 {
            budget = 1;
        }

        let report_file = self.runs_dir.join(format!("{worker}.md"));
        let budget_file = self.runs_dir.join(format!(".{worker}.budget"));
        let elapsed_file = self.runs_dir.join(format!(".{worker}.elapsed"));
        let grace_file = self.runs_dir.join(format!(".{worker}.grace"));
        let sentinel = self.runs_dir.join(format!(".{worker}.nudged"));
        let autoext_file = self.runs_dir.join(format!(".{worker}.autoext"));

        let rc: i32;
        let mut prompt_never_sent = false;

        if state.prompted == 0 {
            if report_file.exists() {
                self.archive_stale(&report_file);
            }

            // Per-phase budget reset: grant fresh allowance for this phase
            let _ = std::fs::write(&budget_file, format!("{budget}\n"));
            let _ = std::fs::remove_file(&elapsed_file);
            let _ = std::fs::remove_file(&grace_file);
            let _ = std::fs::remove_file(&sentinel);
            let _ = std::fs::remove_file(&autoext_file);

            state.prompted = 1;
            state.prompt_ts = now_sec();
            let _ = state.save(&self.state_file);

            let _prev_b = std::env::var("PROMPT_WORKER_BUDGET_SEC").ok();
            std::env::set_var("PROMPT_WORKER_BUDGET_SEC", budget.to_string());
            rc = match crate::worker::prompt::prompt_worker(
                worker,
                Some(prompt),
                false,
                None,
                self.git,
                self.herdr,
            ) {
                Ok(()) => 0,
                Err((c, _)) => c,
            };
            if let Some(pb) = _prev_b {
                std::env::set_var("PROMPT_WORKER_BUDGET_SEC", pb);
            } else {
                std::env::remove_var("PROMPT_WORKER_BUDGET_SEC");
            }
            prompt_never_sent = rc == 1;

            if prompt_never_sent {
                state.prompted = 0;
                let _ = state.save(&self.state_file);
            }
        } else {
            // Resume mid-phase: keep existing .elapsed and .budget
            if report_file.is_file() && file_mtime(&report_file) < state.prompt_ts {
                self.archive_stale(&report_file);
            }
            let _prev_b = std::env::var("PROMPT_WORKER_BUDGET_SEC").ok();
            std::env::set_var("PROMPT_WORKER_BUDGET_SEC", budget.to_string());
            rc = match crate::worker::prompt::prompt_worker(
                worker,
                None,
                true,
                None,
                self.git,
                self.herdr,
            ) {
                Ok(()) => 0,
                Err((c, _)) => c,
            };
            if let Some(pb) = _prev_b {
                std::env::set_var("PROMPT_WORKER_BUDGET_SEC", pb);
            } else {
                std::env::remove_var("PROMPT_WORKER_BUDGET_SEC");
            }
        }

        if rc != 0 {
            let b = if prompt_never_sent {
                format!(
                    "prompt dispatch failed for {worker} (exit 1 before send) — state left prompted=0 so '--resume' re-sends the prompt instead of waiting on an idle worker; run 'quaqua pipeline --resume {} {}'",
                    self.task, state.repo
                )
            } else {
                format!(
                    "{} wait for {worker}: watchdog exit {rc} — fix the cause ('quaqua worker prompt --resume {worker}' keeps a live wait going), then 'quaqua pipeline --resume {} {}'",
                    state.phase, self.task, state.repo
                )
            };
            let _ = self.finish(state, "blocked", rc, Some(&b));
            return Err((rc, String::new()));
        }
        Ok(())
    }

    pub fn check_impl(
        &self,
        state: &mut PipelineState,
        report_path: &Path,
        prev: Option<&str>,
    ) -> Result<Option<StepResult>, (i32, String)> {
        let wt_path = Path::new(&state.wt);
        let rev_out = self.git.run_in(wt_path, &["rev-parse", "HEAD"]);
        let head = match rev_out {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            _ => "?".to_string(),
        };

        let st_out = self.git.run_in(wt_path, &["status", "--porcelain"]);
        let dirty = match st_out {
            Ok(o) if o.status.success() => {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            }
            _ => "?".to_string(),
        };

        if head == "?" || dirty == "?" {
            let b = format!(
                "cannot inspect {} (git rev-parse/status failed) — fix the worktree, then 'quaqua pipeline --resume {} {}'",
                state.wt, self.task, state.repo
            );
            let _ = self.finish(state, "blocked", 3, Some(&b));
            return Err((3, String::new()));
        }

        state.head_sha = head.clone();
        state.commit = if head.len() >= 7 {
            head[..7].to_string()
        } else {
            head.clone()
        };

        if let Some(p) = prev {
            if head == p {
                let prev7 = if p.len() >= 7 { &p[..7] } else { p };
                let b = format!(
                    "fix produced no commits past {prev7} on quaqua/{}-{} — a no-op answer to findings belongs in '## Challenge'. Commit the fixes, then 'quaqua pipeline --resume {} {}'",
                    state.builder, self.task, self.task, state.repo
                );
                let _ = self.finish(state, "blocked", 3, Some(&b));
                return Err((3, String::new()));
            }
        } else if head == state.base_sha {
            if dirty.is_empty() {
                if let Ok(rep_content) = std::fs::read_to_string(report_path) {
                    for line in rep_content.lines() {
                        let t = line.trim();
                        if let Some(rest) =
                            t.strip_prefix("Outcome:").or_else(|| t.strip_prefix("outcome:"))
                        {
                            let rest_upper = rest.trim().to_ascii_uppercase();
                            if rest_upper.starts_with("NO CHANGES")
                                || rest_upper.starts_with("NO-OP")
                                || rest_upper.starts_with("NO OP")
                            {
                                let mut pcontent =
                                    std::fs::read_to_string(&self.pfile).unwrap_or_default();
                                pcontent.push_str(
                                    "- build: no commits — builder declared 'Outcome: no changes'; nothing to review\n",
                                );
                                return self.finish(state, "clean", 0, None).map(Some);
                            }
                        }
                    }
                }
            }
            let base7 = if state.base_sha.len() >= 7 {
                &state.base_sha[..7]
            } else {
                &state.base_sha
            };
            let b = format!(
                "builder produced no commits on quaqua/{}-{} (HEAD == base {base7}) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then 'quaqua pipeline --resume {} {}'",
                state.builder, self.task, self.task, state.repo
            );
            let _ = self.finish(state, "blocked", 3, Some(&b));
            return Err((3, String::new()));
        }

        if !dirty.is_empty() {
            let mut pcontent = std::fs::read_to_string(&self.pfile).unwrap_or_default();
            pcontent.push_str(&format!(
                "- uncommitted/untracked in {}:\n{}\n",
                state.wt, dirty
            ));
            let _ = std::fs::write(&self.pfile, pcontent);
            let b = format!(
                "worktree dirty — the files above are missing from quaqua/{}-{}, so the gate and the reviewer would see a different snapshot. Commit or clean them, then 'quaqua pipeline --resume {} {}'",
                state.builder, self.task, self.task, state.repo
            );
            let _ = self.finish(state, "blocked", 3, Some(&b));
            return Err((3, String::new()));
        }

        Ok(None)
    }
}

pub fn pipeline_finished(final_st: &str, code: i32) -> bool {
    code == 0 && final_st != "blocked"
}

pub fn has_real_blockers(content: &str) -> bool {
    let mut in_blockers = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Blockers") {
            in_blockers = true;
            continue;
        }
        if in_blockers && trimmed.starts_with("## ") {
            break;
        }
        if in_blockers && !trimmed.is_empty() {
            let item = trimmed
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim_start_matches('+')
                .trim();
            if item.is_empty() {
                continue;
            }

            // Extract the leading sentence/declaration before sentence-ending delimiters ('.', '…')
            let first_part = item
                .split(|c: char| c == '.' || c == '…')
                .next()
                .unwrap_or(item);

            // Strip markdown emphasis (*, _, `), trailing dots/ellipsis, and whitespace
            let core = first_part
                .trim_matches(|c: char| c == '*' || c == '_' || c == '`' || c == '.' || c == '…' || c.is_whitespace());
            // If the declaration is "none" (e.g. "None", "None. All done...", "**None**. All 9..."),
            // it is an informational note, NOT a blocker.
            if !core.is_empty() && core.eq_ignore_ascii_case("none") {
                continue;
            }

            return true;
        }
    }
    false
}

pub fn blocked_outcome(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("Outcome:")
            .or_else(|| t.strip_prefix("outcome:"))
            .or_else(|| t.strip_prefix("Status:"))
            .or_else(|| t.strip_prefix("status:"))
            .or_else(|| t.strip_prefix("Result:"))
            .or_else(|| t.strip_prefix("result:"))
        {
            let val = rest.trim().to_ascii_uppercase();
            if val.starts_with("BLOCKED")
                || val.starts_with("PARTIAL")
                || val.starts_with("INCOMPLETE")
                || val.starts_with("FAILED")
            {
                return true;
            }
        }
    }
    has_real_blockers(&content)
}

pub fn log_challenge(source: &Path, pfile: &Path) {
    if let Ok(content) = std::fs::read_to_string(source) {
        let mut extract = String::new();
        let mut in_challenge = false;
        for line in content.lines() {
            if line.starts_with("## Challenge") {
                in_challenge = true;
                extract.push_str(line);
                extract.push('\n');
                continue;
            }
            if in_challenge && line.starts_with("## ") {
                break;
            }
            if in_challenge {
                extract.push_str(line);
                extract.push('\n');
            }
        }
        if !extract.is_empty() {
            let sname = source.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let header = format!("\n## Challenge (from {sname})\n");
            let mut file_content = std::fs::read_to_string(pfile).unwrap_or_default();
            file_content.push_str(&header);
            file_content.push_str(&extract);
            let _ = std::fs::write(pfile, file_content);
        }
    }
}

pub fn log_blockers(source: &Path, pfile: &Path) {
    if let Ok(content) = std::fs::read_to_string(source) {
        let sname = source.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let mut extract = format!("\n## Worker stopped early (from {sname})\n");

        for line in content.lines() {
            let t = line.trim();
            if t.starts_with("Outcome:") || t.starts_with("Status:") || t.starts_with("Result:") {
                extract.push_str(line);
                extract.push('\n');
            }
        }

        let mut in_blockers = false;
        for line in content.lines() {
            if line.starts_with("## Blockers") {
                in_blockers = true;
                continue;
            }
            if in_blockers && line.starts_with("## ") {
                break;
            }
            if in_blockers {
                extract.push_str(line);
                extract.push('\n');
            }
        }

        let mut file_content = std::fs::read_to_string(pfile).unwrap_or_default();
        file_content.push_str(&extract);
        let _ = std::fs::write(pfile, file_content);
    }
}

pub fn append_telemetry(
    runs_dir: &Path,
    ts: &str,
    task: &str,
    repo: &str,
    builder: &str,
    reviewer: &str,
    review: &str,
    rounds: u32,
    changed: bool,
    status: &str,
    minutes: u64,
) {
    let tele_path = runs_dir.join("telemetry.jsonl");
    let line = format!(
        "{{\"ts\":\"{}\",\"task\":\"{}\",\"repo\":\"{}\",\"builder\":\"{}\",\"reviewer\":\"{}\",\"review\":\"{}\",\"rounds\":{},\"review_changed_outcome\":{},\"status\":\"{}\",\"minutes\":{}}}\n",
        ts, task, repo, builder, reviewer, review, rounds, changed, status, minutes
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(tele_path)
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn harness_of_kind<'a>(kind: &str, cfg: &'a crate::config::Config) -> Option<&'a str> {
    cfg.kinds.get(kind).map(|k| k.harness.as_str())
}

pub fn select_default_reviewer(
    builder: &str,
    cfg: &crate::config::Config,
) -> Result<String, (i32, String)> {
    let bh = harness_of_kind(builder, cfg)
        .ok_or_else(|| (1, format!("unknown builder kind: {builder}\n")))?;
    let unavailable: Vec<String> = std::env::var("QUAQUA_UNAVAILABLE_REVIEWERS")
        .map(|s| s.split(',').map(|k| k.trim().to_string()).collect())
        .unwrap_or_default();

    for cand in &cfg.strategy.default_reviewer {
        if unavailable.iter().any(|u| u == cand) {
            continue;
        }
        if let Some(rh) = harness_of_kind(cand, cfg) {
            if bh != rh {
                if let Some(kind_cfg) = cfg.kinds.get(cand) {
                    if kind_cfg.can_review {
                        return Ok(cand.clone());
                    }
                }
            }
        }
    }
    Err((
        1,
        "no available reviewer found in strategy.default_reviewer\n".to_string(),
    ))
}

pub fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        })
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn iso_timestamp() -> String {
    let sec = now_sec();
    let dt = match Command::new("date").arg("-Iseconds").output() {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => {
            format!("{sec}")
        }
    };
    dt
}
