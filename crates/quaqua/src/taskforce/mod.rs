//! Taskforce subcommands (`quaqua taskforce {advisor, dual-lens}`).
//! Reproduces piggery taskforce behavior.

use crate::cap::count_owner_slots;
use crate::git::Git;
use crate::herdr::Herdr;
use crate::worker::cleanup::default_runs_dir;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
# Usage: quaqua taskforce <advisor|dual-lens> <task> <repo> --brief <file> [--base <ref>]";

pub struct TaskforceArgs {
    pub mode: String,
    pub task: String,
    pub repo: PathBuf,
    pub brief: PathBuf,
    pub base: Option<String>,
}

pub fn parse_taskforce_args(args: &[String]) -> Result<TaskforceArgs, (i32, String)> {
    if args.is_empty() {
        return Err((1, format!("{}\n", USAGE)));
    }
    let mode = args[0].clone();
    if mode != "advisor" && mode != "dual-lens" {
        return Err((1, format!("{}\n", USAGE)));
    }

    let mut brief: Option<PathBuf> = None;
    let mut base: Option<String> = None;
    let mut pos = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--brief" {
            if i + 1 >= args.len() {
                return Err((1, format!("{}\n", USAGE)));
            }
            brief = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if let Some(v) = arg.strip_prefix("--brief=") {
            brief = Some(PathBuf::from(v));
            i += 1;
        } else if arg == "--base" {
            if i + 1 >= args.len() {
                return Err((1, format!("{}\n", USAGE)));
            }
            base = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(v) = arg.strip_prefix("--base=") {
            base = Some(v.to_string());
            i += 1;
        } else if arg.starts_with('-') {
            return Err((1, format!("{}\n", USAGE)));
        } else {
            pos.push(arg.clone());
            i += 1;
        }
    }

    if pos.len() < 2 || brief.is_none() {
        return Err((1, format!("{}\n", USAGE)));
    }

    Ok(TaskforceArgs {
        mode,
        task: pos[0].clone(),
        repo: PathBuf::from(&pos[1]),
        brief: brief.unwrap(),
        base,
    })
}

pub fn handle_taskforce(args: &[String], git: &Git, herdr: &Herdr) -> Result<(), (i32, String)> {
    let targs = parse_taskforce_args(args)?;
    let owner = std::env::var("QUAQUA_OWNER").unwrap_or_else(|_| "cos".to_string());
    let runs_dir = default_runs_dir();
    let _ = std::fs::create_dir_all(&runs_dir);

    let cfg = crate::config::load_team_config()?;
    let max_workers = cfg.limits.max_workers_per_orchestrator;
    let weight = cfg
        .limits
        .weights
        .get(&targs.mode)
        .copied()
        .unwrap_or(if targs.mode == "dual-lens" { 2 } else { 1 });

    let slots_used = count_owner_slots(&runs_dir, &owner, None, herdr);
    if slots_used + weight > max_workers {
        return Err((
            2,
            format!(
                "error: worker cap exceeded (owner '{owner}' has {slots_used} slots in use; {} reserves {weight}, cap is {max_workers})\n",
                targs.mode
            ),
        ));
    }

    let brief_content = std::fs::read_to_string(&targs.brief)
        .map_err(|e| (1, format!("cannot read brief {}: {e}\n", targs.brief.display())))?;

    if targs.mode == "advisor" {
        run_advisor(&targs, &brief_content, &runs_dir, &owner, &cfg, git, herdr)
    } else {
        run_dual_lens(&targs, &brief_content, &runs_dir, &owner, &cfg, git, herdr)
    }
}

