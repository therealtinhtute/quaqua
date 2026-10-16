//! `quaqua` — the orchestration CLI.
//! R0 scaffolded the crate; R1 added `config`; R3 adds `worker` and `cap`.

use clap::{Args, Parser, Subcommand};
use quaqua::cap::count_owner_slots;
use quaqua::config;
use quaqua::git::Git;
use quaqua::herdr::Herdr;
use quaqua::worker::cleanup::{cleanup_worker, default_runs_dir};
use quaqua::worker::prompt::{do_check_brief, prompt_worker, USAGE as PROMPT_USAGE};
use quaqua::worker::spawn::{parse_spawn_args, spawn_worker};
use std::path::{Path, PathBuf};

const CLEANUP_USAGE: &str = "\
# Usage: cleanup-worker.sh <name> <repo-path>
# Stops the agent pane and removes its worktree. Branch quaqua/<name> is kept.";

const CAP_USAGE: &str = "\
# Usage: worker-cap.sh <owner> [<exclude-worker>]";

#[derive(Parser)]
#[command(name = "quaqua", version, about = "quaqua orchestration CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Read and check the repo team.toml.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Worker operations: spawn, prompt, cleanup.
    Worker {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Print the number of active worker slots used by owner.
    Cap {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Validate team.toml (exit 0 = ok, non-zero = rejected with a clear error).
    Validate(ConfigArgs),
    /// Print the resolved value of a dotted key, e.g. strategy.default_reviewer.
    Get {
        key: String,
        #[command(flatten)]
        args: ConfigArgs,
    },
}

#[derive(Args)]
struct ConfigArgs {
    /// Path to team.toml (default: $QUAQUA_TEAM_TOML, else ./team.toml).
    #[arg(long)]
    file: Option<PathBuf>,
    /// Path to models.md for the drift check (default: sibling of team.toml).
    #[arg(long)]
    models: Option<PathBuf>,
    /// Validate the config as seen by this project (applies its override).
    #[arg(long)]
    project: Option<String>,
}

fn team_file(explicit: &Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p.clone());
    }
    if let Ok(p) = std::env::var("QUAQUA_TEAM_TOML") {
        return Ok(PathBuf::from(p));
    }
    if PathBuf::from("team.toml").is_file() {
        return Ok(PathBuf::from("team.toml"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut curr = cwd.as_path();
        loop {
            let candidate = curr.join("team.toml");
            if candidate.is_file() {
                return Ok(candidate);
            }
            match curr.parent() {
                Some(p) => curr = p,
                None => break,
            }
        }
    }
    Ok(PathBuf::from("team.toml"))
}

fn do_validate(args: &ConfigArgs) -> Result<(), String> {
    let file = team_file(&args.file)?;
    let cfg = config::load(&file)?;
    let models = config::load_models(args.models.as_deref(), &file)?;
    config::validate(&cfg, models.as_ref())?;
    if let Some(slug) = &args.project {
        if !cfg.projects.contains_key(slug) {
            return Err(format!("unknown project '{slug}'"));
        }
    }
    Ok(())
}

fn do_get(key: &str, args: &ConfigArgs) -> Result<String, String> {
    let file = team_file(&args.file)?;
    let cfg = config::load(&file)?;
    config::get(&cfg, key, args.project.as_deref())
}

fn handle_worker(args: &[String], git: &Git, herdr: &Herdr) -> Result<(), (i32, String)> {
    if args.is_empty() {
        return Err((1, "error: worker subcommand required (spawn, prompt, cleanup)\n".to_string()));
    }
    match args[0].as_str() {
        "spawn" => {
            let opts = parse_spawn_args(&args[1..])?;
            let tfile = team_file(&None)
                .map_err(|e| (1, format!("error: {e}\n")))?;
            let cfg = config::load(&tfile)
                .map_err(|e| (1, format!("error: {e}\n")))?;
            spawn_worker(opts, &cfg, git, herdr)?;
            Ok(())
        }
        "prompt" => {
            let p_args = &args[1..];
            if p_args.is_empty() {
                return Err((1, format!("{PROMPT_USAGE}\n")));
            }
            if p_args[0] == "--check-brief" {
                if p_args.len() != 2 || p_args[1].is_empty() {
                    return Err((1, format!("{PROMPT_USAGE}\n")));
                }
                do_check_brief(&p_args[1])?;
                return Ok(());
            }
            if p_args[0] == "--resume" {
                let mut name = "";
                let mut extend: Option<u64> = None;
                let mut i = 1;
                while i < p_args.len() {
                    let a = &p_args[i];
                    if a == "--extend" {
                        if i + 1 >= p_args.len() {
                            return Err((1, format!("{PROMPT_USAGE}\n")));
                        }
                        let val = &p_args[i + 1];
                        if !val.chars().all(|c| c.is_ascii_digit()) || val.starts_with('0') || val.is_empty() {
                            return Err((1, format!("{PROMPT_USAGE}\n")));
                        }
                        extend = val.parse::<u64>().ok();
                        i += 2;
                    } else if a.starts_with('-') {
                        return Err((1, format!("{PROMPT_USAGE}\n")));
                    } else {
                        if !name.is_empty() {
                            return Err((1, format!("{PROMPT_USAGE}\n")));
                        }
                        name = a.as_str();
                        i += 1;
                    }
                }
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                    return Err((1, format!("{PROMPT_USAGE}\n")));
                }
                prompt_worker(name, None, true, extend, git, herdr)?;
                return Ok(());
            }

            // Normal prompt: <name> <task>
            if p_args.len() != 2 {
                return Err((1, format!("{PROMPT_USAGE}\n")));
            }
            let name = &p_args[0];
            let task = &p_args[1];
            if task.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err((1, format!("{PROMPT_USAGE}\n")));
            }
            if !quaqua::worker::prompt::brief_headers_ok(task) {
                return Err((
                    1,
                    "error: task must be a brief with 'Goal:', 'Hard constraints:', 'Current design (open to challenge):' (rules/_all/brief.md)\n".to_string(),
                ));
            }
            prompt_worker(name, Some(task), false, None, git, herdr)?;
            Ok(())
        }
        "cleanup" => {
            if args.len() != 3 || args[1].is_empty() || args[2].is_empty() {
                return Err((1, format!("{CLEANUP_USAGE}\n")));
            }
            let name = &args[1];
            let repo = Path::new(&args[2]);
            cleanup_worker(name, repo, git, herdr)?;
            Ok(())
        }
        other => {
            Err((1, format!("error: unknown worker subcommand '{other}'\n")))
        }
    }
}

