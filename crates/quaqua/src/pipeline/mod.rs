//! Pipeline engine (`quaqua pipeline`).
//! Implements build -> review -> fix orchestration state machine.

pub mod phases;
pub mod state;

use crate::engine::activities::SystemActivities;
use crate::engine::activity::{
    Activity, ActivityError, ActivityRequest, ActivityResult, GitRequest, GitResult, GridRequest,
    HerdrEventRequest, HerdrEventResult, HerdrRequest, ReportFileRequest, ReportFileResult,
};
use crate::engine::child::{SingleWorkerInput, SingleWorkerOutput};
use crate::engine::clock::RealClock;
use crate::engine::journal::Journal;
use crate::engine::journal_sqlite::SqliteJournal;
use crate::engine::runner::{self, RunnerOptions, RunnerOutcome};
use crate::engine::workflow::{StepDecision, Workflow, WorkflowContext, WorkflowEvent};
use crate::worker::prompt::workflow::{SingleWorkerState, SingleWorkerWorkflow, WatchdogConfig};
pub use phases::{
    blocked_outcome, file_mtime, harness_of_kind, has_real_blockers, log_blockers, log_challenge,
    pipeline_finished, select_default_reviewer, PipelineContext, StepResult,
};
pub use state::{
    PipelineInput, PipelineOutput, PipelinePhase, PipelineState, PipelineWorkflowState,
    ReviewStrategy,
};
use std::collections::HashSet;

use crate::cap::count_owner_slots;
use crate::config::load_team_config;
use crate::git::{Git, GitOps};
use crate::herdr::Herdr;
use crate::util::now_sec;
use crate::worker::cleanup::default_runs_dir;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
# Usage: quaqua pipeline <task> <repo> --brief <file> [--builder sude] [--reviewer suhuynh]
#                    [--review auto|always|skip] [--rounds 2] [--base <ref>]
#        quaqua pipeline --resume <task> <repo> [--rounds 2]
#   Build -> review -> fix loop on top of
#   quaqua worker {spawn,prompt,cleanup}; counts as 2 workers
#   toward the cap. <task> is a slug ([a-z0-9-]): builder <builder>-<task>, a
#   fresh reviewer <reviewer>-<task>r<N> each round (quaqua worker spawn --review).
#   Flow: builder writes a plan -> exit 5 'plan-ready' (the Lead approves by
#   resuming) -> build -> review (auto: diff vs the spawn base is >150 changed
#   lines or touches >1 top-level dir; always/skip override) -> findings go
#   straight back to the builder for a fix, then a fresh reviewer; at most
#   <rounds> review rounds. Ends clean | disputed | blocked. '## Challenge' in
#   any worker runs file (builder or reviewer) stops the pipeline as disputed
#   (Lead decides); a reviewer runs file without a 'Verdict: clean | <N>
#   findings' line stops it as blocked. A phase report that declares itself
#   unfinished ('Outcome: BLOCKED|PARTIAL|INCOMPLETE|FAILED' or a non-empty
#   '## Blockers' section — e.g. written after the watchdog nudge) never
#   advances: the pipeline stops blocked with the evidence copied in, and the
#   state stays resumable ('--resume' continues the same phase) — only a
#   non-blocked exit 0 marks the run finished. After
#   BUILD/FIX the branch tip must hold real commits in a clean worktree — the
#   size gate and the reviewer are bound to that exact SHA; a builder that
#   intentionally changed nothing says 'Outcome: no changes' (clean, review
#   skipped). Reviewers are cleaned right after their runs file is read; the
#   caller cleans the builder after re-running its test.
#   Exit codes reuse quaqua worker prompt's watchdog meanings: 0 finished (status
#   line on stdout + runs/pipeline-<task>.md) · 1 usage/state error · 2 cap ·
#   3 stall/gone (also: resumable snapshot/spawn failures) · 4 budget · 5
#   plan-ready pause, a mandatory grid-arrange pause, or a worker blocked on
#   a question. On 2-5 fix the cause (a stuck wait keeps waiting via
#   'quaqua worker prompt --resume <worker>'), then 'quaqua pipeline --resume <task>
#   <repo>' continues from runs/pipeline-<task>.state. Every stop appends one
#   line to runs/telemetry.jsonl {ts,task,repo,builder,reviewer,
#   review:run|skip,rounds,review_changed_outcome,status,minutes} — the last
#   line per task is current. Exit 6 (malformed report: quaqua worker prompt
#   rejects a phase report missing its '## Lessons' / 'Verdict:' tail, R3)
#   propagates here as blocked like every other watchdog exit.
#   Env budgets (sec): PIPELINE_PLAN_BUDGET_SEC 900 · _BUILD_ 2700 · _REVIEW_
#   900 · _FIX_ 1200 · PIPELINE_MAX_SEC 7200 (work time; time between
#   invocations does not count) · PIPELINE_REVIEW_LINES 150.
#   Grid: after each spawn/cleanup the pipeline arranges its whole owner
#   population into tab GRID_TAB (default <repo-basename>-workers, or
#   <slug>-workers for owner <slug>-lead): idle panes move, working ones are
#   reported 'deferred: <pane> (working)'; GRID_SKIP=1 with QUAQUA_TEST=1
#   disables (tests).";

