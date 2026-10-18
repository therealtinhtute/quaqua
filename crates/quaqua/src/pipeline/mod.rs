//! Pipeline engine (`quaqua pipeline`).
//! Implements build -> review -> fix orchestration state machine.

pub mod phases;
pub mod state;

pub use phases::{
    blocked_outcome, harness_of_kind, has_real_blockers, pipeline_finished,
    select_default_reviewer, PipelineContext, StepResult,
};
pub use state::PipelineState;

use crate::cap::count_owner_slots;
use crate::config::load_team_config;
use crate::git::Git;
use crate::herdr::Herdr;
use crate::util::now_sec;
use crate::worker::cleanup::default_runs_dir;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
# Usage: pipeline.sh <task> <repo> --brief <file> [--builder sude] [--reviewer suhuynh]
#                    [--review auto|always|skip] [--rounds 2] [--base <ref>]
#        pipeline.sh --resume <task> <repo> [--rounds 2]
#   Build -> review -> fix loop on top of
#   spawn-worker.sh / prompt-worker.sh / cleanup-worker.sh; counts as 2 workers
#   toward the cap. <task> is a slug ([a-z0-9-]): builder <builder>-<task>, a
#   fresh reviewer <reviewer>-<task>r<N> each round (spawn-worker.sh --review).
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
#   Exit codes reuse prompt-worker.sh's watchdog meanings: 0 finished (status
#   line on stdout + runs/pipeline-<task>.md) · 1 usage/state error · 2 cap ·
#   3 stall/gone (also: resumable snapshot/spawn failures) · 4 budget · 5
#   plan-ready pause, a mandatory grid-arrange pause, or a worker blocked on
#   a question. On 2-5 fix the cause (a stuck wait keeps waiting via
#   'prompt-worker.sh --resume <worker>'), then 'pipeline.sh --resume <task>
#   <repo>' continues from runs/pipeline-<task>.state. Every stop appends one
#   line to runs/telemetry.jsonl {ts,task,repo,builder,reviewer,
#   review:run|skip,rounds,review_changed_outcome,status,minutes} — the last
#   line per task is current. Exit 6 (malformed report: prompt-worker.sh
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
            state.rounds = r;
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
