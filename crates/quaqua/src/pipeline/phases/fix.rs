//! Fix phase implementation.

use super::{blocked_outcome, log_blockers, log_challenge, PipelineContext, StepResult};
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;

pub fn run_fix<G: GitOps, H: HerdrOps>(
    state: &mut PipelineState,
    ctx: &PipelineContext<'_, G, H>,
) -> Result<StepResult, (i32, String)> {
    let bname = format!("{}-{}", state.builder, ctx.task);
    let revname = format!("{}-{}r{}", state.reviewer, ctx.task, state.round);
    let fix_prompt = format!(
        "Goal: resolve the {} review findings for task '{}'\nHard constraints: stay in your worktree on quaqua/{bname}; commit each fix; keep tests green. A finding that is wrong is NOT fixed — dispute it under '## Challenge' in your runs file with evidence.\nCurrent design (open to challenge): reviewer {revname} reported {} findings — full list in {}/{revname}.md (readable); your worktree and branch are unchanged.\nUnknowns: which findings are real — verify each against the code before changing anything.\nEvidence: committed fixes + test output, or '## Challenge' per wrong finding.\nPipeline phase: FIX (after review round {})",
        state.last_findings,
        ctx.task,
        state.last_findings,
        ctx.runs_dir.display(),
        state.round
    );

    ctx.wait_file(state, &bname, &fix_prompt, ctx.fix_budget)?;

    let fix_md = ctx.runs_dir.join(format!("{bname}.fix{}.md", state.round));
    let cur_md = ctx.runs_dir.join(format!("{bname}.md"));
    let _ = std::fs::rename(&cur_md, &fix_md);

    if let Ok(content) = std::fs::read_to_string(&fix_md) {
        if content
            .lines()
            .any(|l| l.trim().starts_with("## Challenge"))
        {
            log_challenge(&fix_md, &ctx.pfile);
            let b = "builder disputed review findings — Lead resolves the Challenge above";
            return ctx.finish(state, "disputed", 0, Some(b));
        }
    }

    if blocked_outcome(&fix_md) {
        log_blockers(&fix_md, &ctx.pfile);
        let b = "builder's fix report is unfinished (see above) — Lead decides how to proceed";
        return ctx.finish(state, "blocked", 0, Some(b));
    }

    let prev_head = state.head_sha.clone();
    if let Some(done) = ctx.check_impl(state, &fix_md, Some(&prev_head))? {
        return Ok(done);
    }

    let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
    pcontent.push_str(&format!(
        "- fix {}: commit {} ({bname}.fix{}.md)\n",
        state.round, state.commit, state.round
    ));
    let _ = std::fs::write(&ctx.pfile, pcontent);

    state.phase = "review".to_string();
    state.round += 1;
    state.prompted = 0;
    state.reviewer_spawned = 0;
    let _ = state.save(&ctx.state_file);
    Ok(StepResult::Continue)
}