pub fn resolve_project_slug_from_repo(repo: &Path) -> Option<String> {
    if let Ok(root) = crate::workspace::find_quaqua_root() {
        let index = root.join("projects/INDEX.md");
        if index.is_file() {
            if let Ok(content) = std::fs::read_to_string(&index) {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('|') {
                        let parts: Vec<&str> = trimmed.split('|').map(|s| s.trim()).collect();
                        if parts.len() >= 3 && parts[2] != "repo" {
                            let slug = parts[1];
                            let r = parts[2];
                            let p = if let Some(stripped) = r.strip_prefix("~/") {
                                format!("{}/{}", home, stripped)
                            } else if r == "~" {
                                home.clone()
                            } else {
                                r.to_string()
                            };
                            if let Ok(c_repo) = std::fs::canonicalize(Path::new(&p)) {
                                if c_repo == repo {
                                    return Some(slug.to_string());
                                }
                            }
                            if Path::new(&p) == repo {
                                return Some(slug.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(owner) = std::env::var("PIPELINE_LEAD").or_else(|_| std::env::var("QUAQUA_OWNER")) {
        if let Some(slug) = owner.strip_suffix("-lead") {
            return Some(slug.to_string());
        }
    }
    None
}

struct PipelineActivities {
    system: SystemActivities,
}

impl Activity for PipelineActivities {
    fn execute(&self, req: &ActivityRequest) -> Result<ActivityResult, ActivityError> {
        match req {
            ActivityRequest::HerdrEvents(HerdrEventRequest::Subscribe { agent, .. }) => {
                match self.system.execute(req) {
                    Ok(res) => Ok(res),
                    Err(_) => Ok(ActivityResult::HerdrEvents(HerdrEventResult::Subscribed {
                        agent: agent.clone(),
                    })),
                }
            }
            ActivityRequest::HerdrEvents(HerdrEventRequest::PollEventsFor { agent }) => {
                match self.system.execute(req) {
                    Ok(res) => Ok(res),
                    Err(_) => Ok(ActivityResult::HerdrEvents(
                        HerdrEventResult::ScopedEvents {
                            agent: agent.clone(),
                            events: Vec::new(),
                        },
                    )),
                }
            }
            ActivityRequest::HerdrEvents(HerdrEventRequest::PollEvents) => {
                match self.system.execute(req) {
                    Ok(res) => Ok(res),
                    Err(_) => Ok(ActivityResult::HerdrEvents(HerdrEventResult::Events(
                        Vec::new(),
                    ))),
                }
            }
            ActivityRequest::HerdrEvents(HerdrEventRequest::Unsubscribe { agent }) => {
                let _ = self.system.execute(req);
                Ok(ActivityResult::HerdrEvents(
                    HerdrEventResult::Unsubscribed {
                        agent: agent.clone(),
                    },
                ))
            }
            ActivityRequest::ReportFile(ReportFileRequest::CheckMtime { .. }) => {
                match self.system.execute(req) {
                    Ok(res) => Ok(res),
                    Err(ActivityError::NotFound(_)) => {
                        Ok(ActivityResult::ReportFile(ReportFileResult::MtimeMs(0)))
                    }
                    Err(e) => Err(e),
                }
            }
            ActivityRequest::Git(GitRequest::DiffHash { .. }) => match self.system.execute(req) {
                Ok(res) => Ok(res),
                Err(_) => Ok(ActivityResult::Git(GitResult::DiffHash {
                    hash: String::new(),
                    stat: String::new(),
                })),
            },
            ActivityRequest::Herdr(HerdrRequest::AgentInfo { name }) => {
                match self.system.execute(req) {
                    Ok(res) => Ok(res),
                    Err(_) => Ok(ActivityResult::Herdr(
                        crate::engine::activity::HerdrResult::AgentInfo {
                            name: name.clone(),
                            status: "working".to_string(),
                            harness: None,
                            hook_active: false,
                        },
                    )),
                }
            }
            _ => self.system.execute(req),
        }
    }
}

fn seed_state_from_pipeline_state(
    input: PipelineInput,
    state: &PipelineState,
) -> PipelineWorkflowState {
    let mut init_state = PipelineWorkflowState::new(input);
    match state.phase.as_str() {
        "plan" => init_state.phase = PipelinePhase::Plan,
        "build" => init_state.phase = PipelinePhase::Build,
        "review" => {
            init_state.phase = PipelinePhase::Review;
            init_state.round = state.round.max(1);
        }
        "fix" => {
            init_state.phase = PipelinePhase::Fix;
            init_state.round = state.round.max(1);
            let revname = format!("{}-{}r{}", init_state.input.reviewer, init_state.input.task, init_state.round);
            init_state.last_review_report = Some(format!("{revname}.md"));
        }
        _ => {}
    }
    init_state.round = state.round.max(if state.phase == "review" || state.phase == "fix" { 1 } else { 0 });
    init_state.rounds_run = state.rounds_run;
    init_state.last_findings = state.last_findings;
    init_state.review_changed = state.review_changed;
    if !state.head_sha.is_empty() {
        init_state.head_sha = Some(state.head_sha.clone());
    }
    if !state.planfile.is_empty() {
        init_state.planfile = Some(state.planfile.clone());
    }
    init_state
}

fn watchdog_config_from_env(expect_verdict: bool, is_fallback_idle: bool) -> WatchdogConfig {
    let mut config = WatchdogConfig::default();
    config.expect_verdict = expect_verdict;
    config.is_fallback_idle = is_fallback_idle;

    if let Some(p) = std::env::var("PROMPT_WORKER_POLL_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.poll_sec = p;
    }
    if let Some(s) = std::env::var("PROMPT_WORKER_STALL_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.stall_sec = s;
    }
    if let Some(i) = std::env::var("PROMPT_WORKER_IDLE_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.idle_sec = i;
    }
    if let Some(f) = std::env::var("PROMPT_WORKER_IDLE_FALLBACK_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.idle_fallback_sec = f;
    }
    if let Some(g) = std::env::var("PROMPT_WORKER_AGENT_GONE_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.gone_sec = g;
    }
    if let Some(m) = std::env::var("PROMPT_WORKER_MAX_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.hard_cap_sec = m;
    }
    if let Some(n) = std::env::var("PROMPT_WORKER_NUDGE_PCT")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        config.nudge_pct = n;
    }
    config
}

pub fn handle_pipeline(args: &[String], git: &Git, herdr: &Herdr) -> Result<(), (i32, String)> {
    let inv_start = now_sec();
    let runs_dir = default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    let plan_budget = std::env::var("PIPELINE_PLAN_BUDGET_SEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(900);
    let build_budget = std::env::var("PIPELINE_BUILD_BUDGET_SEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2700);
    let review_budget = std::env::var("PIPELINE_REVIEW_BUDGET_SEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(900);
    let fix_budget = std::env::var("PIPELINE_FIX_BUDGET_SEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1200);
    let max_sec = std::env::var("PIPELINE_MAX_SEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(7200);
    let review_lines = std::env::var("PIPELINE_REVIEW_LINES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(150);

    let mut state: PipelineState;
    let task: String;
    let repo_path: PathBuf;
    let state_file: PathBuf;
    let pfile: PathBuf;
    let is_resume = !args.is_empty() && args[0] == "--resume";
    if is_resume {
        let mut resume_rounds: Option<u32> = None;
        let mut pos = Vec::new();
        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--rounds" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                let r: u32 = args[i + 1]
                    .parse()
                    .map_err(|_| (1, "--rounds must be an integer >= 1\n".to_string()))?;
                if r < 1 {
                    return Err((1, "--rounds must be an integer >= 1\n".to_string()));
                }
                resume_rounds = Some(r);
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--rounds=") {
                let r: u32 = v
                    .parse()
                    .map_err(|_| (1, "--rounds must be an integer >= 1\n".to_string()))?;
                if r < 1 {
                    return Err((1, "--rounds must be an integer >= 1\n".to_string()));
                }
                resume_rounds = Some(r);
                i += 1;
            } else if arg.starts_with('-') {
                return Err((1, format!("{}\n", USAGE)));
            } else {
                pos.push(arg.clone());
                i += 1;
            }
        }
        if pos.len() != 2 {
            return Err((1, format!("{}\n", USAGE)));
        }
        task = pos[0].clone();
        if !task
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err((1, format!("{}\n", USAGE)));
        }
        let want_repo = match std::fs::canonicalize(&pos[1]) {
            Ok(p) => p,
            Err(_) => PathBuf::from(&pos[1]),
        };
        state_file = runs_dir.join(format!("pipeline-{task}.state"));
        pfile = runs_dir.join(format!("pipeline-{task}.md"));

        if !state_file.is_file() {
            return Err((
                1,
                format!(
                    "no pipeline state for '{task}' ({}) — nothing to resume\n",
                    state_file.display()
                ),
            ));
        }

        state = PipelineState::load(&state_file).map_err(|e| (1, format!("{e}\n")))?;
        let state_repo = match std::fs::canonicalize(&state.repo) {
            Ok(p) => p,
            Err(_) => PathBuf::from(&state.repo),
        };
        if state_repo != want_repo {
            return Err((
                1,
                format!(
                    "state for '{task}' is on repo {}, not {}\n",
                    state.repo,
                    want_repo.display()
                ),
            ));
        }
        if state.finished == 1 {
            return Err((
                1,
                format!(
                    "pipeline '{task}' already finished — see {}\n",
                    pfile.display()
                ),
            ));
        }

        state.pipeline_pid = std::process::id();
        state.lead_pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
        if let Some(r) = resume_rounds {
            if r > state.rounds {
                state.rounds = r;
            }
        }
        let _ = state.save(&state_file);

        let mut pcontent = std::fs::read_to_string(&pfile).unwrap_or_default();
        pcontent.push_str(&format!(
            "- resumed: {} (phase={})\n",
            phases::iso_timestamp(),
            state.phase
        ));
        let _ = std::fs::write(&pfile, pcontent);
        repo_path = state_repo;
    } else {
        // Fresh start
        let mut builder = "sude".to_string();
        let mut reviewer_opt: Option<String> = None;
        let mut review = "auto".to_string();
        let mut rounds = 2u32;
        let mut brief_opt: Option<PathBuf> = None;
        let mut base = String::new();
        let mut pos = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--brief" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                brief_opt = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--brief=") {
                brief_opt = Some(PathBuf::from(v));
                i += 1;
            } else if arg == "--builder" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                builder = args[i + 1].clone();
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--builder=") {
                builder = v.to_string();
                i += 1;
            } else if arg == "--reviewer" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                reviewer_opt = Some(args[i + 1].clone());
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--reviewer=") {
                reviewer_opt = Some(v.to_string());
                i += 1;
            } else if arg == "--review" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                review = args[i + 1].clone();
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--review=") {
                review = v.to_string();
                i += 1;
            } else if arg == "--rounds" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                rounds = args[i + 1]
                    .parse()
                    .map_err(|_| (1, "--rounds must be an integer >= 1\n".to_string()))?;
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--rounds=") {
                rounds = v
                    .parse()
                    .map_err(|_| (1, "--rounds must be an integer >= 1\n".to_string()))?;
                i += 1;
            } else if arg == "--base" {
                if i + 1 >= args.len() {
                    return Err((1, format!("{}\n", USAGE)));
                }
                base = args[i + 1].clone();
                i += 2;
            } else if let Some(v) = arg.strip_prefix("--base=") {
                base = v.to_string();
                i += 1;
            } else if arg.starts_with('-') {
                return Err((1, format!("{}\n", USAGE)));
            } else {
                pos.push(arg.clone());
                i += 1;
            }
        }

        if pos.len() != 2 {
            return Err((1, format!("{}\n", USAGE)));
        }
        task = pos[0].clone();
        if !task
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err((1, format!("{}\n", USAGE)));
        }

        let raw_repo = PathBuf::from(&pos[1]);
        repo_path = match std::fs::canonicalize(&raw_repo) {
            Ok(p) => p,
            Err(_) => raw_repo,
        };

        let brief = match brief_opt {
            Some(p) if p.is_file() => match std::fs::canonicalize(&p) {
                Ok(cp) => cp,
                Err(_) => p,
            },
            _ => {
                return Err((
                    1,
                    format!(
                        "--brief <file> required and must exist (got: '{}')\n",
                        brief_opt
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|| "none".to_string())
                    ),
                ));
            }
        };

        let brief_str = std::fs::read_to_string(&brief)
            .map_err(|e| (1, format!("cannot read {}: {e}\n", brief.display())))?;
        if !crate::worker::prompt::brief_headers_ok(&brief_str) {
            return Err((
                1,
                "brief must carry the Goal:/Hard constraints:/Current design headers (rules/_all/brief.md)\n".to_string(),
            ));
        }

        let base_cfg = load_team_config()?;
        let proj_slug = resolve_project_slug_from_repo(&repo_path);
        let cfg = if let Some(slug) = proj_slug.as_deref() {
            crate::config::resolve(base_cfg.clone(), Some(slug)).unwrap_or(base_cfg)
        } else {
            base_cfg
        };
        let bh = harness_of_kind(&builder, &cfg)
            .ok_or_else(|| (1, format!("unknown builder kind: {builder}\n")))?;

        let reviewer = match reviewer_opt {
            Some(r) => {
                let rh = harness_of_kind(&r, &cfg)
                    .ok_or_else(|| (1, format!("unknown reviewer kind: {r}\n")))?;
                if !cfg.kinds.get(&r).map(|k| k.can_review).unwrap_or(false) {
                    return Err((
                        1,
                        format!("reviewer kind '{r}' has can_review = false in team.toml\n"),
                    ));
                }
                if bh == rh {
                    return Err((1, format!("builder and reviewer must be different harnesses (models.md): {builder}/{r} are both {bh}\n")));
                }
                r
            }
            None => select_default_reviewer(&builder, &cfg)?,
        };

        let rh = harness_of_kind(&reviewer, &cfg).unwrap();
        if !["auto", "always", "skip"].contains(&review.as_str()) {
            return Err((1, "--review must be auto|always|skip\n".to_string()));
        }
        if rounds < 1 {
            return Err((1, "--rounds must be an integer >= 1\n".to_string()));
        }

        if builder.len() + 1 + task.len() > 20 {
            return Err((
                1,
                format!("builder name {builder}-{task} exceeds 20 chars (models.md Naming)\n"),
            ));
        }
        let round_str = rounds.to_string();
        if reviewer.len() + 1 + task.len() + 1 + round_str.len() > 20 {
            return Err((
                1,
                format!("reviewer name {reviewer}-{task}r{rounds} exceeds 20 chars\n"),
            ));
        }

        let owner = std::env::var("PIPELINE_LEAD")
            .or_else(|_| std::env::var("QUAQUA_OWNER"))
            .unwrap_or_else(|_| "cos".to_string());
        let max_workers = cfg.limits.max_workers_per_orchestrator;
        let pipe_weight = cfg.limits.weights.get("pipeline").copied().unwrap_or(2);

        let slots_used = count_owner_slots(&runs_dir, &owner, None, herdr);
        if slots_used + pipe_weight > max_workers {
            eprintln!("error: worker cap exceeded (owner '{owner}' has {slots_used} slots in use; pipeline reserves {pipe_weight}, cap is {max_workers})");
            return Err((2, String::new()));
        }

        state_file = runs_dir.join(format!("pipeline-{task}.state"));
        pfile = runs_dir.join(format!("pipeline-{task}.md"));

        if state_file.is_file() {
            return Err((
                1,
                format!("pipeline '{task}' already has state — use --resume or a new task slug\n"),
            ));
        }

        let parent_wt = repo_path.parent().unwrap_or(Path::new("."));
        let wt = parent_wt.join(".wt").join(format!("{builder}-{task}"));

        state = PipelineState {
            repo: repo_path.to_string_lossy().to_string(),
            brief: brief.to_string_lossy().to_string(),
            builder: builder.clone(),
            reviewer: reviewer.clone(),
            review: review.clone(),
            rounds,
            base,
            wt: wt.to_string_lossy().to_string(),
            base_sha: String::new(),
            head_sha: String::new(),
            planfile: String::new(),
            commit: String::new(),
            phase: "spawn".to_string(),
            round: 0,
            prompted: 0,
            prompt_ts: 0,
            rounds_run: 0,
            review_run: "skip".to_string(),
            review_changed: false,
            last_findings: 0,
            finished: 0,
            start_elapsed: 0,
            builder_spawned: 0,
            reviewer_spawned: 0,
            lead: std::env::var("PIPELINE_LEAD")
                .or_else(|_| std::env::var("QUAQUA_OWNER"))
                .unwrap_or_default(),
            lead_pane: std::env::var("HERDR_PANE_ID").unwrap_or_default(),
            pipeline_pid: std::process::id(),
        };

        let _ = state.save(&state_file);

        let initial_md = format!(
            "# pipeline {}\n- task: {} · repo: {}\n- builder: {}-{} ({}) · reviewer: {} ({}) · review: {} · max rounds: {}\n- started: {}\n",
            task,
            task,
            state.repo,
            builder,
            task,
            bh,
            reviewer,
            rh,
            review,
            rounds,
            phases::iso_timestamp()
        );
        let _ = std::fs::write(&pfile, initial_md);
    }

    let brief_text = std::fs::read_to_string(&state.brief)
        .map_err(|e| (1, format!("brief file unreadable: {}: {e}\n", state.brief)))?;

    let grid_owner = if state.lead.is_empty() {
        "cos".to_string()
    } else {
        state.lead.clone()
    };
    let grid_tab = std::env::var("GRID_TAB").unwrap_or_else(|_| {
        let repo_name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo");
        crate::worker::spawn::default_grid_tab(&grid_owner, repo_name)
    });

    let base_cfg = load_team_config()?;
    let proj_slug = resolve_project_slug_from_repo(&repo_path);
    let cfg = if let Some(slug) = proj_slug.as_deref() {
        crate::config::resolve(base_cfg.clone(), Some(slug)).unwrap_or(base_cfg)
    } else {
        base_cfg
    };

    let ctx = PipelineContext {
        task: task.clone(),
        repo_path: repo_path.clone(),
        state_file: state_file.clone(),
        pfile: pfile.clone(),
        runs_dir: runs_dir.clone(),
        brief_text,
        git,
        herdr,
        cfg,
        inv_start,
        plan_budget,
        build_budget,
        review_budget,
        fix_budget,
        max_sec,
        review_lines,
        grid_tab,
        grid_owner,
    };

    let db_path = runs_dir.join("quaqua.db");
    let mut journal = SqliteJournal::open(&db_path).map_err(|e| {
        (
            1,
            format!("cannot open sqlite journal {}: {e}\n", db_path.display()),
        )
    })?;
    let mut activities = PipelineActivities {
        system: SystemActivities::with_adapters(herdr.clone(), git.clone(), &repo_path),
    };
    let mut clock = RealClock::new();
    let workflow = PipelineWorkflow::new();
    let parent_wf_id = format!("pipeline:{}", task);

    let abs_brief_path = match std::fs::canonicalize(&state.brief) {
        Ok(p) => p,
        Err(_) => PathBuf::from(&state.brief),
    };

    let review_strategy = match state.review.as_str() {
        "always" => ReviewStrategy::Always,
        "skip" => ReviewStrategy::Skip,
        _ => ReviewStrategy::Auto,
    };
    let resolved_base_sha = if !state.base_sha.is_empty() {
        state.base_sha.clone()
    } else {
        let base_ref = if state.base.is_empty() { "HEAD" } else { &state.base };
        ctx.git
            .run_in(&repo_path, &["rev-parse", base_ref])
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    if !resolved_base_sha.is_empty() && state.base_sha.is_empty() {
        state.base_sha = resolved_base_sha.clone();
        let _ = state.save(&state_file);
    }

    let input = PipelineInput {
        task: task.clone(),
        repo: repo_path.to_string_lossy().to_string(),
        builder: state.builder.clone(),
        reviewer: state.reviewer.clone(),
        brief: ctx.brief_text.clone(),
        brief_path: abs_brief_path.to_string_lossy().to_string(),
        base: state.base.clone(),
        review_strategy,
        max_rounds: state.rounds,
        plan_budget_sec: plan_budget,
        build_budget_sec: build_budget,
        review_budget_sec: review_budget,
        fix_budget_sec: fix_budget,
        base_sha: if resolved_base_sha.is_empty() {
            None
        } else {
            Some(resolved_base_sha)
        },
    };

    let parent_events = journal
        .load_events(&parent_wf_id, 1)
        .map_err(|e| (1, format!("journal error: {e}\n")))?;

    let runner_opts = RunnerOptions {
        snapshot_at_blocked: false,
        ..Default::default()
    };

    let mut parent_outcome = if !is_resume {
        let initial_state = PipelineWorkflowState::new(input.clone());
        runner::run_until_blocked(
            &workflow,
            &parent_wf_id,
            Some(initial_state),
            &mut journal,
            &mut activities,
            &mut clock,
            &runner_opts,
            Vec::new(),
        )
        .map_err(|e| (1, format!("workflow error: {e}\n")))?
    } else if parent_events.is_empty() {
        let init_state = seed_state_from_pipeline_state(input.clone(), &state);
        runner::run_until_blocked(
            &workflow,
            &parent_wf_id,
            Some(init_state),
            &mut journal,
            &mut activities,
            &mut clock,
            &runner_opts,
            Vec::new(),
        )
        .map_err(|e| (1, format!("workflow error: {e}\n")))?
    } else {
        let is_plan_gate = parent_events
            .iter()
            .rev()
            .any(|e| e.kind == "pause_for_approval")
            && !parent_events
                .iter()
                .rev()
                .any(|e| e.kind == "approval_resolved");
        let incoming = if is_plan_gate {
            vec![WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }]
        } else {
            vec![]
        };
        runner::run_until_blocked(
            &workflow,
            &parent_wf_id,
            Some(PipelineWorkflowState::new(input.clone())),
            &mut journal,
            &mut activities,
            &mut clock,
            &runner_opts,
            incoming,
        )
        .map_err(|e| (1, format!("workflow error: {e}\n")))?
    };

    loop {
        if let Some(res) = ctx.check_clock(&mut state) {
            return res.map(|_| ());
        }

        match parent_outcome {
            RunnerOutcome::Completed(out) => {
                if let Some(c) = out.commit {
                    state.commit = if c.len() >= 7 { c[..7].to_string() } else { c };
                }
                state.rounds_run = out.rounds;
                state.review_changed = out.review_changed;
                let b = if out.status == "disputed" {
                    Some("disputed")
                } else {
                    None
                };
                return ctx.finish(&mut state, &out.status, 0, b).map(|_| ());
            }
            RunnerOutcome::Failed { code, reason } => {
                let actual_code = if reason.contains("review rounds exhausted") {
                    0
                } else {
                    code
                };
                return ctx
                    .finish(&mut state, "blocked", actual_code, Some(&reason))
                    .map(|_| ());
            }
            RunnerOutcome::Blocked(reason) => {
                if reason.starts_with("pause_for_approval") {
                    let bname = format!("{}-{}", state.builder, ctx.task);
                    let cur_md = ctx.runs_dir.join(format!("{bname}.md"));
                    let plan_path = ctx.runs_dir.join(format!("{bname}.plan.md"));
                    if cur_md.exists() {
                        let _ = std::fs::rename(&cur_md, &plan_path);
                    }
                    let _ = std::fs::write(ctx.runs_dir.join(format!(".{bname}.read")), "");
                    eprintln!(
                        "PIPELINE plan-ready: approve {} then resume: quaqua pipeline --resume {} {}",
                        plan_path.display(),
                        ctx.task,
                        state.repo
                    );
                    state.phase = "build".to_string();
                    state.planfile = plan_path.to_string_lossy().to_string();
                    state.prompted = 0;
                    let _ = state.save(&ctx.state_file);
                    return ctx.finish(&mut state, "plan-ready", 5, None).map(|_| ());
                }

                let all_events = journal
                    .load_events(&parent_wf_id, 1)
                    .map_err(|e| (1, format!("journal error: {e}\n")))?;
                let mut completed_children = HashSet::new();
                for rec in &all_events {
                    if rec.kind == "child_completed" || rec.kind == "child_failed" {
                        if let Ok(data) =
                            serde_json::from_str::<serde_json::Value>(&rec.payload_json)
                        {
                            if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                                completed_children.insert(id.to_string());
                            }
                        }
                    }
                }

                let mut pending_child: Option<(String, String, SingleWorkerInput)> = None;
                for rec in &all_events {
                    if rec.kind == "child_started" {
                        if let Ok(data) =
                            serde_json::from_str::<serde_json::Value>(&rec.payload_json)
                        {
                            let id = data
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let wf_type = data
                                .get("workflow_type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !completed_children.contains(&id) {
                                if let Some(input_val) = data.get("input") {
                                    if let Ok(worker_in) = serde_json::from_value::<SingleWorkerInput>(
                                        input_val.clone(),
                                    ) {
                                        pending_child = Some((id, wf_type, worker_in));
                                    }
                                }
                            }
                        }
                    }
                }

                let Some((child_id, _wf_type, mut child_input)) = pending_child else {
                    return Err((
                        1,
                        format!("pipeline blocked with no pending child or approval: {reason}\n"),
                    ));
                };
                if child_id.ends_with("-plan") {
                    state.phase = "plan".to_string();
                } else if child_id.ends_with("-build") {
                    state.phase = "build".to_string();
                } else if child_id.contains("-review-") {
                    state.phase = "review".to_string();
                    if let Some(r_idx) = child_id.rfind("-review-r") {
                        if let Ok(r) = child_id[r_idx + 9..].parse::<u32>() {
                            state.round = r;
                        }
                    }
                } else if child_id.contains("-fix-") {
                    state.phase = "fix".to_string();
                    if let Some(f_idx) = child_id.rfind("-fix-r") {
                        if let Ok(r) = child_id[f_idx + 7..].parse::<u32>() {
                            state.round = r;
                        }
                    }
                }
                let _ = state.save(&ctx.state_file);


                let is_reviewer = child_id.contains("-review-");

                let el = state.start_elapsed + (now_sec() - ctx.inv_start);
                let rem = if ctx.max_sec > el {
                    ctx.max_sec - el
                } else {
                    0
                };
                let mut budget = if rem < child_input.budget_sec {
                    rem
                } else {
                    child_input.budget_sec
                };
                if budget < 1 {
                    budget = 1;
                }
                child_input.budget_sec = budget;
                let mut is_fresh_spawn = false;
                if is_reviewer {
                    if state.round == 0 {
                        state.round = 1;
                    }
                    if state.reviewer_spawned == 0 {
                        is_fresh_spawn = true;
                        let rev_report = ctx.runs_dir.join(format!("{}.md", child_input.name));
                        if rev_report.is_file()
                            && state.prompt_ts > 0
                            && file_mtime(&rev_report) < state.prompt_ts
                        {
                            ctx.archive_stale(&rev_report);
                        }

                        let sargs = vec![
                            "--review".to_string(),
                            "--base".to_string(),
                            state.head_sha.clone(),
                            child_input.name.clone(),
                            state.reviewer.clone(),
                            state.repo.clone(),
                        ];
                        let prev_pt = std::env::var("PIPELINE_TASK").ok();
                        let prev_qo = std::env::var("QUAQUA_OWNER").ok();
                        let prev_gs = std::env::var("GRID_SKIP").ok();

                        std::env::set_var("PIPELINE_TASK", &ctx.task);
                        std::env::set_var(
                            "QUAQUA_OWNER",
                            if state.lead.is_empty() {
                                "cos"
                            } else {
                                &state.lead
                            },
                        );
                        std::env::set_var("GRID_SKIP", "1");

                        let spawn_res = (|| -> Result<(), (i32, String)> {
                            let opts = crate::worker::spawn::parse_spawn_args(&sargs)?;
                            crate::worker::spawn::spawn_worker(opts, &ctx.cfg, ctx.git, ctx.herdr)
                        })();

                        if let Some(pt) = prev_pt {
                            std::env::set_var("PIPELINE_TASK", pt);
                        } else {
                            std::env::remove_var("PIPELINE_TASK");
                        }
                        if let Some(qo) = prev_qo {
                            std::env::set_var("QUAQUA_OWNER", qo);
                        } else {
                            std::env::remove_var("QUAQUA_OWNER");
                        }
                        if let Some(gs) = prev_gs {
                            std::env::set_var("GRID_SKIP", gs);
                        } else {
                            std::env::remove_var("GRID_SKIP");
                        }

                        if spawn_res.is_err() {
                            let b = format!(
                                "worker spawn failed for reviewer {} — fix the cause, then 'quaqua pipeline --resume {} {}'",
                                child_input.name, ctx.task, state.repo
                            );
                            return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                        }
                        state.reviewer_spawned = 1;
                        let _ = state.save(&ctx.state_file);

                        let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                        pcontent.push_str(&format!(
                            "- review round {}: {} spawned on {} (quaqua/{}-{} tip)\n",
                            state.round, child_input.name, state.head_sha, state.builder, ctx.task
                        ));
                        let _ = std::fs::write(&ctx.pfile, pcontent);
                    }
                } else {
                    let bname = format!("{}-{}", state.builder, ctx.task);
                    if state.builder_spawned == 0 {
                        is_fresh_spawn = true;
                        let mut sargs = Vec::new();
                        if !state.base.is_empty() {
                            sargs.push("--base".to_string());
                            sargs.push(state.base.clone());
                        }
                        sargs.push(bname.clone());
                        sargs.push(state.builder.clone());
                        sargs.push(state.repo.clone());

                        let prev_pt = std::env::var("PIPELINE_TASK").ok();
                        let prev_qo = std::env::var("QUAQUA_OWNER").ok();
                        let prev_gs = std::env::var("GRID_SKIP").ok();

                        std::env::set_var("PIPELINE_TASK", &ctx.task);
                        std::env::set_var(
                            "QUAQUA_OWNER",
                            if state.lead.is_empty() {
                                "cos"
                            } else {
                                &state.lead
                            },
                        );
                        std::env::set_var("GRID_SKIP", "1");

                        let spawn_res = (|| -> Result<(), (i32, String)> {
                            let opts = crate::worker::spawn::parse_spawn_args(&sargs)?;
                            crate::worker::spawn::spawn_worker(opts, &ctx.cfg, ctx.git, ctx.herdr)
                        })();

                        if let Some(pt) = prev_pt {
                            std::env::set_var("PIPELINE_TASK", pt);
                        } else {
                            std::env::remove_var("PIPELINE_TASK");
                        }
                        if let Some(qo) = prev_qo {
                            std::env::set_var("QUAQUA_OWNER", qo);
                        } else {
                            std::env::remove_var("QUAQUA_OWNER");
                        }
                        if let Some(gs) = prev_gs {
                            std::env::set_var("GRID_SKIP", gs);
                        } else {
                            std::env::remove_var("GRID_SKIP");
                        }

                        if spawn_res.is_err() {
                            let b = format!(
                                "worker spawn failed for builder {bname} — fix the cause, then 'quaqua pipeline --resume {} {}'",
                                ctx.task, state.repo
                            );
                            return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                        }
                        state.builder_spawned = 1;
                        let _ = state.save(&ctx.state_file);

                        let wt_path = Path::new(&state.wt);
                        let rev_out = ctx.git.run_in(wt_path, &["rev-parse", "HEAD"]);
                        let head = rev_out
                            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                            .unwrap_or_else(|_| "unknown".to_string());
                        state.base_sha = head;
                        let _ = state.save(&ctx.state_file);

                        let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                        pcontent.push_str(&format!(
                            "- builder {bname} spawned (base {})\n",
                            state.base_sha
                        ));
                        let _ = std::fs::write(&ctx.pfile, pcontent);
                    }
                }

                if is_fresh_spawn || state.phase == "spawn" {
                    let prev_qo = std::env::var("QUAQUA_OWNER").ok();
                    let prev_pr = std::env::var("PIPELINE_REPO").ok();
                    std::env::set_var("QUAQUA_OWNER", &ctx.grid_owner);
                    std::env::set_var("PIPELINE_REPO", repo_path.to_string_lossy().to_string());

                    let grid_res = if std::env::var("GRID_SKIP").as_deref() == Ok("1")
                        && std::env::var("QUAQUA_TEST").as_deref() == Ok("1")
                    {
                        Ok(crate::engine::activity::ActivityResult::Grid(
                            crate::engine::activity::GridResult::Reconciled { changed: false },
                        ))
                    } else {
                        activities.execute(&ActivityRequest::Grid(GridRequest::Reconcile {
                            tab: Some(ctx.grid_tab.clone()),
                            only: None,
                        }))
                    };

                    if let Some(qo) = prev_qo {
                        std::env::set_var("QUAQUA_OWNER", qo);
                    } else {
                        std::env::remove_var("QUAQUA_OWNER");
                    }
                    if let Some(pr) = prev_pr {
                        std::env::set_var("PIPELINE_REPO", pr);
                    } else {
                        std::env::remove_var("PIPELINE_REPO");
                    }

                    if let Err(e) = grid_res {
                        eprintln!("warning: grid check failed ({e}); arrange panes manually");
                        let b = format!(
                            "grid check failed for {} — arrange the pane per rules/_all/fanout-agent-grid.md, then 'quaqua pipeline --resume {} {}'",
                            child_input.name, ctx.task, state.repo
                        );
                        state.prompted = 0;
                        let _ = state.save(&ctx.state_file);
                        let code = if is_fresh_spawn { 5 } else { 1 };
                        return ctx
                            .finish(&mut state, "blocked", code, Some(&b))
                            .map(|_| ());
                    }
                }

                let child_wf_id = format!("pipeline:{}:{}", ctx.task, child_id);
                let child_events = journal
                    .load_events(&child_wf_id, 1)
                    .map_err(|e| (1, format!("journal error: {e}\n")))?;

                let mut child_completed_out = None;
                let mut child_failed_out = None;

                for rec in &child_events {
                    if rec.kind == "workflow_completed" {
                        if let Ok(out) =
                            serde_json::from_str::<SingleWorkerOutput>(&rec.payload_json)
                        {
                            if out.exit == 0 {
                                child_completed_out = Some(out);
                            }
                        }
                    } else if rec.kind == "workflow_failed" {
                        if let Ok(val) =
                            serde_json::from_str::<serde_json::Value>(&rec.payload_json)
                        {
                            let code = val.get("code").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                            let reason = val
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .unwrap_or("failed")
                                .to_string();
                            child_failed_out = Some((code, reason));
                        }
                    }
                }

                let child_report_path = ctx.runs_dir.join(format!("{}.md", child_input.name));
                if child_report_path.is_file()
                    && state.prompt_ts > 0
                    && file_mtime(&child_report_path) < state.prompt_ts
                {
                    ctx.archive_stale(&child_report_path);
                }
                let child_outcome = if let Some(out) = child_completed_out {
                    RunnerOutcome::Completed(out)
                } else if let Some((code, reason)) = child_failed_out {
                    RunnerOutcome::Failed { code, reason }
                } else {
                    let _prev_b = std::env::var("PROMPT_WORKER_BUDGET_SEC").ok();
                    std::env::set_var("PROMPT_WORKER_BUDGET_SEC", budget.to_string());

                    let already_delivered = child_report_path.is_file() && {
                        let content =
                            std::fs::read_to_string(&child_report_path).unwrap_or_default();
                        crate::worker::prompt::watchdog::validate_report_content(
                            &content,
                            is_reviewer,
                        )
                        .is_ok()
                            || content
                                .lines()
                                .any(|l| l.trim().starts_with("## Challenge"))
                    };

                    let res = if already_delivered {
                        RunnerOutcome::Completed(
                            SingleWorkerOutput::new(0, &child_report_path).unwrap(),
                        )
                    } else if child_events.is_empty() {
                        if state.prompted == 0 {
                            if std::env::var("GRID_SKIP").as_deref() != Ok("1")
                                || (std::env::var("PIPELINE_TASK").is_err()
                                    && std::env::var("QUAQUA_TEST").as_deref() != Ok("1"))
                            {
                                let prev_qo = std::env::var("QUAQUA_OWNER").ok();
                                let prev_pr = std::env::var("PIPELINE_REPO").ok();
                                std::env::set_var("QUAQUA_OWNER", &ctx.grid_owner);
                                std::env::set_var(
                                    "PIPELINE_REPO",
                                    repo_path.to_string_lossy().to_string(),
                                );

                                let grid_check_res =
                                    activities.execute(&ActivityRequest::Grid(GridRequest::Plan {
                                        tab: Some(ctx.grid_tab.clone()),
                                    }));

                                if let Some(qo) = prev_qo {
                                    std::env::set_var("QUAQUA_OWNER", qo);
                                } else {
                                    std::env::remove_var("QUAQUA_OWNER");
                                }
                                if let Some(pr) = prev_pr {
                                    std::env::set_var("PIPELINE_REPO", pr);
                                } else {
                                    std::env::remove_var("PIPELINE_REPO");
                                }

                                if let Err(e) = grid_check_res {
                                    eprintln!(
                                        "warning: grid check failed ({e}); arrange panes manually"
                                    );
                                    let b = format!(
                                        "grid check failed for {} ({e}); arrange panes manually before prompting",
                                        child_input.name
                                    );
                                    state.prompted = 0;
                                    let _ = state.save(&ctx.state_file);
                                    return ctx
                                        .finish(&mut state, "blocked", 1, Some(&b))
                                        .map(|_| ());
                                }
                            }

                            if child_report_path.exists() {
                                ctx.archive_stale(&child_report_path);
                            }

                            let _ = std::fs::write(
                                ctx.runs_dir.join(format!(".{}.budget", child_input.name)),
                                format!("{budget}\n"),
                            );
                            let _ = std::fs::remove_file(
                                ctx.runs_dir.join(format!(".{}.elapsed", child_input.name)),
                            );
                            let _ = std::fs::remove_file(
                                ctx.runs_dir.join(format!(".{}.grace", child_input.name)),
                            );
                            let _ = std::fs::remove_file(
                                ctx.runs_dir.join(format!(".{}.nudged", child_input.name)),
                            );
                            let _ = std::fs::remove_file(
                                ctx.runs_dir.join(format!(".{}.autoext", child_input.name)),
                            );

                            let prompt_body = format!(
                                    "{}. When done, write summary (changes, test cmd+result, risks) to a temp file and rename it to ~/quaqua/runs/{}.md (publish it atomically, never a partial file); reply only the path.",
                                    child_input.brief, child_input.name
                                );
                            let prompt_res =
                                activities.execute(&ActivityRequest::Herdr(HerdrRequest::Prompt {
                                    name: child_input.name.clone(),
                                    task: prompt_body,
                                }));
                            if let Err(e) = prompt_res {
                                let b = format!(
                                        "prompt failed for {} ({e}) — check the worker, then 'quaqua pipeline --resume {} {}'",
                                        child_input.name, ctx.task, state.repo
                                    );
                                state.prompted = 0;
                                let _ = state.save(&ctx.state_file);
                                return ctx.finish(&mut state, "blocked", 1, Some(&b)).map(|_| ());
                            }
                            state.prompted = 1;
                            state.prompt_ts = now_sec();
                            let _ = state.save(&ctx.state_file);
                        } else if child_report_path.is_file()
                            && file_mtime(&child_report_path) < state.prompt_ts
                        {
                            ctx.archive_stale(&child_report_path);
                        }

                        let child_wf = SingleWorkerWorkflow::new();
                        let is_fallback = child_input.kind.contains("opencode")
                            || child_input.kind.contains("omp");
                        let config = watchdog_config_from_env(is_reviewer, is_fallback);
                        eprintln!(
                            "waiting for {} (poll {}s, stall {}s, idle {}s, budget {}s, cap {}s)",
                            child_report_path.display(),
                            config.poll_sec,
                            config.stall_sec,
                            config.idle_sec,
                            budget,
                            config.hard_cap_sec
                        );
                        let child_state = SingleWorkerState::new(child_input.clone())
                            .with_report_path(&child_report_path)
                            .with_config(config);
                        runner::run_until_blocked(
                            &child_wf,
                            &child_wf_id,
                            Some(child_state),
                            &mut journal,
                            &mut activities,
                            &mut clock,
                            &runner_opts,
                            Vec::new(),
                        )
                        .map_err(|e| (1, format!("child runner error: {e}\n")))?
                    } else {
                        if child_report_path.is_file()
                            && file_mtime(&child_report_path) < state.prompt_ts
                        {
                            ctx.archive_stale(&child_report_path);
                        }
                        let child_wf = SingleWorkerWorkflow::new();
                        let is_fallback = child_input.kind.contains("opencode")
                            || child_input.kind.contains("omp");
                        let config = watchdog_config_from_env(is_reviewer, is_fallback);
                        eprintln!(
                            "waiting for {} (poll {}s, stall {}s, idle {}s, budget {}s, cap {}s)",
                            child_report_path.display(),
                            config.poll_sec,
                            config.stall_sec,
                            config.idle_sec,
                            budget,
                            config.hard_cap_sec
                        );
                        let child_state = SingleWorkerState::new(child_input.clone())
                            .with_report_path(&child_report_path)
                            .with_config(config);
                        runner::run_until_blocked(
                            &child_wf,
                            &child_wf_id,
                            Some(child_state),
                            &mut journal,
                            &mut activities,
                            &mut clock,
                            &runner_opts,
                            Vec::new(),
                        )
                        .map_err(|e| (1, format!("child resume error: {e}\n")))?
                    };

                    if let Some(pb) = _prev_b {
                        std::env::set_var("PROMPT_WORKER_BUDGET_SEC", pb);
                    } else {
                        std::env::remove_var("PROMPT_WORKER_BUDGET_SEC");
                    }
                    res
                };

                match child_outcome {
                    RunnerOutcome::Completed(child_out) => {
                        if child_out.exit != 0 {
                            if child_out.exit == 6 {
                                eprintln!(
                                    "WATCHDOG malformed-report | name={} missing=verdict report={}",
                                    child_input.name,
                                    child_report_path.display()
                                );
                            } else {
                                eprintln!("WATCHDOG exit={}", child_out.exit);
                            }
                            let b = format!("watchdog exit {}", child_out.exit);
                            return ctx
                                .finish(&mut state, "blocked", child_out.exit, Some(&b))
                                .map(|_| ());
                        }

                        let _ = std::fs::write(
                            ctx.runs_dir.join(format!(".{}.read", child_input.name)),
                            "",
                        );
                        state.prompted = 0;
                        let _ = state.save(&ctx.state_file);

                        let mut child_output_json = serde_json::to_value(&child_out).unwrap();

                        if child_id.ends_with("-plan") {
                            let planfile =
                                ctx.runs_dir.join(format!("{}.plan.md", child_input.name));
                            if child_report_path.exists() {
                                let _ = std::fs::rename(&child_report_path, &planfile);
                            }
                            state.planfile = planfile.to_string_lossy().to_string();

                            let mut pcontent =
                                std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                            pcontent.push_str(&format!(
                                "- plan: {}\n",
                                planfile.file_name().and_then(|n| n.to_str()).unwrap_or("")
                            ));
                            let _ = std::fs::write(&ctx.pfile, pcontent);
                            let _ = state.save(&ctx.state_file);
                            child_output_json["report_path"] =
                                 serde_json::Value::String(planfile.to_string_lossy().to_string());
                        } else if child_id.ends_with("-build") {
                            let build_md =
                                ctx.runs_dir.join(format!("{}.build.md", child_input.name));
                            if child_report_path.exists() {
                                let _ = std::fs::rename(&child_report_path, &build_md);
                            }

                            let wt_path = Path::new(&state.wt);
                            let rev_out = ctx.git.run_in(wt_path, &["rev-parse", "HEAD"]);
                            let head = match rev_out {
                                Ok(o) if o.status.success() => {
                                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                                }
                                _ => "?".to_string(),
                            };
                            let st_out = ctx.git.run_in(wt_path, &["status", "--porcelain"]);
                            let dirty = match st_out {
                                Ok(o) if o.status.success() => {
                                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                                }
                                _ => "?".to_string(),
                            };

                            if head == "?" || dirty == "?" {
                                let b = format!(
                                    "cannot inspect {} (git rev-parse/status failed) — fix the worktree, then 'quaqua pipeline --resume {} {}'",
                                    state.wt, ctx.task, state.repo
                                );
                                return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                            }

                            if !dirty.is_empty() {
                                let mut pcontent =
                                    std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                                pcontent.push_str(&format!(
                                    "- uncommitted/untracked in {}:\n{}\n",
                                    state.wt, dirty
                                ));
                                let _ = std::fs::write(&ctx.pfile, pcontent);
                                let b = format!(
                                    "worktree dirty — the files above are missing from quaqua/{}-{}, so the gate and the reviewer would see a different snapshot. Commit or clean them, then 'quaqua pipeline --resume {} {}'",
                                    state.builder, ctx.task, ctx.task, state.repo
                                );
                                return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                            }

                            let mut skip_review = false;
                            let rep_content =
                                std::fs::read_to_string(&build_md).unwrap_or_default();
                            let has_no_changes = report_has_outcome_no_changes(&rep_content);

                            if head == state.base_sha {
                                if !has_no_changes {
                                    let base7 = if state.base_sha.len() >= 7 {
                                        &state.base_sha[..7]
                                    } else {
                                        &state.base_sha
                                    };
                                    let b = format!(
                                        "builder produced no commits on quaqua/{}-{} (HEAD == base {base7}) and did not declare 'Outcome: no changes' — commit the work (or fix the report), then 'quaqua pipeline --resume {} {}'",
                                        state.builder, ctx.task, ctx.task, state.repo
                                    );
                                    return ctx
                                        .finish(&mut state, "blocked", 3, Some(&b))
                                        .map(|_| ());
                                }
                                skip_review = true;
                            } else {
                                state.head_sha = head.clone();
                                state.commit = if head.len() >= 7 {
                                    head[..7].to_string()
                                } else {
                                    head.clone()
                                };

                                let mut pcontent =
                                    std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                                pcontent.push_str(&format!(
                                    "- build: commit {} ({}.build.md)\n",
                                    state.commit, child_input.name
                                ));
                                let _ = std::fs::write(&ctx.pfile, pcontent);

                                if state.review == "auto" {
                                    let numstat_out = ctx.git.run_in(
                                        wt_path,
                                        &["diff", "--numstat", &state.base_sha, &state.head_sha],
                                    );
                                    let mut total_lines = 0;
                                    if let Ok(o) = numstat_out {
                                        for line in String::from_utf8_lossy(&o.stdout).lines() {
                                            let parts: Vec<&str> =
                                                line.split_whitespace().collect();
                                            if parts.len() >= 2 {
                                                let add: usize = parts[0].parse().unwrap_or(0);
                                                let del: usize = parts[1].parse().unwrap_or(0);
                                                total_lines += add + del;
                                            }
                                        }
                                    }
                                    let names_out = ctx.git.run_in(
                                        wt_path,
                                        &["diff", "--name-only", &state.base_sha, &state.head_sha],
                                    );
                                    let mut dirs = std::collections::BTreeSet::new();
                                    if let Ok(o) = names_out {
                                        for line in String::from_utf8_lossy(&o.stdout).lines() {
                                            let path = line.trim();
                                            if let Some(idx) = path.find('/') {
                                                dirs.insert(path[..idx].to_string());
                                            } else if !path.is_empty() {
                                                dirs.insert(".".to_string());
                                            }
                                        }
                                    }
                                    let mut pcontent =
                                        std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                                    pcontent.push_str(&format!(
                                        "- review gate: {total_lines} changed lines, {} top-level dirs (run if >{} or >1)\n",
                                        dirs.len(),
                                        ctx.review_lines
                                    ));
                                    if total_lines > ctx.review_lines || dirs.len() > 1 {
                                        state.review_run = "run".to_string();
                                    } else {
                                        state.review_run = "skip".to_string();
                                        skip_review = true;
                                        pcontent.push_str(&format!(
                                            "- review: skipped (gate: {})\n",
                                            state.review
                                        ));
                                    }
                                    let _ = std::fs::write(&ctx.pfile, pcontent);
                                } else if state.review == "always" {
                                    state.review_run = "run".to_string();
                                } else {
                                    state.review_run = "skip".to_string();
                                    skip_review = true;
                                    let mut pcontent =
                                        std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                                    pcontent.push_str(&format!(
                                        "- review: skipped (gate: {})\n",
                                        state.review
                                    ));
                                    let _ = std::fs::write(&ctx.pfile, pcontent);
                                }
                            }
                            let _ = state.save(&ctx.state_file);

                            child_output_json["report_path"] =
                                serde_json::Value::String(build_md.to_string_lossy().to_string());
                            child_output_json["skip_review"] =
                                serde_json::Value::Bool(skip_review);
                            child_output_json["commit"] =
                                serde_json::Value::String(state.head_sha.clone());
                        } else if is_reviewer {
                            let rev_content =
                                std::fs::read_to_string(&child_report_path).unwrap_or_default();

                            let cleanup_reviewer = || {
                                let prev_pt = std::env::var("PIPELINE_TASK").ok();
                                let prev_gs = std::env::var("GRID_SKIP").ok();
                                std::env::set_var("PIPELINE_TASK", &ctx.task);
                                std::env::set_var("GRID_SKIP", "1");
                                let _ = crate::worker::cleanup::cleanup_worker(
                                    &child_input.name,
                                    Path::new(&state.repo),
                                    ctx.git,
                                    ctx.herdr,
                                );
                                if let Some(pt) = prev_pt {
                                    std::env::set_var("PIPELINE_TASK", pt);
                                } else {
                                    std::env::remove_var("PIPELINE_TASK");
                                }
                                if let Some(gs) = prev_gs {
                                    std::env::set_var("GRID_SKIP", gs);
                                } else {
                                    std::env::remove_var("GRID_SKIP");
                                }
                                let _ = activities.execute(&ActivityRequest::Grid(
                                    GridRequest::Reconcile {
                                        tab: Some(ctx.grid_tab.clone()),
                                        only: None,
                                    },
                                ));
                            };
                            cleanup_reviewer();

                            let mut verdict_line = None;
                            for line in rev_content.lines() {
                                let t = line.trim();
                                if t.to_ascii_lowercase().starts_with("verdict:") {
                                    verdict_line = Some(t.to_string());
                                }
                            }
                            let mut pcontent =
                                std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                            pcontent.push_str(&format!(
                                "- review round {}: {} ({}.md)\n",
                                state.round,
                                verdict_line.as_deref().unwrap_or("NO VERDICT LINE"),
                                child_input.name
                            ));
                            let _ = std::fs::write(&ctx.pfile, pcontent);

                            state.reviewer_spawned = 0;
                            let _ = state.save(&ctx.state_file);

                            child_output_json["report_path"] = serde_json::Value::String(
                                child_report_path.to_string_lossy().to_string(),
                            );
                        } else if child_id.contains("-fix-") {
                            let fix_md = ctx
                                .runs_dir
                                .join(format!("{}.fix{}.md", child_input.name, state.round));
                            if child_report_path.exists() {
                                let _ = std::fs::rename(&child_report_path, &fix_md);
                            }

                            let prev_sha = state.head_sha.clone();
                            let wt_path = Path::new(&state.wt);
                            let rev_out = ctx.git.run_in(wt_path, &["rev-parse", "HEAD"]);
                            let head = match rev_out {
                                Ok(o) if o.status.success() => {
                                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                                }
                                _ => "?".to_string(),
                            };
                            let st_out = ctx.git.run_in(wt_path, &["status", "--porcelain"]);
                            let dirty = match st_out {
                                Ok(o) if o.status.success() => {
                                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                                }
                                _ => "?".to_string(),
                            };

                            if head == "?" || dirty == "?" {
                                let b = format!(
                                    "cannot inspect {} (git rev-parse/status failed) — fix the worktree, then 'quaqua pipeline --resume {} {}'",
                                    state.wt, ctx.task, state.repo
                                );
                                return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                            }

                            if !dirty.is_empty() {
                                let mut pcontent =
                                    std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                                pcontent.push_str(&format!(
                                    "- uncommitted/untracked in {}:\n{}\n",
                                    state.wt, dirty
                                ));
                                let _ = std::fs::write(&ctx.pfile, pcontent);
                                let b = format!(
                                    "worktree dirty — the files above are missing from quaqua/{}-{}, so the gate and the reviewer would see a different snapshot. Commit or clean them, then 'quaqua pipeline --resume {} {}'",
                                    state.builder, ctx.task, ctx.task, state.repo
                                );
                                return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                            }

                            if head == prev_sha {
                                let prev7 = if prev_sha.len() >= 7 {
                                    &prev_sha[..7]
                                } else {
                                    &prev_sha
                                };
                                let b = format!(
                                    "fix produced no commits past {prev7} on quaqua/{}-{} — a no-op answer to findings belongs in '## Challenge'. Commit the fixes, then 'quaqua pipeline --resume {} {}'",
                                    state.builder, ctx.task, ctx.task, state.repo
                                );
                                return ctx.finish(&mut state, "blocked", 3, Some(&b)).map(|_| ());
                            }

                            state.head_sha = head.clone();
                            state.commit = if head.len() >= 7 {
                                head[..7].to_string()
                            } else {
                                head.clone()
                            };

                            let mut pcontent =
                                std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
                            pcontent.push_str(&format!(
                                "- fix {}: commit {} ({}.fix{}.md)\n",
                                state.round, state.commit, child_input.name, state.round
                            ));
                            let _ = std::fs::write(&ctx.pfile, pcontent);
                            let _ = state.save(&ctx.state_file);

                            child_output_json["report_path"] =
                                serde_json::Value::String(fix_md.to_string_lossy().to_string());
                            child_output_json["commit"] =
                                serde_json::Value::String(state.head_sha.clone());
                        }

                        parent_outcome = runner::run_until_blocked(
                            &workflow,
                            &parent_wf_id,
                            Some(PipelineWorkflowState::new(input.clone())),
                            &mut journal,
                            &mut activities,
                            &mut clock,
                            &runner_opts,
                            vec![WorkflowEvent::ChildCompleted {
                                id: child_id.clone(),
                                output: child_output_json,
                            }],
                        )
                        .map_err(|e| (1, format!("parent resume error: {e}\n")))?;
                    }
                    RunnerOutcome::Failed { code, reason: _ } => {
                        eprintln!("WATCHDOG exit={code}");
                        let b = format!("watchdog exit {code}");
                        return ctx
                            .finish(&mut state, "blocked", code, Some(&b))
                            .map(|_| ());
                    }
                    RunnerOutcome::Blocked(reason) => {
                        return ctx
                            .finish(&mut state, "blocked", 5, Some(&reason))
                            .map(|_| ());
                    }
                }
            }
        }
    }
}