fn run_advisor(
    args: &TaskforceArgs,
    brief: &str,
    runs_dir: &Path,
    owner: &str,
    cfg: &crate::config::Config,
    git: &Git,
    herdr: &Herdr,
) -> Result<(), (i32, String)> {
    let members = cfg
        .taskforce
        .get("advisor")
        .map(|t| t.members.clone())
        .unwrap_or_else(|| vec!["quansu".to_string()]);
    let kind = members.first().map(|s| s.as_str()).unwrap_or("quansu");
    let worker_name = format!("{kind}-{}", args.task);

    let opts = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: worker_name.clone(),
        kind: kind.to_string(),
        repo_path: args.repo.clone(),
    };

    // Set PIPELINE_TASK during inner spawn to prevent double-charging the reservation
    std::env::set_var("PIPELINE_TASK", &args.task);
    std::env::set_var("QUAQUA_OWNER", owner);
    let spawn_res = crate::worker::spawn::spawn_worker(opts, cfg, git, herdr);
    std::env::remove_var("PIPELINE_TASK");
    spawn_res?;

    // Write taskforce marker
    let tf_marker = runs_dir.join(format!("{worker_name}.taskforce"));
    let _ = std::fs::write(&tf_marker, &args.task);

    let prompt_text = format!(
        "Goal: advisor recommendation for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: recommendation + confidence, evidence, alternatives, strongest downside, and the smallest check that would prove it wrong (marked not run). Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file.",
        args.task, brief
    );

    let prompt_res = crate::worker::prompt::prompt_worker(&worker_name, Some(&prompt_text), false, None, git, herdr);
    let report_path = runs_dir.join(format!("{worker_name}.md"));
    if prompt_res.is_ok() {
        println!("{}", report_path.display());
    }
    prompt_res
}

fn run_dual_lens(
    args: &TaskforceArgs,
    brief: &str,
    runs_dir: &Path,
    owner: &str,
    cfg: &crate::config::Config,
    git: &Git,
    herdr: &Herdr,
) -> Result<(), (i32, String)> {
    let members = cfg
        .taskforce
        .get("dual-lens")
        .map(|t| t.members.clone())
        .unwrap_or_else(|| vec!["truonglao".to_string(), "quansu".to_string()]);
    let kind1 = members.first().map(|s| s.as_str()).unwrap_or("truonglao");
    let kind2 = members.get(1).map(|s| s.as_str()).unwrap_or("quansu");

    let w1 = format!("{kind1}-{}dl1", args.task);
    let w2 = format!("{kind2}-{}dl2", args.task);

    let state_file = runs_dir.join(format!("taskforce-{}.state", args.task));
    let pid = std::process::id();
    let state_content = format!(
        "task={}\nkind=dual-lens\nowner={}\nfinished=0\ntaskforce_pid={}\n",
        args.task, owner, pid
    );
    let _ = std::fs::write(&state_file, state_content);

    let tf1 = runs_dir.join(format!("{w1}.taskforce"));
    let tf2 = runs_dir.join(format!("{w2}.taskforce"));
    let _ = std::fs::write(&tf1, &args.task);
    let _ = std::fs::write(&tf2, &args.task);

    let opts1 = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: w1.clone(),
        kind: kind1.to_string(),
        repo_path: args.repo.clone(),
    };

    let opts2 = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: w2.clone(),
        kind: kind2.to_string(),
        repo_path: args.repo.clone(),
    };

    std::env::set_var("PIPELINE_TASK", &args.task);
    std::env::set_var("QUAQUA_OWNER", owner);
    let s1 = crate::worker::spawn::spawn_worker(opts1, cfg, git, herdr);
    let s2 = if s1.is_ok() {
        crate::worker::spawn::spawn_worker(opts2, cfg, git, herdr)
    } else {
        Ok(())
    };
    std::env::remove_var("PIPELINE_TASK");

    if let Err(e) = s1 {
        let _ = std::fs::write(&state_file, format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\n", args.task, owner));
        return Err(e);
    }
    if let Err(e) = s2 {
        let _ = crate::worker::cleanup::cleanup_worker(&w1, &args.repo, git, herdr);
        let _ = std::fs::write(&state_file, format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\n", args.task, owner));
        return Err(e);
    }

    let prompt1 = format!(
        "Goal: dual-lens assessment (lens 1) for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{w1}.md — publish atomically, never a partial file.",
        args.task, brief
    );
    let prompt2 = format!(
        "Goal: dual-lens assessment (lens 2) for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{w2}.md — publish atomically, never a partial file.",
        args.task, brief
    );

    let p1_res = crate::worker::prompt::prompt_worker(&w1, Some(&prompt1), false, None, git, herdr);
    let p2_res = crate::worker::prompt::prompt_worker(&w2, Some(&prompt2), false, None, git, herdr);

    let _ = std::fs::write(
        &state_file,
        format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\ntaskforce_pid={}\n", args.task, owner, pid),
    );

    p1_res?;
    p2_res?;

    println!("{}", runs_dir.join(format!("{w1}.md")).display());
    println!("{}", runs_dir.join(format!("{w2}.md")).display());
    Ok(())
}
