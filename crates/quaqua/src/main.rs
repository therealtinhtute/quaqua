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
    /// Agent grid layout planner and reconciler.
    Grid {
        #[command(subcommand)]
        cmd: GridCmd,
    },
}

#[derive(Subcommand)]
enum GridCmd {
    /// Pure layout planner: snapshot JSON on stdin/file -> ops JSON on stdout.
    Plan {
        /// Path to snapshot JSON file (default: stdin).
        #[arg(long, short)]
        file: Option<PathBuf>,
        /// Optional positional snapshot file path.
        path: Option<PathBuf>,
    },
    /// Reconcile worker panes into the agent grid layout.
    Reconcile {
        /// Block until grid check has no pending moves for these workers.
        #[arg(long)]
        check: bool,
        /// Repair / arrange workers into grid (default).
        #[arg(long)]
        repair: bool,
        /// Label of the worker tab (default: from team.toml [grid] or 'workers').
        #[arg(long)]
        tab: Option<String>,
        /// Comma-separated worker names to scope moves.
        #[arg(long)]
        only: Option<String>,
        /// Owner scope (default: $QUAQUA_OWNER or 'cos').
        #[arg(long)]
        owner: Option<String>,
        /// Print planned actions without executing.
        #[arg(long)]
        dry_run: bool,
        /// Path to team.toml.
        #[arg(long)]
        team_file: Option<PathBuf>,
    },
    /// Herdr event hook reconciler.
    Hook,
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

fn do_grid_plan(file_opt: Option<PathBuf>, path_opt: Option<PathBuf>) -> Result<(), String> {
    let target = file_opt.or(path_opt);
    let content = match target {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            buf
        }
    };

    let val: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("invalid snapshot JSON: {e}"))?;
    let snap = quaqua::grid::parse_snapshot_value(&val)?;
    let def_statuses = team_file(&None)
        .ok()
        .and_then(|p| config::load(&p).ok())
        .map(|c| c.grid.deferred_statuses);
    let def_slices: Option<Vec<&str>> = def_statuses
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());
    let ops = quaqua::grid::plan_with_deferred(&snap, def_slices.as_deref())?;
    let json_out = serde_json::to_string(&ops).map_err(|e| e.to_string())?;
    println!("{json_out}");
    Ok(())
}

fn do_grid_reconcile(
    check: bool,
    repair: bool,
    tab: Option<String>,
    only: Option<String>,
    owner: Option<String>,
    dry_run: bool,
    team_opt: Option<PathBuf>,
) -> Result<(), String> {
    let (default_workers_tab, def_statuses) = if let Ok(tf) = team_file(&team_opt) {
        let cfg = config::load(&tf).ok();
        (
            cfg.as_ref().map(|c| c.grid.workers_tab.clone()).unwrap_or_else(|| "workers".to_string()),
            cfg.map(|c| c.grid.deferred_statuses),
        )
    } else {
        ("workers".to_string(), None)
    };

    if check && repair {
        return Err("cannot specify both --check and --repair".to_string());
    }

    let opts = quaqua::grid::ReconcileOptions {
        check,
        repair: if check { false } else { true },
        tab,
        only,
        owner,
        dry_run,
        timeout_sec: None,
    };
    let mut driver = quaqua::grid::CliHerdrDriver::new();
    let def_slices: Option<Vec<&str>> = def_statuses
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());
    quaqua::grid::run_reconcile(&mut driver, &opts, &default_workers_tab, def_slices.as_deref())
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
        Some(Command::Grid { cmd }) => match cmd {
            GridCmd::Plan { file, path } => match do_grid_plan(file, path) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
            GridCmd::Reconcile {
                check,
                repair,
                tab,
                only,
                owner,
                dry_run,
                team_file,
            } => match do_grid_reconcile(check, repair, tab, only, owner, dry_run, team_file) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
            GridCmd::Hook => match quaqua::grid::run_hook() {
                Ok(()) => 0,
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