/// Deterministic engine workflow implementing plan gate -> build child -> review child -> fix cycle.
#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineWorkflow;

impl PipelineWorkflow {
    pub fn new() -> Self {
        Self
    }

    fn builder_worker_name(&self, state: &PipelineWorkflowState) -> String {
        format!("{}-{}", state.input.builder, state.input.task)
    }

    fn reviewer_worker_name(&self, state: &PipelineWorkflowState) -> String {
        format!(
            "{}-{}r{}",
            state.input.reviewer, state.input.task, state.round
        )
    }

    fn plan_child_id(&self, state: &PipelineWorkflowState) -> String {
        format!("{}-{}-plan", state.input.builder, state.input.task)
    }

    fn build_child_id(&self, state: &PipelineWorkflowState) -> String {
        format!("{}-{}-build", state.input.builder, state.input.task)
    }

    fn review_child_id(&self, state: &PipelineWorkflowState) -> String {
        format!(
            "{}-{}-review-r{}",
            state.input.reviewer, state.input.task, state.round
        )
    }

    fn fix_child_id(&self, state: &PipelineWorkflowState) -> String {
        format!(
            "{}-{}-fix-r{}",
            state.input.builder, state.input.task, state.round
        )
    }

    fn build_plan_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.plan_child_id(state);
        let worker_name = self.builder_worker_name(state);
        let plan_prompt = format!(
            "{}\n\nPipeline phase: PLAN — write only the implementation plan (steps, files to touch, risks) as your runs file; do not change code yet. The plan is approved before you build. If the premise is wrong, write '## Challenge' with evidence instead of planning around it.",
            state.input.brief
        );
        let worker_input = SingleWorkerInput::new(
            &worker_name,
            &state.input.builder,
            &state.input.repo,
            &plan_prompt,
            state.input.plan_budget_sec,
            None,
        );
        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        (child_id, input_val)
    }

    fn build_build_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.build_child_id(state);
        let worker_name = self.builder_worker_name(state);
        let planfile = state.planfile.as_deref().unwrap_or("plan");
        let build_prompt = format!(
            "Approved plan: {planfile}\n\n{}\n\nPipeline phase: BUILD — implement the plan, keep tests green, commit on quaqua/{worker_name}. A premise that is wrong is NOT built — dispute it under '## Challenge' in your runs file with evidence. If the task is already satisfied with no code changes needed, say 'Outcome: no changes' in your runs file.",
            state.input.brief
        );
        let worker_input = SingleWorkerInput::new(
            &worker_name,
            &state.input.builder,
            &state.input.repo,
            &build_prompt,
            state.input.build_budget_sec,
            None,
        );
        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        (child_id, input_val)
    }

    fn build_review_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.review_child_id(state);
        let worker_name = self.reviewer_worker_name(state);
        let bname = self.builder_worker_name(state);
        let base_ref = state
            .input
            .base_sha
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if !state.input.base.is_empty() {
                    Some(state.input.base.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("HEAD");
        let review_prompt = format!(
            "Goal: review changes for task '{}' (round {})\nHard constraints: review only diff between base {base_ref} and branch quaqua/{bname}; do not edit code; foreground tests only.\nCurrent design: brief in {}\nEvidence: runs file MUST end with exactly one line: 'Verdict: clean' or 'Verdict: <N> findings' (findings numbered under ## Findings; dispute with ## Challenge).\nPipeline phase: REVIEW (round {})",
            state.input.task, state.round, state.input.brief_path, state.round
        );
        let worker_input = SingleWorkerInput::new(
            &worker_name,
            &state.input.reviewer,
            &state.input.repo,
            &review_prompt,
            state.input.review_budget_sec,
            None,
        );
        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        (child_id, input_val)
    }

    fn build_fix_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.fix_child_id(state);
        let worker_name = self.builder_worker_name(state);
        let revname = self.reviewer_worker_name(state);
        let fallback_findings = format!("{revname}.md");
        let findings_file = state
            .last_review_report
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&fallback_findings);
        let fix_prompt = format!(
            "Goal: resolve the {} review findings for task '{}'\nHard constraints: stay in your worktree on quaqua/{worker_name}; commit each fix; keep tests green. A finding that is wrong is NOT fixed — dispute it under '## Challenge' in your runs file with evidence.\nCurrent design (open to challenge): reviewer {revname} reported {} findings — full list in {} (readable); your worktree and branch are unchanged.\nUnknowns: which findings are real — verify each against the code before changing anything.\nEvidence: committed fixes + test output, or '## Challenge' per wrong finding.\nPipeline phase: FIX (after review round {})",
            state.last_findings, state.input.task, state.last_findings, findings_file, state.round
        );
        let worker_input = SingleWorkerInput::new(
            &worker_name,
            &state.input.builder,
            &state.input.repo,
            &fix_prompt,
            state.input.fix_budget_sec,
            None,
        );
        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        (child_id, input_val)
    }


    fn handle_review_verdict(
        &self,
        state: &mut PipelineWorkflowState,
        findings: u32,
        is_challenge: bool,
    ) -> StepDecision<PipelineOutput> {
        if is_challenge {
            state.phase = PipelinePhase::Disputed;
            return StepDecision::Complete(PipelineOutput {
                status: "disputed".to_string(),
                rounds: state.rounds_run,
                review_changed: state.review_changed,
                commit: state.head_sha.clone(),
            });
        }

        state.last_findings = findings;
        if findings == 0 {
            state.phase = PipelinePhase::Complete;
            return StepDecision::Complete(PipelineOutput {
                status: "clean".to_string(),
                rounds: state.rounds_run,
                review_changed: state.review_changed,
                commit: state.head_sha.clone(),
            });
        }

        state.review_changed = true;
        if state.round >= state.input.max_rounds {
            let reason = format!(
                "review rounds exhausted ({} rounds, {} findings remain) — Lead decides how to proceed",
                state.round, state.last_findings
            );
            return block(
                state,
                PipelinePhase::Review,
                state::BlockedKind::RoundsExhausted,
                5,
                reason,
            );
        }

        state.phase = PipelinePhase::Fix;
        let (child_id, input_val) = self.build_fix_child(state);
        state.active_child_id = Some(child_id.clone());
        StepDecision::StartChild {
            id: child_id,
            workflow_type: "single_worker".to_string(),
            input: input_val,
        }
    }
}
fn block(
    state: &mut PipelineWorkflowState,
    from: PipelinePhase,
    kind: state::BlockedKind,
    code: i32,
    reason: impl Into<String>,
) -> StepDecision<PipelineOutput> {
    let reason_str = reason.into();
    state.phase = PipelinePhase::Blocked;
    state.blocked_from = Some(from);
    state.blocked_kind = Some(kind);
    state.blocked_code = code;
    state.blocked_reason = Some(reason_str.clone());
    StepDecision::PauseForApproval {
        reason: format!("blocked {code}: {reason_str}"),
    }
}


