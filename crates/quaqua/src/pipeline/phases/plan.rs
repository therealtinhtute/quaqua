//! Plan phase implementation.

use super::{blocked_outcome, log_blockers, log_challenge, PipelineContext, StepResult};
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;

pub fn run_plan<G: GitOps, H: HerdrOps>(
    state: &mut PipelineState,
    ctx: &PipelineContext<'_, G, H>,
) -> Result<StepResult, (i32, String)> {
    let bname = format!("{}-{}", state.builder, ctx.task);
    let plan_prompt = format!(
        "{}\n\nPipeline phase: PLAN — write only the implementation plan (steps, files to touch, risks) as your runs file; do not change code yet. The plan is approved before you build. If the premise is wrong, write '## Challenge' with evidence instead of planning around it.",
        ctx.brief_text
    );

    ctx.wait_file(state, &bname, &plan_prompt, ctx.plan_budget)?;

    let planfile = ctx.runs_dir.join(format!("{bname}.plan.md"));
    let cur_md = ctx.runs_dir.join(format!("{bname}.md"));
    let _ = std::fs::rename(&cur_md, &planfile);

    state.planfile = planfile.to_string_lossy().to_string();
    let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
    pcontent.push_str(&format!(
        "- plan: {}\n",
        planfile.file_name().and_then(|n| n.to_str()).unwrap_or("")
    ));
    let _ = std::fs::write(&ctx.pfile, pcontent);
    let _ = state.save(&ctx.state_file);
    if let Ok(content) = std::fs::read_to_string(&planfile) {
        if content
            .lines()
            .any(|l| l.trim().starts_with("## Challenge"))
        {
            log_challenge(&planfile, &ctx.pfile);
            let b = "builder challenged the premise at plan stage — Lead resolves the Challenge above";
            return ctx.finish(state, "disputed", 0, Some(b));
        }
    }

    if blocked_outcome(&planfile) {
        log_blockers(&planfile, &ctx.pfile);
        let b = "builder's plan reports unfinished work — Lead decides how to proceed";
        return ctx.finish(state, "blocked", 0, Some(b));
    }

    state.phase = "build".to_string();
    state.prompted = 0;
    let _ = state.save(&ctx.state_file);

    eprintln!(
        "PIPELINE plan-ready: approve {} then resume: pipeline.sh --resume {} {}",
        state.planfile, ctx.task, state.repo
    );
    ctx.finish(state, "plan-ready", 5, None)
}
