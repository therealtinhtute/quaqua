//! `quaqua` — the orchestration CLI. R0 scaffolded the crate; R1 adds
//! `config validate` and `config get`, which read the repo-root `team.toml`.

use clap::{Args, Parser, Subcommand};
use quaqua::config;
use std::path::PathBuf;

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
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Validate team.toml (exit 0 = ok, non-zero = rejected with a clear error).
    Validate(ConfigArgs),
    /// Print the resolved value of a dotted key, e.g. strategy.default_reviewer.
    Get {
        /// Dotted key, e.g. `strategy.default_reviewer`.
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
    Ok(PathBuf::from("team.toml"))
}

fn do_validate(args: &ConfigArgs) -> Result<(), String> {
    let file = team_file(&args.file)?;
    let cfg = config::load(&file)?;
    let models = config::load_models(args.models.as_deref(), &file)?;
    // Validate the raw file once. `validate` already bounds every [projects.*]
    // override (and the weights) against the top-level cap, so a bad override is
    // caught even without --project.
    config::validate(&cfg, models.as_ref())?;
    // Selecting a project only needs it to exist: its override was already
    // checked against the global baseline above. Re-validating a resolved config
    // would wrongly re-check the *other* projects against this project's cap.
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

fn main() {
    let cli = Cli::parse();
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
        None => 0,
    };
    std::process::exit(code);
}