impl Workflow for PipelineWorkflow {
    type Input = PipelineInput;
    type State = PipelineWorkflowState;
    type Output = PipelineOutput;

    fn step(
        &self,
        state: &mut Self::State,
        _ctx: &mut WorkflowContext,
        event: Option<WorkflowEvent>,
    ) -> StepDecision<Self::Output> {
        match event {
            None => match state.phase {
                PipelinePhase::Plan => {
                    let (child_id, input_val) = self.build_plan_child(state);
                    state.active_child_id = Some(child_id.clone());
                    StepDecision::StartChild {
                        id: child_id,
                        workflow_type: "single_worker".to_string(),
                        input: input_val,
                    }
                }
                PipelinePhase::PlanReadyGate => {
                    let reason = format!(
                        "approve {} then resume",
                        state.planfile.as_deref().unwrap_or("plan")
                    );
                    StepDecision::PauseForApproval { reason }
                }
                PipelinePhase::Build => {
                    let (child_id, input_val) = self.build_build_child(state);
                    state.active_child_id = Some(child_id.clone());
                    StepDecision::StartChild {
                        id: child_id,
                        workflow_type: "single_worker".to_string(),
                        input: input_val,
                    }
                }
                PipelinePhase::Review => {
                    if state.round == 0 {
                        state.round = 1;
                    }
                    let (child_id, input_val) = self.build_review_child(state);
                    state.active_child_id = Some(child_id.clone());
                    StepDecision::StartChild {
                        id: child_id,
                        workflow_type: "single_worker".to_string(),
                        input: input_val,
                    }
                }
                PipelinePhase::Fix => {
                    let (child_id, input_val) = self.build_fix_child(state);
                    state.active_child_id = Some(child_id.clone());
                    StepDecision::StartChild {
                        id: child_id,
                        workflow_type: "single_worker".to_string(),
                        input: input_val,
                    }
                }
                PipelinePhase::Complete => StepDecision::Complete(PipelineOutput {
                    status: "clean".to_string(),
                    rounds: state.rounds_run,
                    review_changed: state.review_changed,
                    commit: state.head_sha.clone(),
                }),
                PipelinePhase::Failed => StepDecision::Fail {
                    code: 1,
                    reason: "pipeline failed".to_string(),
                },
                PipelinePhase::Disputed => StepDecision::Complete(PipelineOutput {
                    status: "disputed".to_string(),
                    rounds: state.rounds_run,
                    review_changed: state.review_changed,
                    commit: state.head_sha.clone(),
                }),
                PipelinePhase::Blocked => {
                    let code = state.blocked_code;
                    let r = state
                        .blocked_reason
                        .as_deref()
                        .unwrap_or("waiting for approval to resume");
                    StepDecision::PauseForApproval {
                        reason: format!("blocked {code}: {r}"),
                    }
                }
            },
            Some(WorkflowEvent::ApprovalResolved { approved, reason }) => {
                match state.phase {
                    PipelinePhase::PlanReadyGate => {
                        if approved {
                            state.phase = PipelinePhase::Build;
                            let (child_id, input_val) = self.build_build_child(state);
                            state.active_child_id = Some(child_id.clone());
                            StepDecision::StartChild {
                                id: child_id,
                                workflow_type: "single_worker".to_string(),
                                input: input_val,
                            }
                        } else {
                            state.phase = PipelinePhase::Failed;
                            StepDecision::Fail {
                                code: 1,
                                reason: reason.unwrap_or_else(|| "plan approval rejected".to_string()),
                            }
                        }
                    }
                    PipelinePhase::Blocked => {
                        if !approved {
                            state.phase = PipelinePhase::Failed;
                            return StepDecision::Fail {
                                code: 1,
                                reason: reason.unwrap_or_else(|| "blocked gate aborted".to_string()),
                            };
                        }
                        match state.blocked_kind {
                            Some(state::BlockedKind::RoundsExhausted) => {
                                if let Some(r) = reason.as_deref() {
                                    if let Ok(directive) = serde_json::from_str::<state::ResumeDirective>(r) {
                                        state.input.max_rounds = state.input.max_rounds.max(directive.max_rounds);
                                    }
                                }
                                if state.round < state.input.max_rounds {
                                    state.phase = PipelinePhase::Fix;
                                    state.blocked_kind = None;
                                    state.blocked_from = None;
                                    state.blocked_code = 0;
                                    state.blocked_reason = None;
                                    let (child_id, input_val) = self.build_fix_child(state);
                                    state.active_child_id = Some(child_id.clone());
                                    StepDecision::StartChild {
                                        id: child_id,
                                        workflow_type: "single_worker".to_string(),
                                        input: input_val,
                                    }
                                } else {
                                    let reason = format!(
                                        "review rounds exhausted ({} rounds, {} findings remain) — Lead decides how to proceed",
                                        state.round, state.last_findings
                                    );
                                    block(
                                        state,
                                        PipelinePhase::Review,
                                        state::BlockedKind::RoundsExhausted,
                                        5,
                                        reason,
                                    )
                                }
                            }
                            Some(state::BlockedKind::Restart) | None => {
                                let from_phase = state.blocked_from.unwrap_or(PipelinePhase::Plan);
                                state.phase = from_phase;
                                state.blocked_kind = None;
                                state.blocked_from = None;
                                state.blocked_code = 0;
                                state.blocked_reason = None;
                                let (child_id, input_val) = match from_phase {
                                    PipelinePhase::Plan => self.build_plan_child(state),
                                    PipelinePhase::Build => self.build_build_child(state),
                                    PipelinePhase::Review => self.build_review_child(state),
                                    PipelinePhase::Fix => self.build_fix_child(state),
                                    _ => self.build_plan_child(state),
                                };
                                state.active_child_id = Some(child_id.clone());
                                StepDecision::StartChild {
                                    id: child_id,
                                    workflow_type: "single_worker".to_string(),
                                    input: input_val,
                                }
                            }
                        }
                    }
                    _ => StepDecision::Continue,
                }
            }
            Some(WorkflowEvent::ChildCompleted { id, output }) => {
                if let Some(active_id) = &state.active_child_id {
                    if id != *active_id {
                        return StepDecision::Continue;
                    }
                }
                state.active_child_id = None;
                if let Some(skip) = output.get("skip_review").and_then(|v| v.as_bool()) {
                    if skip {
                        state.review_skip = true;
                    }
                }
                if let Some(commit) = output.get("commit").and_then(|v| v.as_str()) {
                    if !commit.is_empty() {
                        state.head_sha = Some(commit.to_string());
                    }
                }

                let worker_output: SingleWorkerOutput = match serde_json::from_value(output) {
                    Ok(o) => o,
                    Err(err) => {
                        let reason = format!(
                            "child worker {id} returned invalid output contract: {err}"
                        );
                        return block(state, state.phase, state::BlockedKind::Restart, 6, reason);
                    }
                };

                if worker_output.exit != 0 {
                    let reason = format!(
                        "child worker {id} failed with exit {}",
                        worker_output.exit
                    );
                    return block(
                        state,
                        state.phase,
                        state::BlockedKind::Restart,
                        worker_output.exit,
                        reason,
                    );
                }

                let rp = worker_output.report_path.to_string_lossy().to_string();

                match state.phase {
                    PipelinePhase::Plan => {
                        state.planfile = Some(rp.clone());
                        state.pending_report_read = Some(rp.clone());
                        StepDecision::ScheduleActivity {
                            id: format!("read-plan-{id}"),
                            activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                path: rp,
                            }),
                        }
                    }
                    PipelinePhase::Build => {
                        state.pending_report_read = Some(rp.clone());
                        StepDecision::ScheduleActivity {
                            id: format!("read-build-{id}"),
                            activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                path: rp,
                            }),
                        }
                    }
                    PipelinePhase::Review => {
                        state.rounds_run = state.round;
                        state.last_review_report = Some(rp.clone());
                        state.pending_report_read = Some(rp.clone());
                        StepDecision::ScheduleActivity {
                            id: format!("read-report-r{}", state.round),
                            activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                path: rp,
                            }),
                        }
                    }
                    PipelinePhase::Fix => {
                        state.pending_report_read = Some(rp.clone());
                        StepDecision::ScheduleActivity {
                            id: format!("read-fix-{id}"),
                            activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                path: rp,
                            }),
                        }
                    }
                    _ => StepDecision::Continue,
                }
            }
            Some(WorkflowEvent::ActivityCompleted { id: _, result }) => {
                if let Some(pending_path) = state.pending_report_read.take() {
                    match result {
                        ActivityResult::ReportFile(ReportFileResult::Content(content)) => {
                            match state.phase {
                                PipelinePhase::Plan => {
                                    if content
                                        .lines()
                                        .any(|l| l.trim().starts_with("## Challenge"))
                                    {
                                        state.phase = PipelinePhase::Disputed;
                                        return StepDecision::Complete(PipelineOutput {
                                            status: "disputed".to_string(),
                                            rounds: 0,
                                            review_changed: false,
                                            commit: None,
                                        });
                                    }
                                    if let Some(reason) = report_is_unfinished(&content) {
                                        return block(
                                            state,
                                            PipelinePhase::Plan,
                                            state::BlockedKind::Restart,
                                            5,
                                            format!("plan report unfinished: {reason}"),
                                        );
                                    }
                                    state.phase = PipelinePhase::PlanReadyGate;
                                    let reason = format!(
                                        "approve {} then resume",
                                        state.planfile.as_deref().unwrap_or(&pending_path)
                                    );
                                    StepDecision::PauseForApproval { reason }
                                }
                                PipelinePhase::Build => {
                                    if content
                                        .lines()
                                        .any(|l| l.trim().starts_with("## Challenge"))
                                    {
                                        state.phase = PipelinePhase::Disputed;
                                        return StepDecision::Complete(PipelineOutput {
                                            status: "disputed".to_string(),
                                            rounds: 0,
                                            review_changed: false,
                                            commit: None,
                                        });
                                    }
                                    if let Some(reason) = report_is_unfinished(&content) {
                                        return block(
                                            state,
                                            PipelinePhase::Build,
                                            state::BlockedKind::Restart,
                                            5,
                                            format!("build report unfinished: {reason}"),
                                        );
                                    }
                                    if report_has_outcome_no_changes(&content) {
                                        state.phase = PipelinePhase::Complete;
                                        return StepDecision::Complete(PipelineOutput {
                                            status: "clean".to_string(),
                                            rounds: 0,
                                            review_changed: false,
                                            commit: state.head_sha.clone(),
                                        });
                                    }
                                    if let Some(commit) = extract_commit_from_report(&content) {
                                        state.head_sha = Some(commit);
                                    }
                                    if state.input.review_strategy == ReviewStrategy::Skip
                                        || state.review_skip
                                    {
                                        state.phase = PipelinePhase::Complete;
                                        return StepDecision::Complete(PipelineOutput {
                                            status: "clean".to_string(),
                                            rounds: 0,
                                            review_changed: false,
                                            commit: state.head_sha.clone(),
                                        });
                                    }
                                    state.phase = PipelinePhase::Review;
                                    state.round = 1;
                                    let (child_id, input_val) = self.build_review_child(state);
                                    state.active_child_id = Some(child_id.clone());
                                    StepDecision::StartChild {
                                        id: child_id,
                                        workflow_type: "single_worker".to_string(),
                                        input: input_val,
                                    }
                                }
                                PipelinePhase::Review => match parse_report_content(&content) {
                                    Ok((findings, is_challenge)) => {
                                        self.handle_review_verdict(state, findings, is_challenge)
                                    }
                                    Err(reason) => block(
                                        state,
                                        PipelinePhase::Review,
                                        state::BlockedKind::Restart,
                                        6,
                                        reason,
                                    ),
                                },
                                PipelinePhase::Fix => {
                                    if content
                                        .lines()
                                        .any(|l| l.trim().starts_with("## Challenge"))
                                    {
                                        state.phase = PipelinePhase::Disputed;
                                        return StepDecision::Complete(PipelineOutput {
                                            status: "disputed".to_string(),
                                            rounds: state.rounds_run,
                                            review_changed: state.review_changed,
                                            commit: state.head_sha.clone(),
                                        });
                                    }
                                    if let Some(reason) = report_is_unfinished(&content) {
                                        return block(
                                            state,
                                            PipelinePhase::Fix,
                                            state::BlockedKind::Restart,
                                            5,
                                            format!("fix report unfinished: {reason}"),
                                        );
                                    }
                                    if let Some(commit) = extract_commit_from_report(&content) {
                                        state.head_sha = Some(commit);
                                    }
                                    state.round += 1;
                                    state.phase = PipelinePhase::Review;
                                    let (child_id, input_val) = self.build_review_child(state);
                                    state.active_child_id = Some(child_id.clone());
                                    StepDecision::StartChild {
                                        id: child_id,
                                        workflow_type: "single_worker".to_string(),
                                        input: input_val,
                                    }
                                }
                                _ => StepDecision::Continue,
                            }
                        }
                        other => {
                            let reason = format!(
                                "activity reading {pending_path} failed or returned unexpected result: {other:?}"
                            );
                            block(
                                state,
                                state.phase,
                                state::BlockedKind::Restart,
                                3,
                                reason,
                            )
                        }
                    }
                } else {
                    StepDecision::Continue
                }
            }
            Some(WorkflowEvent::ChildFailed { id, code, reason }) => {
                if let Some(active_id) = &state.active_child_id {
                    if id != *active_id {
                        return StepDecision::Continue;
                    }
                }
                state.active_child_id = None;
                block(state, state.phase, state::BlockedKind::Restart, code, reason)
            }
            Some(WorkflowEvent::TimerFired { id: _ }) => StepDecision::Continue,
        }
    }
}

