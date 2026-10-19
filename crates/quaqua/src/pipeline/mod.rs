//! Pipeline engine (`quaqua pipeline`).
//! Implements build -> review -> fix orchestration state machine.

pub mod phases;
pub mod state;

use crate::engine::activity::{
    ActivityRequest, ActivityResult, ReportFileRequest, ReportFileResult,
};
use crate::engine::child::SingleWorkerInput;
use crate::engine::workflow::{StepDecision, Workflow, WorkflowContext, WorkflowEvent};
pub use phases::{
    blocked_outcome, harness_of_kind, has_real_blockers, pipeline_finished,
    select_default_reviewer, PipelineContext, StepResult,
};
pub use state::{
    PipelineInput, PipelineOutput, PipelinePhase, PipelineState, PipelineWorkflowState,
    ReviewStrategy,
};

use crate::cap::count_owner_slots;
use crate::config::load_team_config;
use crate::git::Git;
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

    if !args.is_empty() && args[0] == "--resume" {
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

    // Phase machine loop
    loop {
        if let Some(res) = ctx.check_clock(&mut state) {
            return res.map(|_| ());
        }

        let step = match state.phase.as_str() {
            "spawn" => phases::spawn::run_spawn(&mut state, &ctx)?,
            "plan" => phases::plan::run_plan(&mut state, &ctx)?,
            "build" => phases::build::run_build(&mut state, &ctx)?,
            "review" => phases::review::run_review(&mut state, &ctx)?,
            "fix" => phases::fix::run_fix(&mut state, &ctx)?,
            _ => {
                return Err((
                    1,
                    format!(
                        "corrupt state: phase={} ({})\n",
                        state.phase,
                        ctx.state_file.display()
                    ),
                ));
            }
        };

        if let StepResult::Done = step {
            return Ok(());
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
            state.phase = PipelinePhase::Complete;
            return StepDecision::Complete(PipelineOutput {
                status: "findings".to_string(),
                rounds: state.rounds_run,
                review_changed: true,
                commit: state.head_sha.clone(),
            });
        }

        state.phase = PipelinePhase::Fix;
        let bname = format!("{}-{}", state.input.builder, state.input.task);
        let revname = format!(
            "{}-{}r{}",
            state.input.reviewer, state.input.task, state.round
        );
        let fix_prompt = format!(
            "Goal: resolve the {} review findings for task '{}' (reviewer {revname})\nPipeline phase: FIX (round {})",
            state.last_findings, state.input.task, state.round
        );
        let worker_input = SingleWorkerInput::new(
            &bname,
            &state.input.builder,
            &state.input.repo,
            &fix_prompt,
            state.input.fix_budget_sec,
            None,
        );
        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        let child_id = format!("{}-fix{}", bname, state.round);
        state.active_child_id = Some(child_id.clone());
        StepDecision::StartChild {
            id: child_id,
            workflow_type: "single_worker".to_string(),
            input: input_val,
        }
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
                    let child_id = format!("{}-{}", state.input.builder, state.input.task);
                    let plan_prompt = format!(
                        "{}\n\nPipeline phase: PLAN — write only the implementation plan (steps, files to touch, risks) as your runs file; do not change code yet. The plan is approved before you build. If the premise is wrong, write '## Challenge' with evidence instead of planning around it.",
                        state.input.brief
                    );
                    let worker_input = SingleWorkerInput::new(
                        &child_id,
                        &state.input.builder,
                        &state.input.repo,
                        &plan_prompt,
                        state.input.plan_budget_sec,
                        None,
                    );
                    let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
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
                    let child_id = format!("{}-{}", state.input.builder, state.input.task);
                    let build_prompt = format!(
                        "{}\n\nPipeline phase: BUILD — implement the plan, run tests, commit.",
                        state.input.brief
                    );
                    let worker_input = SingleWorkerInput::new(
                        &child_id,
                        &state.input.builder,
                        &state.input.repo,
                        &build_prompt,
                        state.input.build_budget_sec,
                        None,
                    );
                    let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
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
                    let child_id = format!(
                        "{}-{}r{}",
                        state.input.reviewer, state.input.task, state.round
                    );
                    let review_prompt = format!(
                        "Goal: review changes for task '{}' (round {})\nPipeline phase: REVIEW",
                        state.input.task, state.round
                    );
                    let worker_input = SingleWorkerInput::new(
                        &child_id,
                        &state.input.reviewer,
                        &state.input.repo,
                        &review_prompt,
                        state.input.review_budget_sec,
                        None,
                    );
                    let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
                    state.active_child_id = Some(child_id.clone());
                    StepDecision::StartChild {
                        id: child_id,
                        workflow_type: "single_worker".to_string(),
                        input: input_val,
                    }
                }
                PipelinePhase::Fix => {
                    let bname = format!("{}-{}", state.input.builder, state.input.task);
                    let child_id = format!("{}-fix{}", bname, state.round);
                    let fix_prompt = format!(
                        "Goal: resolve the {} review findings for task '{}'\nPipeline phase: FIX (round {})",
                        state.last_findings, state.input.task, state.round
                    );
                    let worker_input = SingleWorkerInput::new(
                        &bname,
                        &state.input.builder,
                        &state.input.repo,
                        &fix_prompt,
                        state.input.fix_budget_sec,
                        None,
                    );
                    let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
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
            },
            Some(WorkflowEvent::ApprovalResolved { approved, reason }) => {
                if state.phase == PipelinePhase::PlanReadyGate {
                    if approved {
                        state.phase = PipelinePhase::Build;
                        let child_id = format!("{}-{}", state.input.builder, state.input.task);
                        let build_prompt = format!(
                            "{}\n\nPipeline phase: BUILD — implement the plan, run tests, commit.",
                            state.input.brief
                        );
                        let worker_input = SingleWorkerInput::new(
                            &child_id,
                            &state.input.builder,
                            &state.input.repo,
                            &build_prompt,
                            state.input.build_budget_sec,
                            None,
                        );
                        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
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
                } else {
                    StepDecision::Continue
                }
            }
            Some(WorkflowEvent::ChildCompleted { id, output }) => {
                let exit_code = output
                    .get("exit")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32)
                    .unwrap_or(0);

                if exit_code != 0 {
                    state.phase = PipelinePhase::Failed;
                    return StepDecision::Fail {
                        code: exit_code,
                        reason: format!("child worker {id} failed with exit {exit_code}"),
                    };
                }

                match state.phase {
                    PipelinePhase::Plan => {
                        state.planfile = Some(format!("{id}.plan.md"));
                        state.active_child_id = None;

                        let challenge = output
                            .get("challenge")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if challenge {
                            state.phase = PipelinePhase::Disputed;
                            return StepDecision::Complete(PipelineOutput {
                                status: "disputed".to_string(),
                                rounds: 0,
                                review_changed: false,
                                commit: None,
                            });
                        }

                        state.phase = PipelinePhase::PlanReadyGate;
                        let reason = format!(
                            "approve {} then resume",
                            state.planfile.as_deref().unwrap_or("plan")
                        );
                        StepDecision::PauseForApproval { reason }
                    }
                    PipelinePhase::Build => {
                        state.active_child_id = None;
                        if let Some(c) = output.get("commit").and_then(|v| v.as_str()) {
                            state.head_sha = Some(c.to_string());
                        }

                        if state.input.review_strategy == ReviewStrategy::Skip {
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
                        let child_id = format!("{}-{}r1", state.input.reviewer, state.input.task);
                        let review_prompt = format!(
                            "Goal: review changes for task '{}' (round 1)\nPipeline phase: REVIEW",
                            state.input.task
                        );
                        let worker_input = SingleWorkerInput::new(
                            &child_id,
                            &state.input.reviewer,
                            &state.input.repo,
                            &review_prompt,
                            state.input.review_budget_sec,
                            None,
                        );
                        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
                        state.active_child_id = Some(child_id.clone());
                        StepDecision::StartChild {
                            id: child_id,
                            workflow_type: "single_worker".to_string(),
                            input: input_val,
                        }
                    }
                    PipelinePhase::Review => {
                        state.rounds_run = state.round;
                        state.active_child_id = None;

                        if let Some(findings) = output.get("findings").and_then(|v| v.as_u64()) {
                            let challenge = output
                                .get("challenge")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            return self.handle_review_verdict(state, findings as u32, challenge);
                        }

                        if let Some(verdict) = output.get("verdict").and_then(|v| v.as_str()) {
                            let (findings, challenge) = parse_verdict_str(verdict);
                            return self.handle_review_verdict(state, findings, challenge);
                        }

                        if let Some(report_path) =
                            output.get("report_path").and_then(|v| v.as_str())
                        {
                            state.pending_report_read = Some(report_path.to_string());
                            return StepDecision::ScheduleActivity {
                                id: format!("read-report-r{}", state.round),
                                activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                    path: report_path.to_string(),
                                }),
                            };
                        }

                        let challenge = output
                            .get("challenge")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        self.handle_review_verdict(state, 0, challenge)
                    }
                    PipelinePhase::Fix => {
                        state.active_child_id = None;
                        state.round += 1;
                        state.phase = PipelinePhase::Review;

                        let child_id = format!(
                            "{}-{}r{}",
                            state.input.reviewer, state.input.task, state.round
                        );
                        let review_prompt = format!(
                            "Goal: review changes for task '{}' (round {})\nPipeline phase: REVIEW",
                            state.input.task, state.round
                        );
                        let worker_input = SingleWorkerInput::new(
                            &child_id,
                            &state.input.reviewer,
                            &state.input.repo,
                            &review_prompt,
                            state.input.review_budget_sec,
                            None,
                        );
                        let input_val = serde_json::to_value(&worker_input).unwrap_or_default();
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
            Some(WorkflowEvent::ActivityCompleted { id: _, result }) => {
                if state.phase == PipelinePhase::Review && state.pending_report_read.is_some() {
                    state.pending_report_read = None;
                    if let ActivityResult::ReportFile(ReportFileResult::Content(content)) = result {
                        let (findings, is_challenge) = parse_report_content(&content);
                        return self.handle_review_verdict(state, findings, is_challenge);
                    }
                }
                StepDecision::Continue
            }
            Some(WorkflowEvent::ChildFailed {
                id: _,
                code,
                reason,
            }) => {
                state.phase = PipelinePhase::Failed;
                StepDecision::Fail { code, reason }
            }
            Some(WorkflowEvent::TimerFired { id: _ }) => StepDecision::Continue,
        }
    }
}

