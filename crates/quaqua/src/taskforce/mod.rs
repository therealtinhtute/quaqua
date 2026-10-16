//! Taskforce subcommands (`quaqua taskforce {advisor, dual-lens}`).
//! Reproduces piggery taskforce behavior.

use crate::cap::count_owner_slots;
use crate::git::Git;
use crate::herdr::Herdr;
use crate::worker::cleanup::home_quaqua_runs;
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
    let runs_dir = home_quaqua_runs();
    let _ = std::fs::create_dir_all(&runs_dir);

    let slots_used = count_owner_slots(&runs_dir, &owner, None, herdr);
    let weight = if targs.mode == "dual-lens" { 2 } else { 1 };
    let max_workers = 4;

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
        run_advisor(&targs, &brief_content, &runs_dir, &owner, git, herdr)
    } else {
        run_dual_lens(&targs, &brief_content, &runs_dir, &owner, git, herdr)
    }
}

fn run_advisor(
    args: &TaskforceArgs,
    brief: &str,
    runs_dir: &Path,
    _owner: &str,
    git: &Git,
    herdr: &Herdr,
) -> Result<(), (i32, String)> {
    let worker_name = format!("quansu-{}", args.task);
    let cfg = crate::config::load_team_config()?;

    let opts = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: worker_name.clone(),
        kind: "quansu".to_string(),
        repo_path: args.repo.clone(),
    };

    crate::worker::spawn::spawn_worker(opts, &cfg, git, herdr)?;

    // Write taskforce marker
    let tf_marker = runs_dir.join(format!("{worker_name}.taskforce"));
    let _ = std::fs::write(&tf_marker, &args.task);

    let prompt_text = format!(
        "Goal: advisor recommendation for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: recommendation + confidence, evidence, alternatives, strongest downside, and the smallest check that would prove it wrong (marked not run). Write to a temp file and rename it to ~/quaqua/runs/{worker_name}.md — publish atomically, never a partial file.",
        args.task, brief
    );

    let prompt_res = crate::worker::prompt::prompt_worker(&worker_name, Some(&prompt_text), false, None, git, herdr);

    let _ = crate::worker::cleanup::cleanup_worker(&worker_name, &args.repo, git, herdr);
    let _ = std::fs::remove_file(tf_marker);

    prompt_res
}

fn run_dual_lens(
    args: &TaskforceArgs,
    brief: &str,
    runs_dir: &Path,
    owner: &str,
    git: &Git,
    herdr: &Herdr,
) -> Result<(), (i32, String)> {
    let state_file = runs_dir.join(format!("taskforce-{}.state", args.task));
    let pid = std::process::id();
    let state_content = format!(
        "task={}\nkind=dual-lens\nowner={}\nfinished=0\ntaskforce_pid={}\n",
        args.task, owner, pid
    );
    let _ = std::fs::write(&state_file, state_content);

    let cfg = crate::config::load_team_config()?;
    let w1 = format!("truonglao-{}dl1", args.task);
    let w2 = format!("quansu-{}dl2", args.task);

    let opts1 = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: w1.clone(),
        kind: "truonglao".to_string(),
        repo_path: args.repo.clone(),
    };

    let opts2 = crate::worker::spawn::SpawnOptions {
        base: args.base.clone(),
        review: true,
        name: w2.clone(),
        kind: "quansu".to_string(),
        repo_path: args.repo.clone(),
    };

    if let Err(e) = crate::worker::spawn::spawn_worker(opts1, &cfg, git, herdr) {
        let _ = std::fs::write(&state_file, format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\n", args.task, owner));
        return Err(e);
    }
    if let Err(e) = crate::worker::spawn::spawn_worker(opts2, &cfg, git, herdr) {
        let _ = crate::worker::cleanup::cleanup_worker(&w1, &args.repo, git, herdr);
        let _ = std::fs::write(&state_file, format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\n", args.task, owner));
        return Err(e);
    }

    let tf1 = runs_dir.join(format!("{w1}.taskforce"));
    let tf2 = runs_dir.join(format!("{w2}.taskforce"));
    let _ = std::fs::write(&tf1, &args.task);
    let _ = std::fs::write(&tf2, &args.task);

    let prompt1 = format!(
        "Goal: dual-lens assessment (lens 1) for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{w1}.md — publish atomically, never a partial file.",
        args.task, brief
    );
    let prompt2 = format!(
        "Goal: dual-lens assessment (lens 2) for task '{}'\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs.\nCurrent design (open to challenge): {}\nEvidence: assessment, agreements, and concerns. Write to a temp file and rename it to ~/quaqua/runs/{w2}.md — publish atomically, never a partial file.",
        args.task, brief
    );

    let _ = crate::worker::prompt::prompt_worker(&w1, Some(&prompt1), false, None, git, herdr);
    let _ = crate::worker::prompt::prompt_worker(&w2, Some(&prompt2), false, None, git, herdr);

    let _ = crate::worker::cleanup::cleanup_worker(&w1, &args.repo, git, herdr);
    let _ = crate::worker::cleanup::cleanup_worker(&w2, &args.repo, git, herdr);
    let _ = std::fs::remove_file(tf1);
    let _ = std::fs::remove_file(tf2);

    let _ = std::fs::write(
        &state_file,
        format!("task={}\nkind=dual-lens\nowner={}\nfinished=1\ntaskforce_pid={}\n", args.task, owner, pid),
    );

    println!("taskforce dual-lens for {} complete", args.task);
    Ok(())
}