fn report_is_unfinished(content: &str) -> Option<&'static str> {
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("Outcome:")
            .or_else(|| t.strip_prefix("outcome:"))
        {
            let upper = rest.trim().to_ascii_uppercase();
            if upper.starts_with("BLOCKED")
                || upper.starts_with("PARTIAL")
                || upper.starts_with("INCOMPLETE")
                || upper.starts_with("FAILED")
            {
                return Some("phase report declared itself unfinished");
            }
        }
    }
    if has_real_blockers(content) {
        return Some("phase report reports real blockers");
    }
    None
}

fn report_has_outcome_no_changes(content: &str) -> bool {
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("Outcome:")
            .or_else(|| t.strip_prefix("outcome:"))
        {
            let lower = rest.trim().to_ascii_lowercase();
            if lower.starts_with("no changes") {
                return true;
            }
        }
    }
    false
}

fn parse_verdict_str(verdict_line: &str) -> Result<(u32, bool), String> {
    if verdict_line.contains("## Challenge") {
        return Ok((0, true));
    }
    let trimmed = verdict_line.trim();
    let rest = if let Some(stripped) = trimmed
        .strip_prefix("Verdict:")
        .or_else(|| trimmed.strip_prefix("verdict:"))
    {
        stripped.trim()
    } else {
        trimmed
    };

    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower == "clean"
        || rest_lower.starts_with("clean ")
        || rest_lower.starts_with("clean.")
        || rest_lower.starts_with("clean,")
        || rest_lower.starts_with("clean:")
        || rest_lower.starts_with("clean -")
    {
        return Ok((0, false));
    }

    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        if let Ok(n) = parts[0].parse::<u32>() {
            let noun = parts[1].trim_matches(|c: char| {
                c == ':' || c == ',' || c == '.' || c == '(' || c == ')' || c == '[' || c == ']'
            });
            if noun.eq_ignore_ascii_case("findings") || noun.eq_ignore_ascii_case("finding") {
                return Ok((n, false));
            }
        }
    }

    Err(format!(
        "unparseable verdict line: '{verdict_line}'; expected 'Verdict: clean' or 'Verdict: <N> findings'"
    ))
}

