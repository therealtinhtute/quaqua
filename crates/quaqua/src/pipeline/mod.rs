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
        let mut input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        if let Some(map) = input_val.as_object_mut() {
            map.insert(
                "report_path".to_string(),
                serde_json::Value::String(format!("runs/{child_id}.md")),
            );
        }
        (child_id, input_val)
    }

    fn build_build_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.build_child_id(state);
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
        let mut input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        if let Some(map) = input_val.as_object_mut() {
            map.insert(
                "report_path".to_string(),
                serde_json::Value::String(format!("runs/{child_id}.md")),
            );
        }
        (child_id, input_val)
    }

    fn build_review_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.review_child_id(state);
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
        let mut input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        if let Some(map) = input_val.as_object_mut() {
            map.insert(
                "report_path".to_string(),
                serde_json::Value::String(format!("runs/{child_id}.md")),
            );
        }
        (child_id, input_val)
    }

    fn build_fix_child(&self, state: &PipelineWorkflowState) -> (String, serde_json::Value) {
        let child_id = self.fix_child_id(state);
        let revname = format!(
            "{}-{}r{}",
            state.input.reviewer, state.input.task, state.round
        );
        let fix_prompt = format!(
            "Goal: resolve the {} review findings for task '{}' (reviewer {revname})\nPipeline phase: FIX (round {})",
            state.last_findings, state.input.task, state.round
        );
        let worker_input = SingleWorkerInput::new(
            &child_id,
            &state.input.builder,
            &state.input.repo,
            &fix_prompt,
            state.input.fix_budget_sec,
            None,
        );
        let mut input_val = serde_json::to_value(&worker_input).unwrap_or_default();
        if let Some(map) = input_val.as_object_mut() {
            map.insert(
                "report_path".to_string(),
                serde_json::Value::String(format!("runs/{child_id}.md")),
            );
        }
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
            state.phase = PipelinePhase::Complete;
            return StepDecision::Complete(PipelineOutput {
                status: "findings".to_string(),
                rounds: state.rounds_run,
                review_changed: true,
                commit: state.head_sha.clone(),
            });
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
            },
            Some(WorkflowEvent::ApprovalResolved { approved, reason }) => {
                if state.phase == PipelinePhase::PlanReadyGate {
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
                } else {
                    StepDecision::Continue
                }
            }
            Some(WorkflowEvent::ChildCompleted { id, output }) => {
                // Check if this completion corresponds to the active child id
                if let Some(active_id) = &state.active_child_id {
                    if id != *active_id {
                        return StepDecision::Continue;
                    }
                }
                state.active_child_id = None;

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
                        let report_path = output
                            .get("report_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let Some(rp) = report_path {
                            state.planfile = Some(rp.clone());
                            if let Some(challenge) =
                                output.get("challenge").and_then(|v| v.as_bool())
                            {
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
                                return StepDecision::PauseForApproval {
                                    reason: format!("approve {} then resume", rp),
                                };
                            }
                            state.pending_report_read = Some(rp.clone());
                            return StepDecision::ScheduleActivity {
                                id: format!("read-plan-{id}"),
                                activity: ActivityRequest::ReportFile(ReportFileRequest::Read {
                                    path: rp,
                                }),
                            };
                        }

                        // Fallback when no report_path was provided
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
                        StepDecision::PauseForApproval {
                            reason: format!(
                                "approve {} then resume",
                                state.planfile.as_deref().unwrap_or("plan")
                            ),
                        }
                    }
                    PipelinePhase::Build => {
                        if let Some(c) = output.get("commit").and_then(|v| v.as_str()) {
                            state.head_sha = Some(c.to_string());
                        }

                        let report_path = output
                            .get("report_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let Some(rp) = report_path {
                            if let Some(challenge) =
                                output.get("challenge").and_then(|v| v.as_bool())
                            {
                                if challenge {
                                    state.phase = PipelinePhase::Disputed;
                                    return StepDecision::Complete(PipelineOutput {
                                        status: "disputed".to_string(),
                                        rounds: 0,
                                        review_changed: false,
                                        commit: None,
                                    });
                                }
                            } else {
                                state.pending_report_read = Some(rp.clone());
                                return StepDecision::ScheduleActivity {
                                    id: format!("read-build-{id}"),
                                    activity: ActivityRequest::ReportFile(
                                        ReportFileRequest::Read { path: rp },
                                    ),
                                };
                            }
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
                        let (child_id, input_val) = self.build_review_child(state);
                        state.active_child_id = Some(child_id.clone());
                        StepDecision::StartChild {
                            id: child_id,
                            workflow_type: "single_worker".to_string(),
                            input: input_val,
                        }
                    }
                    PipelinePhase::Review => {
                        state.rounds_run = state.round;

                        if let Some(findings) = output.get("findings").and_then(|v| v.as_u64()) {
                            let challenge = output
                                .get("challenge")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            return self.handle_review_verdict(state, findings as u32, challenge);
                        }

                        if let Some(verdict) = output.get("verdict").and_then(|v| v.as_str()) {
                            match parse_verdict_str(verdict) {
                                Ok((findings, challenge)) => {
                                    return self.handle_review_verdict(state, findings, challenge);
                                }
                                Err(reason) => {
                                    state.phase = PipelinePhase::Failed;
                                    return StepDecision::Fail { code: 6, reason };
                                }
                            }
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

                        // No findings, verdict, or report_path -> fail closed!
                        state.phase = PipelinePhase::Failed;
                        StepDecision::Fail {
                            code: 6,
                            reason: "review child completed without verdict or report_path"
                                .to_string(),
                        }
                    }
                    PipelinePhase::Fix => {
                        if let Some(c) = output.get("commit").and_then(|v| v.as_str()) {
                            state.head_sha = Some(c.to_string());
                        }

                        let report_path = output
                            .get("report_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let Some(rp) = report_path {
                            if let Some(challenge) =
                                output.get("challenge").and_then(|v| v.as_bool())
                            {
                                if challenge {
                                    state.phase = PipelinePhase::Disputed;
                                    return StepDecision::Complete(PipelineOutput {
                                        status: "disputed".to_string(),
                                        rounds: state.rounds_run,
                                        review_changed: state.review_changed,
                                        commit: state.head_sha.clone(),
                                    });
                                }
                            } else {
                                state.pending_report_read = Some(rp.clone());
                                return StepDecision::ScheduleActivity {
                                    id: format!("read-fix-{id}"),
                                    activity: ActivityRequest::ReportFile(
                                        ReportFileRequest::Read { path: rp },
                                    ),
                                };
                            }
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
                                    if let Some(commit) = extract_commit_from_report(&content) {
                                        state.head_sha = Some(commit);
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
                                    Err(reason) => {
                                        state.phase = PipelinePhase::Failed;
                                        StepDecision::Fail { code: 6, reason }
                                    }
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
                            state.phase = PipelinePhase::Failed;
                            StepDecision::Fail {
                                code: 3,
                                reason: format!(
                                    "activity reading {pending_path} failed or returned unexpected result: {other:?}"
                                ),
                            }
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
                state.phase = PipelinePhase::Failed;
                StepDecision::Fail { code, reason }
            }
            Some(WorkflowEvent::TimerFired { id: _ }) => StepDecision::Continue,
        }
    }
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
    {
        return Ok((0, false));
    }

    for part in rest.split_whitespace() {
        let clean_part = part.trim_matches(|c: char| {
            c == ':' || c == ',' || c == '.' || c == '(' || c == ')' || c == '[' || c == ']'
        });
        if let Ok(n) = clean_part.parse::<u32>() {
            return Ok((n, false));
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
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("Verdict:") || t.starts_with("verdict:") {
            return parse_verdict_str(t);
        }
    }
    Err("review report has no 'Verdict: clean | <N> findings' line".to_string())
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
                if !sha.is_empty() {
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

        // Step 1: Initial step triggers plan child with distinct plan id
        let d1 = wf.step(&mut state, &mut ctx, None);
        match d1 {
            StepDecision::StartChild {
                id, workflow_type, ..
            } => {
                assert_eq!(id, "sude-task-plan-plan");
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
                id: "sude-task-plan-plan".to_string(),
                output: json!({ "exit": 0, "challenge": false, "report_path": "runs/plan.md" }),
            }),
        );
        match d2 {
            StepDecision::PauseForApproval { reason } => {
                assert!(reason.contains("approve"));
                assert!(reason.contains("runs/plan.md"));
            }
            other => panic!("expected PauseForApproval, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::PlanReadyGate);
        assert_eq!(state.planfile.as_deref(), Some("runs/plan.md"));

        // Step 3: Approval resolved -> advances to build child with distinct build id
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
                assert_eq!(id, "sude-task-plan-build");
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

        // Jump to Build phase and start build child
        state.phase = PipelinePhase::Build;
        state.active_child_id = Some("sude-task-fix-build".to_string());

        // Build completes -> launches reviewer child round 1
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-fix-build".to_string(),
                output: json!({ "exit": 0, "commit": "abc1234", "challenge": false }),
            }),
        );
        match d1 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "suhuynh-task-fix-review-r1");
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
                id: "suhuynh-task-fix-review-r1".to_string(),
                output: json!({ "exit": 0, "findings": 2 }),
            }),
        );
        match d2 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-fix-fix-r1");
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
                id: "sude-task-fix-fix-r1".to_string(),
                output: json!({ "exit": 0, "challenge": false }),
            }),
        );
        match d3 {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "suhuynh-task-fix-review-r2");
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
                id: "suhuynh-task-fix-review-r2".to_string(),
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
        let mut ctx = WorkflowContext::new(1000);
        let mut journal = InMemoryJournal::default();

        // 1. Initial step emits plan child (seq 1)
        let d1 = wf.step(&mut state, &mut ctx, None);
        match d1 {
            StepDecision::StartChild {
                id,
                workflow_type,
                input,
            } => {
                assert_eq!(id, "sude-task-resume-plan");
                assert_eq!(workflow_type, "single_worker");
                assert_eq!(input["report_path"], "runs/sude-task-resume-plan.md");
            }
            other => panic!("expected StartChild for plan, got {other:?}"),
        }
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(1, 1000, "child_started", "id=sude-task-resume-plan"),
            )
            .unwrap();

        // 2. Plan child completes -> PauseForApproval (seq 2)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-resume-plan".to_string(),
                output: json!({ "exit": 0, "report_path": "runs/plan.md", "challenge": false }),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(2, 1100, "child_completed", "id=sude-task-resume-plan"),
            )
            .unwrap();

        // 3. Operator approves -> start build child (seq 3, 4)
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
                &EventRecord::new(3, 1200, "approval_resolved", "approved=true"),
            )
            .unwrap();
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(4, 1200, "child_started", "id=sude-task-resume-build"),
            )
            .unwrap();

        // 4. Build child completes -> advances to Review round 1 (seq 5)
        let _ = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "sude-task-resume-build".to_string(),
                output: json!({ "exit": 0, "commit": "deadbeef", "challenge": false }),
            }),
        );
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(5, 1300, "child_completed", "id=sude-task-resume-build"),
            )
            .unwrap();

        assert_eq!(state.phase, PipelinePhase::Review);
        assert_eq!(state.round, 1);

        // Snapshot the state at sequence 5
        let snapshot_json = serde_json::to_string(&state).expect("serialize snapshot");
        journal
            .append_snapshot("pipe-task-resume", &SnapshotRecord::new(5, snapshot_json))
            .expect("append snapshot");

        // 5. Append in-flight event occurring after snapshot (seq 6: review child completes with 1 finding)
        journal
            .append_event(
                "pipe-task-resume",
                &EventRecord::new(
                    6,
                    1400,
                    "child_completed",
                    json!({
                        "id": "suhuynh-task-resume-review-r1",
                        "output": { "exit": 0, "findings": 1 }
                    })
                    .to_string(),
                ),
            )
            .unwrap();

        // 6. Simulate crash & recovery: restore snapshot + replay events since snapshot seq
        let loaded_snapshot = journal
            .load_latest_snapshot("pipe-task-resume")
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(loaded_snapshot.seq, 5);

        let mut restored_state: PipelineWorkflowState =
            serde_json::from_str(&loaded_snapshot.state_json).expect("deserialize snapshot");
        assert_eq!(restored_state.phase, PipelinePhase::Review);
        assert_eq!(restored_state.round, 1);

        let subsequent_events = journal
            .load_events("pipe-task-resume", loaded_snapshot.seq + 1)
            .expect("load subsequent events");
        assert_eq!(subsequent_events.len(), 1);

        // Replay event seq 6 onto restored state
        let event6_json: serde_json::Value =
            serde_json::from_str(&subsequent_events[0].payload_json).unwrap();
        let replayed_event = WorkflowEvent::ChildCompleted {
            id: event6_json["id"].as_str().unwrap().to_string(),
            output: event6_json["output"].clone(),
        };
        let decision = wf.step(&mut restored_state, &mut ctx, Some(replayed_event));

        // Invariant: resumes at Fix child for round 1, without replaying plan or build child
        match decision {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-resume-fix-r1");
            }
            other => panic!("expected StartChild for fix r1, got {other:?}"),
        }
        assert_eq!(restored_state.phase, PipelinePhase::Fix);
        assert_eq!(restored_state.last_findings, 1);

        // Verify historical journal contains plan/build events only up to snapshot, never re-appended
        let all_events = journal.load_events("pipe-task-resume", 1).unwrap();
        let plan_starts = all_events
            .iter()
            .filter(|e| e.payload_json.contains("sude-task-resume-plan"))
            .count();
        let build_starts = all_events
            .iter()
            .filter(|e| e.payload_json.contains("sude-task-resume-build"))
            .count();
        assert_eq!(plan_starts, 2); // 1 start + 1 complete
        assert_eq!(build_starts, 2); // 1 start + 1 complete
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

        // Register mock reports for each child
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
        assert_eq!(output.commit.as_deref(), Some("c0ffee2"));
    }

    #[test]
    fn test_review_with_no_verdict_fails_closed() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-noverdict", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;

        let mut ctx = WorkflowContext::new(1000);

        // Case 1: ChildCompleted with no verdict and no report_path fails immediately with code 6
        let d1 = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-noverdict-review-r1".to_string(),
                output: json!({ "exit": 0 }),
            }),
        );
        match d1 {
            StepDecision::Fail { code, reason } => {
                assert_eq!(code, 6);
                assert!(reason.contains("without verdict or report_path"));
            }
            other => panic!("expected Fail code 6, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Failed);

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
            StepDecision::Fail { code, reason } => {
                assert_eq!(code, 6);
                assert!(reason.contains("no 'Verdict: clean | <N> findings' line"));
            }
            other => panic!("expected Fail code 6, got {other:?}"),
        }
        assert_eq!(state2.phase, PipelinePhase::Failed);
    }

    #[test]
    fn test_review_verdict_with_findings_substring_clean_parses_findings() {
        let wf = PipelineWorkflow;
        let input = PipelineInput::new("task-substr", "/repo", "sude", "suhuynh", "brief");
        let mut state = PipelineWorkflowState::new(input);
        state.phase = PipelinePhase::Review;
        state.round = 1;

        let mut ctx = WorkflowContext::new(1000);

        // Line has both numbers and "clean" (e.g. "Verdict: 2 findings (not clean)")
        // Must parse as 2 findings, NOT clean!
        let decision = wf.step(
            &mut state,
            &mut ctx,
            Some(WorkflowEvent::ChildCompleted {
                id: "suhuynh-task-substr-review-r1".to_string(),
                output: json!({
                    "exit": 0,
                    "verdict": "Verdict: 2 findings (not clean)"
                }),
            }),
        );
        match decision {
            StepDecision::StartChild { id, .. } => {
                assert_eq!(id, "sude-task-substr-fix-r1");
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
            StepDecision::Fail { code, reason } => {
                assert_eq!(code, 3);
                assert!(reason.contains("runs/missing.md"));
            }
            other => panic!("expected Fail code 3, got {other:?}"),
        }
        assert_eq!(state.phase, PipelinePhase::Failed);
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
                output: json!({ "exit": 0, "report_path": "runs/build-chal.md" }),
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
                output: json!({ "exit": 0, "findings": 0 }),
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
