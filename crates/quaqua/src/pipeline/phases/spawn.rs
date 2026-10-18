//! Spawn phase implementation.

use super::{PipelineContext, StepResult};
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;
use std::path::Path;

pub fn run_spawn<G: GitOps, H: HerdrOps>(
    state: &mut PipelineState,
    ctx: &PipelineContext<'_, G, H>,
) -> Result<StepResult, (i32, String)> {
    let bname = format!("{}-{}", state.builder, ctx.task);
    if state.builder_spawned == 0 {
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
                "spawn-worker.sh failed for builder {bname} — fix the cause, then 'pipeline.sh --resume {} {}'",
                ctx.task, state.repo
            );
            return ctx.finish(state, "blocked", 3, Some(&b));
        }
        state.builder_spawned = 1;
        let _ = state.save(&ctx.state_file);
    }

    if !ctx.run_grid_hook() {
        let b = format!(
            "grid check failed for {bname} — arrange the pane per rules/_all/fanout-agent-grid.md, then 'pipeline.sh --resume {} {}'",
            ctx.task, state.repo
        );
        let _ = state.save(&ctx.state_file);
        return ctx.finish(state, "blocked", 5, Some(&b));
    }

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

    state.phase = "plan".to_string();
    state.prompted = 0;
    let _ = state.save(&ctx.state_file);
    Ok(StepResult::Continue)
}