fn parse_report_content(content: &str) -> Result<(u32, bool), String> {
    if content
        .lines()
        .any(|l| l.trim().starts_with("## Challenge"))
    {
        return Ok((0, true));
    }
    let mut last_verdict_line = None;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("Verdict:") || t.starts_with("verdict:") {
            last_verdict_line = Some(t);
        }
    }
    if let Some(v_line) = last_verdict_line {
        parse_verdict_str(v_line)
    } else {
        Err("review report has no 'Verdict: clean | <N> findings' line".to_string())
    }
}

fn extract_commit_from_report(content: &str) -> Option<String> {
    for line in content.lines() {
        let t = line.trim();
        for prefix in &["commit ", "Commit: ", "New commit: ", "commit: "] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let sha = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches('`');
                if !sha.is_empty() && (sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    return Some(sha.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::activity::ReportFileResult;
    use crate::engine::guard::scan_source_for_determinism_violations;
    use crate::engine::journal::{EventRecord, Journal, SnapshotRecord};
    use crate::engine::testkit::{InMemoryJournal, MockActivities};
    use serde_json::json;

    #[test]
    fn test_plan_gate_pauses_for_approval_and_proceeds() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-plan", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        let mut ctx = WorkflowContext::new(1000);

        // Step 1: Initial step triggers plan child with distinct plan id and builder worker name
        let d1 = wf.step(&mut state, &mut ctx, None);
        match d1 {
            StepDecision::StartChild {
                id,
                workflow_type,
                input,
            } => {
                assert_eq!(id, "sude-task-plan-plan");
                assert_eq!(workflow_type, "single_worker");
                assert_eq!(input["name"], "sude-task-plan");
            }
            other => panic!("expected StartChild for plan, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Plan);

        // Step 2: Plan child completes with SingleWorkerOutput { exit: 0, report_path: "runs/sude-task-plan.md" }
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-plan-plan".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/sude-task-plan.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d2 {
            StepDecision::ScheduleActivity { id, activity } => {
                assert_eq!(id, "read-plan-sude-task-plan-plan");
                assert_eq!(
                    activity,
                    ActivityRequest::ReportFile(ReportFileRequest::Read {
                        path: "runs/sude-task-plan.md".to_string()
                    })
                );
            }
            other => panic!("expected ScheduleActivity for plan report read, got {other:?}"),
        }

        // Deliver report content without challenge -> enters PlanReadyGate with PauseForApproval
        let d3 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-plan-sude-task-plan-plan".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Implementation Plan\n- step 1\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d3 {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.contains("approve"));
                assert!(reason.contains("runs/sude-task-plan.md"));
            }
            other => panic!("expected PauseForApproval, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::PlanReadyGate);
        assert_eq!(state.planfile.as_deref(), Some("runs/sude-task-plan.md"));

        // Step 4: Approval resolved -> advances to build child with distinct build id and same builder worker name
        let d4 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }),
        );
        match d4 {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "sude-task-plan-build");
                assert_eq!(input["name"], "sude-task-plan");
                assert!(input["brief"]
                    .as_str()
                    .unwrap()
                    .contains("runs/sude-task-plan.md"));
            }
            other => panic!("expected StartChild for build, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Build);
    }

    #[test]
    fn test_review_verdict_with_findings_drives_fix_and_fresh_review() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-fix", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        let mut ctx = WorkflowContext::new(2000);

        state.phase = PipelinePhase::Build;
        state.active_child_id = Some("sude-task-fix-build".to_string());

        // 1. Build completes with SingleWorkerOutput { exit: 0, report_path: "runs/sude-task-fix.md" }
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fix-build".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/sude-task-fix.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d1 {
            StepDecision::ScheduleActivity { id, .. } => {
                assert_eq!(id, "read-build-sude-task-fix-build");
            }
            other => panic!("expected ScheduleActivity for build report read, got {other:?}"),
        }

        // Deliver build report content with commit -> launches reviewer child round 1
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-build-sude-task-fix-build".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Build Report\nCommit: abc1234\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d2 {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "suhuynh-task-fix-review-r1");
                assert_eq!(input["name"], "suhuynh-task-fixr1");
                assert!(input["brief"].as_str().unwrap().contains("Verdict: clean"));
            }
            other => panic!("expected StartChild for review r1, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 1);
        assert_eq!(state.head_sha.as_deref(), Some("abc1234"));

        // 2. Reviewer child completes with SingleWorkerOutput -> schedules report read
        let d3 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-fix-review-r1".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "/absolute/runs/suhuynh-task-fixr1.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d3 {
            StepDecision::ScheduleActivity { id, activity } => {
                assert_eq!(id, "read-report-r1");
                assert_eq!(
                    activity,
                    ActivityRequest::ReportFile(ReportFileRequest::Read {
                        path: "/absolute/runs/suhuynh-task-fixr1.md".to_string()
                    })
                );
            }
            other => panic!("expected ScheduleActivity for review report read, got {other:?}"),
        }

        // Deliver review report with 2 findings -> drives fix child
        let d4 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\nVerdict: 2 findings\n- finding 1\n- finding 2\n## Lessons\nnone"
                        .to_string(),
                )),
            }),
        );
        match d4 {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "sude-task-fix-fix-r1");
                assert_eq!(input["name"], "sude-task-fix");
                assert!(input["brief"]
                    .as_str()
                    .unwrap()
                    .contains("/absolute/runs/suhuynh-task-fixr1.md"));
            }
            other => panic!("expected StartChild for fix child, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(state.last_findings, 2);
        assert!(state.review_changed);
        assert_eq!(
            state.last_review_report.as_deref(),
            Some("/absolute/runs/suhuynh-task-fixr1.md")
        );

        // 3. Fix child completes with SingleWorkerOutput -> schedules report read
        let d5 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fix-fix-r1".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/sude-task-fix.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d5 {
            StepDecision::ScheduleActivity { id, .. } => {
                assert_eq!(id, "read-fix-sude-task-fix-fix-r1");
            }
            other => panic!("expected ScheduleActivity for fix report read, got {other:?}"),
        }

        // Deliver fix report content with new commit -> launches fresh reviewer child round 2
        let d6 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-fix-sude-task-fix-fix-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Fix Report\nCommit: def5678\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d6 {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "suhuynh-task-fix-review-r2");
                assert_eq!(input["name"], "suhuynh-task-fixr2");
            }
            other => panic!("expected StartChild for review r2, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 2);
        assert_eq!(state.head_sha.as_deref(), Some("def5678"));

        // 4. Round 2 reviewer child completes with SingleWorkerOutput -> schedules report read
        let d7 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-fix-review-r2".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/suhuynh-task-fixr2.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d7 {
            StepDecision::ScheduleActivity { id, .. } => {
                assert_eq!(id, "read-report-r2");
            }
            other => panic!("expected ScheduleActivity for r2 report read, got {other:?}"),
        }

        // Deliver review r2 report with clean verdict -> complete workflow
        let d8 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r2".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review r2\nVerdict: clean\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d8 {
            StepDecision::Complete(output) => {
                assert_eq!(output.status, "clean");
                assert_eq!(output.rounds, 2);
                assert!(output.review_changed);
                assert_eq!(output.commit.as_deref(), Some("def5678"));
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Complete);
    }

    #[test]
    fn test_mid_pipeline_state_resumes_without_replaying_completed_phases() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-resume", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        let mut ctx = WorkflowContext::new(1000);
        let mut journal = InMemoryJournal::default();

        // 1. Initial step emits plan child (seq 1)
        let d1 = wf.step(&mut state, &mut ctx, None);
        match d1 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-resume-plan");
            }
            other => panic!("expected StartChild for plan, got {other:?}"),
        }
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(1, 1000, "child_started", "id=sude-task-resume-plan"),
            )
            .unwrap();

        // 2. Plan child completes with SingleWorkerOutput -> ScheduleActivity (seq 2)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-resume-plan".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/sude-task-resume.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(2, 1100, "child_completed", "id=sude-task-resume-plan"),
            )
            .unwrap();

        // Plan report read returns content without challenge -> PauseForApproval (seq 3)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-plan-sude-task-resume-plan".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Plan\n- step 1\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(3, 1150, "pause_for_approval", "reason=plan-ready"),
            )
            .unwrap();

        // 3. Operator approves -> start build child (seq 4, 5)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(4, 1200, "approval_resolved", "approved=true"),
            )
            .unwrap();
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(5, 1200, "child_started", "id=sude-task-resume-build"),
            )
            .unwrap();

        // 4. Build child completes with SingleWorkerOutput -> ScheduleActivity (seq 6)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-resume-build".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/sude-task-resume.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(6, 1300, "child_completed", "id=sude-task-resume-build"),
            )
            .unwrap();

        // Build report read returns content with commit -> advances to Review round 1 (seq 7)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-build-sude-task-resume-build".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Build\nCommit: deadbeef\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(7, 1350, "child_started", "id=suhuynh-task-resume-review-r1"),
            )
            .unwrap();

        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 1);
        assert_eq!(state.head_sha.as_deref(), Some("deadbeef"));

        // Snapshot the state at sequence 7
        let snapshot_json = serde_json::to_string(&state).expect("serialize snapshot");
        journal
            .append_snapshot("pipe-task-resume", &SnapshotRecord::new(7, snapshot_json))
            .expect("append snapshot");

        // 5. Append in-flight event occurring after snapshot (seq 8: review child completes)
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(
                    8,
                    1400,
                    "child_completed",
                    serde_json::to_value(
                        SingleWorkerOutput::new(0, "runs/suhuynh-task-resumer1.md").unwrap(),
                    )
                    .unwrap()
                    .to_string(),
                ),
            )
            .unwrap();

        // 6. Simulate crash & recovery: restore snapshot + replay events since snapshot seq
        let loaded_snapshot = journal
            .load_latest_snapshot("pipe-task-resume")
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(loaded_snapshot.seq, 7);

        let mut restored_state: PipelineWorkflowState =
            serde_json::from_str(&loaded_snapshot.state_json).expect("deserialize snapshot");
        assert_eq!(restored_state.phase, PipelinePhase::Review);
        assert_eq!(restored_state.round, 1);

        let subsequent_events = journal
            .load_events("pipe-task-resume", loaded_snapshot.seq + 1)
            .expect("load subsequent events");
        assert_eq!(subsequent_events.len(), 1);

        // Replay event seq 8 onto restored state
        let event8_payload: SingleWorkerOutput =
            serde_json::from_str(&subsequent_events[0].payload_json).unwrap();
        let replayed_event = WorkflowEvent::ChildCompleted {
            id: "suhuynh-task-resume-review-r1".to_string(),
            output: serde_json::to_value(event8_payload).unwrap(),
        };
        let d_after_replay = wf.step(&mut restored_state, &mut ctx, Some(replayed_event));
        match d_after_replay {
            StepDecision::ScheduleActivity { id, .. } => {
                assert_eq!(id, "read-report-r1");
            }
            other => panic!("expected ScheduleActivity for review report, got {other:?}"),
        }

        // Deliver report content with 1 finding
        let decision = wf.step(
            &mut restored_state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\nVerdict: 1 findings\n- fix this\n## Lessons\nnone".to_string(),
                )),
            }),
        );

        // Invariant: resumes at Fix child for round 1, without replaying plan or build child
        match decision {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "sude-task-resume-fix-r1");
                assert_eq!(input["name"], "sude-task-resume");
            }
            other => panic!("expected StartChild for fix r1, got {other:?}"),
        }
        assert_eq!(restored_state.phase, PipelinePhase::Fix);
        assert_eq!(restored_state.last_findings, 1);
    }

    #[test]
    fn test_pipeline_workflow_passes_determinism_guard() {
        let mod_source = include_str!("mod.rs");
        let violations = scan_source_for_determinism_violations(mod_source, "pipeline/mod.rs");
        assert!(
            violations.is_empty(),
            "found determinism violations in PipelineWorkflow: {violations:?}"
        );
    }

    #[test]
    fn test_pipeline_workflow_run_to_completion_clean_path() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-e2e", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        let mut ctx = WorkflowContext::new(0);
        let mut activities = MockActivities::new();

        // Register mock reports for each child (worker names: sude-task-e2e, suhuynh-task-e2er1)
        activities.register(
            "read-plan-sude-task-e2e-plan",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Plan\n- step 1\n## Lessons\nnone".to_string(),
            )),
        );
        activities.register(
            "read-build-sude-task-e2e-build",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Build\nCommit: c0ffee2\n## Lessons\nnone".to_string(),
            )),
        );
        activities.register(
            "read-report-r1",
            ActivityResult::ReportFile(ReportFileResult::Content(
                "# Review\nVerdict: clean\n## Lessons\nnone".to_string(),
            )),
        );

        let mut event: Option<WorkflowEvent> = None;
        let mut output = None;

        for _ in 0..100 {
            let decision = wf.step(&mut state, &mut ctx, event.take());
            match decision {
                StepDecision::StartChild { id, .. } => {
                    let report_path = if id.contains("review") {
                        "runs/suhuynh-task-e2er1.md"
                    } else {
                        "runs/sude-task-e2e.md"
                    };
                    event = Some(WorkflowEvent::ChildCompleted {
                        id,
                        output: serde_json::to_value(
                            SingleWorkerOutput::new(0, report_path).unwrap(),
                        )
                        .unwrap(),
                    });
                }
                StepDecision::ScheduleActivity { id, activity } => {
                    let result = activities.execute(&id, &activity).unwrap();
                    event = Some(WorkflowEvent::ActivityCompleted { id, result });
                }
                StepDecision::PauseForApproval { .. } => {
                    event = Some(WorkflowEvent::ApprovalResolved {
                        approved: true,
                        reason: None,
                    });
                }
                StepDecision::Complete(out) => {
                    output = Some(out);
                    break;
                }
                StepDecision::Fail { code, reason } => {
                    panic!("workflow failed unexpectedly: code={code}, reason={reason}");
                }
                StepDecision::Continue => {}
                StepDecision::ScheduleTimer { .. } => {}
            }
        }

        let out = output.expect("workflow should complete clean");
        assert_eq!(out.status, "clean");
        assert_eq!(out.rounds, 1);
        assert_eq!(out.commit.as_deref(), Some("c0ffee2"));
    }

    #[test]
    fn test_review_rounds_exhausted_fails_with_exit_5_blocked() {
        let wf = PipelineWorkflow;
        let mut input = PipelineInput::new("task-exhaust", "/repo", "sude", "suhuynh", "brief");
        input.max_rounds = 1;
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;
        state.pending_report_read = Some("runs/suhuynh-task-exhaustr1.md".to_string());

        let mut ctx = WorkflowContext::new(1000);

        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\nVerdict: 3 findings\n- f1\n- f2\n- f3\n## Lessons\nnone".to_string(),
                )),
            }),
        );

        match decision {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 5:"));
                assert!(reason.contains("exhausted"));
                assert!(reason.contains("3 findings remain"));
            }
            other => panic!("expected PauseForApproval (blocked 5), got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Blocked);
        assert_eq!(state.blocked_kind, Some(state::BlockedKind::RoundsExhausted));
        assert_eq!(state.blocked_code, 5);
        assert_eq!(state.blocked_from, Some(PipelinePhase::Review));
    }

    #[test]
    fn test_review_with_no_verdict_fails_closed() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-noverdict", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;

        let mut ctx = WorkflowContext::new(1000);

        // Case 1: ChildCompleted with invalid contract (missing report_path) fails immediately with code 6
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-noverdict-review-r1".to_string(),
                output: json!({ "exit": 0 }),
            }),
        );
        match d1 {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 6:"));
                assert!(reason.contains("invalid output contract"));
            }
            other => panic!("expected PauseForApproval blocked 6, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Blocked);
        assert_eq!(state.blocked_kind, Some(state::BlockedKind::Restart));
        assert_eq!(state.blocked_code, 6);
        assert_eq!(state.blocked_from, Some(PipelinePhase::Review));

        // Case 2: Report content with no Verdict line fails with code 6
        let mut state2 = PipelineWorkflowState::new(PipelineInput::new(
            "task-noverdict2",
            "/repo",
            "sude",
            "suhuynh",
            "brief",
        ));
        state2.phase = PipelinePhase::Review;
        state2.round = 1;
        state2.pending_report_read = Some("runs/report.md".to_string());

        let d2 = wf.step(
            &mut state2,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review Report without verdict\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d2 {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 6:"));
                assert!(reason.contains("no 'Verdict: clean | <N> findings' line"));
            }
            other => panic!("expected PauseForApproval blocked 6, got {other:?}"),
        }
        assert_eq!(state2.phase, PipelinePhase::Blocked);
        assert_eq!(state2.blocked_kind, Some(state::BlockedKind::Restart));
        assert_eq!(state2.blocked_code, 6);
        assert_eq!(state2.blocked_from, Some(PipelinePhase::Review));
    }

    #[test]
    fn test_review_verdict_with_findings_substring_clean_parses_findings() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-substr", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;
        state.pending_report_read = Some("runs/report.md".to_string());

        let mut ctx = WorkflowContext::new(1000);

        // Line has both numbers and "clean" (e.g. "Verdict: 2 findings (not clean)")
        // Must parse as 2 findings, NOT clean!
        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\nVerdict: 2 findings (not clean)\n- f1\n- f2\n## Lessons\nnone"
                        .to_string(),
                )),
            }),
        );
        match decision {
            StepDecision::StartChild { id, input, .. } => {
                assert_eq!(id, "sude-task-substr-fix-r1");
                assert_eq!(input["name"], "sude-task-substr");
            }
            other => panic!("expected StartChild for fix, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(state.last_findings, 2);
    }

    #[test]
    fn test_failed_report_read_fails_workflow_without_deadlock() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-failread", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;
        state.pending_report_read = Some("runs/missing.md".to_string());

        let mut ctx = WorkflowContext::new(1000);

        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Exists(false)),
            }),
        );
        match decision {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 3:"));
                assert!(reason.contains("runs/missing.md"));
            }
            other => panic!("expected PauseForApproval blocked 3, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Blocked);
        assert_eq!(state.blocked_kind, Some(state::BlockedKind::Restart));
        assert_eq!(state.blocked_code, 3);
        assert_eq!(state.blocked_from, Some(PipelinePhase::Review));
    }

    #[test]
    fn test_challenge_in_build_report_stops_as_disputed() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-chal", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Build;
        state.active_child_id = Some("sude-task-chal-build".to_string());

        let mut ctx = WorkflowContext::new(1000);

        // Build child completed, reports path
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-chal-build".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/build-chal.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        match d1 {
            StepDecision::ScheduleActivity { id, .. } => {
                assert_eq!(id, "read-build-sude-task-chal-build");
            }
            other => panic!("expected ScheduleActivity, got {other:?}"),
        }

        // Report content has ## Challenge
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-build-sude-task-chal-build".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Build\n## Challenge\npremise is wrong\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d2 {
            StepDecision::Complete(output) => {
                assert_eq!(output.status, "disputed");
            }
            other => panic!("expected Complete with disputed, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Disputed);
    }

    #[test]
    fn test_plan_report_with_blockers_fails_as_blocked() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-blk", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Plan;
        state.active_child_id = Some("sude-task-blk-plan".to_string());

        let mut ctx = WorkflowContext::new(1000);

        // Plan child completed
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-blk-plan".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/plan-blk.md").unwrap(),
                )
                .unwrap(),
            }),
        );

        // Deliver report with real blocker
        let d = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-plan-sude-task-blk-plan".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Plan\n## Blockers\nNeed access to credentials\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 5:"));
                assert!(reason.contains("real blockers"));
            }
            other => panic!("expected PauseForApproval blocked 5, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Blocked);
        assert_eq!(state.blocked_kind, Some(state::BlockedKind::Restart));
        assert_eq!(state.blocked_code, 5);
        assert_eq!(state.blocked_from, Some(PipelinePhase::Plan));
    }

    #[test]
    fn test_build_report_with_outcome_no_changes_skips_review() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-nochange", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Build;
        state.active_child_id = Some("sude-task-nochange-build".to_string());

        let mut ctx = WorkflowContext::new(1000);

        // Build child completed
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-nochange-build".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/build-nochange.md").unwrap(),
                )
                .unwrap(),
            }),
        );

        // Deliver report with Outcome: no changes
        let d = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-build-sude-task-nochange-build".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Build\nOutcome: no changes\n## Lessons\nnone".to_string(),
                )),
            }),
        );
        match d {
            StepDecision::Complete(output) => {
                assert_eq!(output.status, "clean");
                assert_eq!(output.rounds, 0); // Review skipped!
            }
            other => panic!("expected Complete clean with 0 rounds, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Complete);
    }

    #[test]
    fn test_mismatched_child_id_ignored() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-mismatch", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Fix;
        state.active_child_id = Some("sude-task-mismatch-fix-r1".to_string());

        let mut ctx = WorkflowContext::new(1000);

        // Stale review completion arrives
        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-mismatch-review-r1".to_string(),
                output: serde_json::to_value(
                    SingleWorkerOutput::new(0, "runs/suhuynh-task-mismatchr1.md").unwrap(),
                )
                .unwrap(),
            }),
        );
        assert_eq!(decision, StepDecision::Continue);
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(
            state.active_child_id.as_deref(),
            Some("sude-task-mismatch-fix-r1")
        );
    }

    #[test]
    fn test_child_failure_propagates_exit_code() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-fail", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Build;
        state.active_child_id = Some("sude-task-fail-build".to_string());

        let mut ctx = WorkflowContext::new(5000);

        // Child exits with 4 (time budget exceeded)
        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fail-build".to_string(),
                output: serde_json::to_value(SingleWorkerOutput::new(4, "runs/fail.md").unwrap())
                    .unwrap(),
            }),
        );

        match decision {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.starts_with("blocked 4:"));
                assert!(reason.contains("exit 4"));
            }
            other => panic!("expected PauseForApproval with exit 4, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Blocked);
        assert_eq!(state.blocked_kind, Some(state::BlockedKind::Restart));
        assert_eq!(state.blocked_code, 4);
        assert_eq!(state.blocked_from, Some(PipelinePhase::Build));
    }

    #[test]
    fn test_blocked_gate_rejected_fails_terminally() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-reject", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Blocked;
        state.blocked_kind = Some(state::BlockedKind::Restart);
        state.blocked_from = Some(PipelinePhase::Build);
        state.blocked_code = 5;

        let mut ctx = WorkflowContext::new(1000);

        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: false,
                reason: Some("operator aborted task".to_string()),
            }),
        );

        match decision {
            StepDecision::Fail { code, reason } => {
                assert_eq!(code, 1);
                assert_eq!(reason, "operator aborted task");
            }
            other => panic!("expected Fail code 1, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Failed);
    }

    #[test]
    fn test_blocked_gate_approved_restart_and_rounds_exhausted() {
        let wf = PipelineWorkflow;
        let mut input = PipelineInput::new("task-resume-test", "/repo", "sude", "suhuynh", "brief");
        input.max_rounds = 1;
        let mut state = PipelineWorkflowState::new(input);

        // Subcase A: Restart on Build
        state.phase = PipelinePhase::Blocked;
        state.blocked_kind = Some(state::BlockedKind::Restart);
        state.blocked_from = Some(PipelinePhase::Build);
        state.blocked_code = 4;

        let mut ctx = WorkflowContext::new(1000);
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }),
        );
        match d1 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-resume-test-build");
            }
            other => panic!("expected StartChild on restart, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Build);

        // Subcase B: RoundsExhausted with directive max_rounds: 3
        state.phase = PipelinePhase::Blocked;
        state.round = 1;
        state.blocked_kind = Some(state::BlockedKind::RoundsExhausted);
        state.blocked_from = Some(PipelinePhase::Review);
        state.blocked_code = 5;

        let directive_json = serde_json::to_string(&state::ResumeDirective { max_rounds: 3 }).unwrap();
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: Some(directive_json),
            }),
        );
        match d2 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-resume-test-fix-r1");
            }
            other => panic!("expected StartChild for fix after directive, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(state.input.max_rounds, 3);
    }
    static PIPELINE_DRIVER_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TestEnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl TestEnvVarGuard {
        fn set(key: &'static str, val: impl AsRef<std::ffi::OsStr>) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, prev }
        }
    }

    impl Drop for TestEnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn setup_test_pipeline_workspace(test_name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let pid = std::process::id();
        let base_dir = std::env::temp_dir().join(format!("quaqua-test-{test_name}-{pid}"));
        let _ = std::fs::remove_dir_all(&base_dir);
        let _ = std::fs::create_dir_all(&base_dir);

        let repo_dir = base_dir.join("repo");
        let _ = std::fs::create_dir_all(&repo_dir);

        let runs_dir = base_dir.join("runs");
        let _ = std::fs::create_dir_all(&runs_dir);

        let _ = std::process::Command::new("git")
            .args(&["init", "-b", "main"])
            .current_dir(&repo_dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(&["config", "user.name", "Tester"])
            .current_dir(&repo_dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(&["config", "user.email", "tester@example.com"])
            .current_dir(&repo_dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(&["commit", "--allow-empty", "-m", "initial commit"])
            .current_dir(&repo_dir)
            .output();

        let brief_file = repo_dir.join("brief.md");
        std::fs::write(
            &brief_file,
            "Goal: test feature\nHard constraints: none\nCurrent design: none\n",
        )
        .unwrap();

        let team_file = base_dir.join("team.toml");
        std::fs::write(&team_file, include_str!("../../../../team.toml")).unwrap();

        let stub_bin = base_dir.join("herdr-stub");
        let script = r#"#!/bin/sh
case "$1" in
  pane)
    case "$2" in
      split) echo '{"result": {"pane": {"pane_id": "test-pane"}}}'; exit 0 ;;
      report-metadata|send-keys|wait-output|close) exit 0 ;;
      *) exit 0 ;;
    esac ;;
  agent)
    case "$2" in
      prompt)
        NAME="$3"
        PROMPT="$4"
        case "$NAME" in
          sude-*)
            case "$PROMPT" in
              *PLAN*)
                cat <<'REPEOF' > "$QUAQUA_RUNS_DIR/$NAME.md"