fn parse_verdict_str(verdict: &str) -> (u32, bool) {
    if verdict.contains("## Challenge") {
        return (0, true);
    }
    let v_lower = verdict.to_ascii_lowercase();
    if v_lower.contains("clean") {
        return (0, false);
    }
    for part in verdict.split_whitespace() {
        if let Ok(n) = part.parse::<u32>() {
            return (n, false);
        }
    }
    (0, false)
}

fn parse_report_content(content: &str) -> (u32, bool) {
    if content
        .lines()
        .any(|l| l.trim().starts_with("## Challenge"))
    {
        return (0, true);
    }
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("Verdict:") || t.starts_with("verdict:") {
            return parse_verdict_str(t);
        }
    }
    (0, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::activity::ReportFileResult;
    use crate::engine::guard::scan_source_for_determinism_violations;
    use crate::engine::journal::{Journal, SnapshotRecord};
    use crate::engine::testkit::{
        run_to_completion, InMemoryJournal, MockActivities, VirtualClock,
    };
    use serde_json::json;

    #[test]
    fn test_plan_gate_pauses_for_approval_and_proceeds() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-plan", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        let mut ctx = WorkflowContext::new(1000);

        // Step 1: Initial step triggers plan child
        let d1 = wf.step(&mut state, &mut ctx, None);
        match d1 {
            StepDecision::StartChild {
                id, workflow_type, ..
            } => {
                assert_eq!(id, "sude-task-plan");
                assert_eq!(workflow_type, "single_worker");
            }
            other => panic!("expected StartChild for plan, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Plan);

        // Step 2: Plan child completes with exit 0 -> pause for approval gate
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-plan".to_string(),
                output: json!({ "exit": 0 }),
            }),
        );
        match d2 {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.contains("approve"));
            }
            other => panic!("expected PauseForApproval, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::PlanReadyGate);

        // Step 3: Approval resolved -> advances to build child
        let d3 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ApprovalResolved {
                approved: true,
                reason: None,
            }),
        );
        match d3 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-plan");
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

        // Jump to Build phase
        state.phase = PipelinePhase::Build;

        // Build completes -> launches reviewer child round 1
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fix".to_string(),
                output: json!({ "exit": 0, "commit": "abc1234" }),
            }),
        );
        match d1 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "suhuynh-task-fixr1");
            }
            other => panic!("expected StartChild for review r1, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 1);

        // Reviewer child completes with 2 findings -> drives fix child
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-fixr1".to_string(),
                output: json!({ "exit": 0, "findings": 2 }),
            }),
        );
        match d2 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-fix-fix1");
            }
            other => panic!("expected StartChild for fix child, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(state.last_findings, 2);
        assert!(state.review_changed);

        // Fix child completes -> launches fresh reviewer child round 2
        let d3 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fix-fix1".to_string(),
                output: json!({ "exit": 0 }),
            }),
        );
        match d3 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "suhuynh-task-fixr2");
            }
            other => panic!("expected StartChild for review r2, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 2);

        // Round 2 reviewer child completes clean -> complete workflow
        let d4 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-fixr2".to_string(),
                output: json!({ "exit": 0, "findings": 0 }),
            }),
        );
        match d4 {
            StepDecision::Complete(output) => {
                assert_eq!(output.status, "clean");
                assert_eq!(output.rounds, 2);
                assert!(output.review_changed);
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
        state.phase = PipelinePhase::Review;
        state.round = 1;
        state.planfile = Some("sude-task-resume.plan.md".to_string());
        state.head_sha = Some("deadbeef".to_string());

        // Snapshot to in-memory journal
        let mut journal = InMemoryJournal::default();
        let snapshot_json = serde_json::to_string(&state).expect("serialize snapshot");
        journal
            .append_snapshot("pipe-task-resume", &SnapshotRecord::new(1, snapshot_json))
            .expect("append snapshot");

        // Crash and restore from journal
        let loaded_snapshot = journal
            .load_latest_snapshot("pipe-task-resume")
            .expect("load snapshot")
            .expect("snapshot exists");
        let mut restored_state: PipelineWorkflowState =
            serde_json::from_str(&loaded_snapshot.state_json).expect("deserialize snapshot");

        assert_eq!(restored_state.phase, PipelinePhase::Review);
        assert_eq!(restored_state.round, 1);

        // Step the restored state with no incoming event
        let mut ctx = WorkflowContext::new(3000);
        let decision = wf.step(&mut restored_state, &mut ctx, None);

        // Invariant: resumes at Review round 1 directly without re-emitting plan or build child
        match decision {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "suhuynh-task-resumer1");
            }
            other => panic!("expected StartChild for reviewer r1, got {other:?}"),
        }
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
        let initial_state = PipelineWorkflowState::new(input);
        let mut clock = VirtualClock::new(0);
        let mut activities = MockActivities::new();
        let mut journal = InMemoryJournal::default();

        let result = run_to_completion(
            &wf,
            "pipe-wf-e2e",
            initial_state,
            &mut clock,
            &mut activities,
            &mut journal,
        );

        let output = result.expect("run to completion should succeed");
        assert_eq!(output.status, "clean");
        assert_eq!(output.rounds, 1);
    }

    #[test]
    fn test_review_verdict_via_mock_activity_report() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-act", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;

        let mut ctx = WorkflowContext::new(4000);

        // Child completes with report_path
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-actr1".to_string(),
                output: json!({
                    "exit": 0,
                    "report_path": "runs/suhuynh-task-actr1.md"
                }),
            }),
        );

        match d1 {
            StepDecision::ScheduleActivity { id, activity } => {
                assert_eq!(id, "read-report-r1");
                match activity {
                    ActivityRequest::ReportFile(ReportFileRequest::Read { path }) => {
                        assert_eq!(path, "runs/suhuynh-task-actr1.md");
                    }
                    other => panic!("expected ReportFile Read, got {other:?}"),
                }
            }
            other => panic!("expected ScheduleActivity for report read, got {other:?}"),
        }

        // Deliver ActivityCompleted with 1 finding report
        let report_text = "# Report\nVerdict: 1 findings\n- finding one\n## Lessons\nnone";
        let d2 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ActivityCompleted {
                id: "read-report-r1".to_string(),
                result: ActivityResult::ReportFile(ReportFileResult::Content(
                    report_text.to_string(),
                )),
            }),
        );

        match d2 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-act-fix1");
            }
            other => panic!("expected StartChild for fix, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Fix);
        assert_eq!(state.last_findings, 1);
    }

    #[test]
    fn test_child_failure_propagates_exit_code() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-fail", "/repo", "sude", "suhuynh", "sample brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Build;

        let mut ctx = WorkflowContext::new(5000);

        // Child exits with 4 (time budget exceeded)
        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fail".to_string(),
                output: json!({ "exit": 4 }),
            }),
        );

        match decision {
            StepDecision::Fail { code, reason } => {
                assert_eq!(code, 4);
                assert!(reason.contains("exit 4"));
            }
            other => panic!("expected Fail with exit 4, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Failed);
    }
}