fn handle_cap(args: &[String], herdr: &Herdr) -> Result<(), (i32, String)> {
    if args.len() > 2 {
        return Err((1, format!("{CAP_USAGE}\n")));
    }
    let owner = args.first().map(String::as_str).unwrap_or("cos");
    let exclude = args.get(1).map(String::as_str);
    let runs_dir = default_runs_dir();
    let count = count_owner_slots(&runs_dir, owner, exclude, herdr);
    println!("{count}");
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let git = Git::new();
    let herdr = Herdr::new();

    let code = match cli.command {
        Some(Command::Config { cmd }) => match cmd {
            ConfigCmd::Validate(args) => match do_validate(&args) {
                Ok(()) => {
                    println!("ok");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
            ConfigCmd::Get { key, args } => match do_get(&key, &args) {
                Ok(value) => {
                    println!("{value}");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
        },
        Some(Command::Worker { args }) => match handle_worker(&args, &git, &herdr) {
            Ok(()) => 0,
            Err((rc, err)) => {
                if !err.is_empty() {
                    eprint!("{err}");
                }
                rc
            }
        },
        Some(Command::Cap { args }) => match handle_cap(&args, &herdr) {
            Ok(()) => 0,
            Err((rc, err)) => {
                if !err.is_empty() {
                    eprint!("{err}");
                }
                rc
            }
        },
        None => 0,
    };
    std::process::exit(code);
}