# Plan
Implementation steps.
## Lessons
none
REPEOF
                ;;
              *BUILD*)
                WT="$(dirname "$QUAQUA_RUNS_DIR")/.wt/$NAME"
                if [ -d "$WT" ]; then
                  (cd "$WT" && git commit --allow-empty -m "build commit" 2>/dev/null)
                fi
                cat <<'REPEOF' > "$QUAQUA_RUNS_DIR/$NAME.md"
# Build
Implemented plan.
## Lessons
none
REPEOF
                ;;
              *FIX*)
                WT="$(dirname "$QUAQUA_RUNS_DIR")/.wt/$NAME"
                if [ -d "$WT" ]; then
                  (cd "$WT" && git commit --allow-empty -m "fix commit" 2>/dev/null)
                fi
                cat <<'REPEOF' > "$QUAQUA_RUNS_DIR/$NAME.md"
# Fix
Fixed defect.
## Lessons
none
REPEOF
                ;;
            esac
            exit 0
            ;;
          suhuynh-*r1)
            cat <<'REPEOF' > "$QUAQUA_RUNS_DIR/$NAME.md"
# Review
## Findings
1. An issue
Verdict: 1 finding
## Lessons
none
REPEOF
            exit 0
            ;;
          suhuynh-*r2)
            cat <<'REPEOF' > "$QUAQUA_RUNS_DIR/$NAME.md"
# Review
Verdict: clean
## Lessons
none
REPEOF
            exit 0
            ;;
        esac
        exit 0
        ;;
      start) exit 0 ;;
      get) echo '{"result": {"agent": {"agent_status": "idle"}}}'; exit 0 ;;
      list) echo '{"result": {"agents": []}}'; exit 0 ;;
      wait-idle) exit 0 ;;
      *) exit 0 ;;
    esac ;;
  *) exit 0 ;;
esac
"#;
        std::fs::write(&stub_bin, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub_bin).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub_bin, perms).unwrap();

        (base_dir, repo_dir, runs_dir, stub_bin, team_file)
    }

    #[test]
    fn test_verdict_parser_strict() {
        assert_eq!(parse_verdict_str("Verdict: clean"), Ok((0, false)));
        assert_eq!(parse_verdict_str("Verdict: clean."), Ok((0, false)));
        assert_eq!(parse_verdict_str("verdict: clean"), Ok((0, false)));
        assert_eq!(parse_verdict_str("Verdict: 0 findings"), Ok((0, false)));
        assert_eq!(parse_verdict_str("Verdict: 1 finding"), Ok((1, false)));
        assert_eq!(parse_verdict_str("Verdict: 2 findings"), Ok((2, false)));
        assert_eq!(parse_verdict_str("Verdict: 3 findings."), Ok((3, false)));
        assert_eq!(parse_verdict_str("Verdict: 2 findings (see details)"), Ok((2, false)));

        // Strict parser failures (findings #2)
        assert!(parse_verdict_str("Verdict: not clean").is_err());
        assert!(parse_verdict_str("Verdict: 0 of 3 findings addressed").is_err());
        assert!(parse_verdict_str("Verdict: findings below").is_err());
        assert!(parse_verdict_str("Verdict: see above").is_err());
        assert!(parse_verdict_str("Verdict: unknown").is_err());
        assert!(parse_verdict_str("Verdict: ").is_err());
    }

    #[test]
    #[ignore = "driver: T2"]
    fn test_finding2_driver_unparseable_verdict_fails_closed() {
        let _lock = PIPELINE_DRIVER_TEST_MUTEX.lock().unwrap();
        let (base_dir, repo_dir, runs_dir, stub_bin, team_file) = setup_test_pipeline_workspace("f2-bad");

        let _guard_team = TestEnvVarGuard::set("QUAQUA_TEAM_TOML", team_file.to_str().unwrap());
        let _guard_herdr = TestEnvVarGuard::set("HERDR_BIN_PATH", stub_bin.to_str().unwrap());
        let _guard_skip = TestEnvVarGuard::set("GRID_SKIP", "1");
        let _guard_test = TestEnvVarGuard::set("QUAQUA_TEST", "1");
        let _guard_runs = TestEnvVarGuard::set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());
        let _guard_owner = TestEnvVarGuard::set("QUAQUA_OWNER", "cos");
        let git = Git::new();
        let herdr = Herdr { bin: stub_bin.to_string_lossy().to_string() };
        let brief_file = repo_dir.join("brief.md");

        let task = "f2-bad";
        // 1. Initial invocation (Plan)
        std::fs::write(
            runs_dir.join(format!("sude-{task}.md")),
            "# Plan\nImplementation steps.\n## Lessons\nnone\n",
        ).unwrap();

        let args = vec![
            task.to_string(),
            repo_dir.to_string_lossy().to_string(),
            "--brief".to_string(),
            brief_file.to_string_lossy().to_string(),
            "--review".to_string(),
            "always".to_string(),
        ];
        let plan_res = handle_pipeline(&args, &git, &herdr);
        assert_eq!(plan_res, Err((5, String::new())), "plan pause must return exit 5");

        // 2. Setup build commit in worktree and unparseable review report
        let wt_dir = base_dir.join(".wt").join(format!("sude-{task}"));
        let _ = std::process::Command::new("git")
            .args(&["commit", "--allow-empty", "-m", "build commit"])
            .current_dir(&wt_dir)
            .output();

        std::fs::write(
            runs_dir.join(format!("sude-{task}.md")),
            "# Build\nImplemented.\n## Lessons\nnone\n",
        ).unwrap();

        std::fs::write(
            runs_dir.join(format!("suhuynh-{task}r1.md")),
            "# Review\nVerdict: not clean\n## Lessons\nnone\n",
        ).unwrap();

        // 3. Resume invocation
        let resume_args = vec![
            "--resume".to_string(),
            task.to_string(),
            repo_dir.to_string_lossy().to_string(),
        ];
        let resume_res = handle_pipeline(&resume_args, &git, &herdr);
        // Must end blocked with non-zero exit code 6 (malformed report / unparseable verdict), NEVER clean exit 0!
        assert_eq!(resume_res, Err((6, String::new())), "unparseable verdict line must fail closed with exit 6");

        let pfile_content = std::fs::read_to_string(runs_dir.join(format!("pipeline-{task}.md"))).unwrap();
        assert!(pfile_content.contains("- status: blocked"), "pfile must record blocked status");
        assert!(!pfile_content.contains("- status: clean"), "pfile must not record clean status");

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn test_finding3_review_prompt_carries_gated_base_sha_and_never_empty() {
        let wf = PipelineWorkflow;

        // Case A: base_sha is set
        let mut input_with_sha = PipelineInput::new("task-f3a", "/repo", "sude", "suhuynh", "sample brief");
        input_with_sha.base_sha = Some("gated-base-sha-1234".to_string());
        let mut state_a = PipelineWorkflowState::new(input_with_sha);
        state_a.round = 1;

        let (_id_a, val_a) = wf.build_review_child(&state_a);
        let prompt_a = val_a["brief"].as_str().unwrap();
        assert!(
            prompt_a.contains("diff between base gated-base-sha-1234 and branch quaqua/sude-task-f3a"),
            "review prompt must contain gated base SHA, got: {prompt_a}"
        );
        assert!(!prompt_a.contains("base  and branch"), "review prompt must not contain empty base");

        // Case B: base_sha is None and base is empty string -> falls back to HEAD, never empty string!
        let mut input_empty_base = PipelineInput::new("task-f3b", "/repo", "sude", "suhuynh", "sample brief");
        input_empty_base.base = String::new();
        input_empty_base.base_sha = None;
        let mut state_b = PipelineWorkflowState::new(input_empty_base);
        state_b.round = 1;

        let (_id_b, val_b) = wf.build_review_child(&state_b);
        let prompt_b = val_b["brief"].as_str().unwrap();
        assert!(
            prompt_b.contains("diff between base HEAD and branch quaqua/sude-task-f3b"),
            "review prompt with empty base must fallback to HEAD, got: {prompt_b}"
        );
        assert!(!prompt_b.contains("base  and branch"), "review prompt must not contain empty base");
    }

    #[test]
    fn test_finding4_fix_prompt_carries_real_review_report_path_both_rounds() {
        let wf = PipelineWorkflow;
        let mut input = PipelineInput::new("task-f4", "/repo", "sude", "suhuynh", "sample brief");
        input.max_rounds = 3;
        let mut state = PipelineWorkflowState::new(input);

        // Round 1
        state.phase = PipelinePhase::Review;
        state.round = 1;
        let r1_path = "/path/to/runs/suhuynh-task-f4r1.md";
        let mut ctx = WorkflowContext::new(1000);

        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-f4-review-r1".to_string(),
                output: serde_json::to_value(SingleWorkerOutput::new(0, r1_path).unwrap()).unwrap(),
            }),
        );
        assert!(matches!(d1, StepDecision::ScheduleActivity { .. }));

        let d1_content = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\n## Findings\n1. Issue one\nVerdict: 1 finding\n## Lessons\nnone".to_string(),
                )),
            }),
        );

        match d1_content {
            StepDecision::StartChild { input, .. } => {
                let prompt = input["brief"].as_str().unwrap();
                assert!(
                    prompt.contains(r1_path),
                    "Round 1 fix prompt must contain real review report path '{r1_path}', got: {prompt}"
                );
            }
            other => panic!("expected StartChild for fix r1, got {other:?}"),
        }

        // Round 2
        state.phase = PipelinePhase::Review;
        state.round = 2;
        state.active_child_id = Some("suhuynh-task-f4-review-r2".to_string());
        let r2_path = "/path/to/runs/suhuynh-task-f4r2.md";
        let _d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-f4-review-r2".to_string(),
                output: serde_json::to_value(SingleWorkerOutput::new(0, r2_path).unwrap()).unwrap(),
            }),
        );

        let d2_content = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r2".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    "# Review\n## Findings\n1. Remaining issue\nVerdict: 1 finding\n## Lessons\nnone".to_string(),
                )),
            }),
        );

        match d2_content {
            StepDecision::StartChild { input, .. } => {
                let prompt = input["brief"].as_str().unwrap();
                assert!(
                    prompt.contains(r2_path),
                    "Round 2 fix prompt must contain real review report path '{r2_path}', got: {prompt}"
                );
            }
            other => panic!("expected StartChild for fix r2, got {other:?}"),
        }
    }

    #[test]
    fn test_finding5_handle_pipeline_e2e_integration() {
        let _lock = PIPELINE_DRIVER_TEST_MUTEX.lock().unwrap();
        let (base_dir, repo_dir, runs_dir, stub_bin, team_file) = setup_test_pipeline_workspace("f5-e2e");

        let _guard_team = TestEnvVarGuard::set("QUAQUA_TEAM_TOML", team_file.to_str().unwrap());
        let _guard_herdr = TestEnvVarGuard::set("HERDR_BIN_PATH", stub_bin.to_str().unwrap());
        let _guard_skip = TestEnvVarGuard::set("GRID_SKIP", "1");
        let _guard_test = TestEnvVarGuard::set("QUAQUA_TEST", "1");
        let _guard_runs = TestEnvVarGuard::set("QUAQUA_RUNS_DIR", runs_dir.to_str().unwrap());
        let _guard_owner = TestEnvVarGuard::set("QUAQUA_OWNER", "cos");
        let _guard_poll = TestEnvVarGuard::set("PROMPT_WORKER_POLL_SEC", "1");
        let _guard_stall = TestEnvVarGuard::set("PROMPT_WORKER_STALL_SEC", "10");
        let _guard_idle = TestEnvVarGuard::set("PROMPT_WORKER_IDLE_SEC", "10");

        let git = Git::new();
        let herdr = Herdr { bin: stub_bin.to_string_lossy().to_string() };
        let brief_file = repo_dir.join("brief.md");

        let task = "f5-e2e";
        let args = vec![
            task.to_string(),
            repo_dir.to_string_lossy().to_string(),
            "--brief".to_string(),
            brief_file.to_string_lossy().to_string(),
            "--review".to_string(),
            "always".to_string(),
            "--rounds".to_string(),
            "2".to_string(),
        ];

        // 1. Initial invocation runs Plan, pauses with exit 5 (plan-ready)
        let plan_res = handle_pipeline(&args, &git, &herdr);
        assert_eq!(plan_res, Err((5, String::new())), "plan pause must return exit 5 (plan-ready)");

        // 2. Resume invocation drives Build -> Review r1 (1 finding) -> Fix r1 -> Review r2 (clean)
        let resume_args = vec![
            "--resume".to_string(),
            task.to_string(),
            repo_dir.to_string_lossy().to_string(),
        ];

        let final_res = handle_pipeline(&resume_args, &git, &herdr);
        assert_eq!(final_res, Ok(()), "full pipeline run must finish with Ok(()) / clean exit 0");

        let pfile_content = std::fs::read_to_string(runs_dir.join(format!("pipeline-{task}.md"))).unwrap();
        assert!(pfile_content.contains("- status: clean"), "pipeline md must report clean status");
        assert!(pfile_content.contains("- review round 1:"), "pipeline md must log review round 1");
        assert!(pfile_content.contains("- fix 1:"), "pipeline md must log fix 1");
        assert!(pfile_content.contains("- review round 2:"), "pipeline md must log review round 2");

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn test_finding6_resume_with_snapshot_vs_without_matches() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-f6", "/repo", "sude", "suhuynh", "sample brief");
        let initial_state = PipelineWorkflowState::new(input.clone());

        // Journal A: No snapshots
        let mut journal_a = InMemoryJournal::new();
        // Journal B: Snapshot at pause_for_approval
        let mut journal_b = InMemoryJournal::new();

        let mut act_a = PipelineActivities { system: SystemActivities::new() };
        let mut act_b = PipelineActivities { system: SystemActivities::new() };
        let mut clock_a = RealClock::new();
        let mut clock_b = RealClock::new();
        let opts_no_snap = RunnerOptions { snapshot_at_blocked: false, ..Default::default() };
        let opts_with_snap = RunnerOptions { snapshot_at_blocked: true, ..Default::default() };

        // Run to Plan pause in both
        let out_a1 = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_a,
            &mut act_a,
            &mut clock_a,
            &opts_no_snap,
            vec![],
        ).unwrap();

        let out_b1 = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_b,
            &mut act_b,
            &mut clock_b,
            &opts_with_snap,
            vec![],
        ).unwrap();

        assert_eq!(out_a1, out_b1);

        let tmp_runs = std::env::temp_dir().join(format!("quaqua-f6-runs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_runs);
        let plan_file = tmp_runs.join("sude-task-f6.plan.md");
        std::fs::write(&plan_file, "# Plan\nImplementation steps.\n## Lessons\nnone\n").unwrap();
        let plan_out = SingleWorkerOutput::new(0, &plan_file).unwrap();

        let _ = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_a,
            &mut act_a,
            &mut clock_a,
            &opts_no_snap,
            vec![
                WorkflowEvent::ChildCompleted {
                    id: "sude-task-f6-plan".to_string(),
                    output: serde_json::to_value(&plan_out).unwrap(),
                },
            ],
        ).unwrap();

        let _ = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_b,
            &mut act_b,
            &mut clock_b,
            &opts_with_snap,
            vec![
                WorkflowEvent::ChildCompleted {
                    id: "sude-task-f6-plan".to_string(),
                    output: serde_json::to_value(&plan_out).unwrap(),
                },
            ],
        ).unwrap();

        // Verify Journal B has a snapshot while Journal A has none
        assert!(journal_a.load_latest_snapshot("pipeline:task-f6").unwrap().is_none());
        assert!(journal_b.load_latest_snapshot("pipeline:task-f6").unwrap().is_some());

        // Resume both with ApprovalResolved: both must yield identical Build state and child decision!
        let resume_event = vec![WorkflowEvent::ApprovalResolved { approved: true, reason: None }];
        let res_a = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_a,
            &mut act_a,
            &mut clock_a,
            &opts_no_snap,
            resume_event.clone(),
        ).unwrap();

        let res_b = runner::run_until_blocked(
            &wf,
            "pipeline:task-f6",
            Some(initial_state.clone()),
            &mut journal_b,
            &mut act_b,
            &mut clock_b,
            &opts_with_snap,
            resume_event,
        ).unwrap();

        assert_eq!(res_a, res_b);
        match res_a {
            RunnerOutcome::Blocked(reason) => assert!(reason.contains("no pending events")),
            other => panic!("expected Blocked for build child start, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&tmp_runs);
    }
}
