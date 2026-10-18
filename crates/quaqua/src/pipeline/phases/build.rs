//! Build phase implementation.

use super::{blocked_outcome, log_blockers, log_challenge, PipelineContext, StepResult};
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;
use std::collections::BTreeSet;
use std::path::Path;

pub fn run_build<G: GitOps, H: HerdrOps>(
    state: &mut PipelineState,
    ctx: &PipelineContext<'_, G, H>,
) -> Result<StepResult, (i32, String)> {
    let bname = format!("{}-{}", state.builder, ctx.task);
    let build_prompt = format!(
        "{}\n\nPipeline phase: BUILD — your plan is approved ({}). Implement it in your worktree, commit on quaqua/{bname}, run the tests. If the premise or the plan proves wrong, stop and write '## Challenge' with evidence instead of forcing it. If the work turns out to need no changes at all, say 'Outcome: no changes' in your runs file.",
        ctx.brief_text, state.planfile
    );

    ctx.wait_file(state, &bname, &build_prompt, ctx.build_budget)?;

    let build_md = ctx.runs_dir.join(format!("{bname}.build.md"));
    let cur_md = ctx.runs_dir.join(format!("{bname}.md"));
    let _ = std::fs::rename(&cur_md, &build_md);

    if let Ok(content) = std::fs::read_to_string(&build_md) {
        if content
            .lines()
            .any(|l| l.trim().starts_with("## Challenge"))
        {
            log_challenge(&build_md, &ctx.pfile);
            let b = "builder challenged the premise during build — Lead resolves the Challenge above";
            return ctx.finish(state, "disputed", 0, Some(b));
        }
    }

    if blocked_outcome(&build_md) {
        log_blockers(&build_md, &ctx.pfile);
        let b = "builder's build report is unfinished (see above) — Lead decides how to proceed";
        return ctx.finish(state, "blocked", 0, Some(b));
    }

    if let Some(done) = ctx.check_impl(state, &build_md, None)? {
        return Ok(done);
    }

    let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
    pcontent.push_str(&format!(
        "- build: commit {} ({bname}.build.md)\n",
        state.commit
    ));

    let wt_path = Path::new(&state.wt);
    let diff_target = if state.base_sha.is_empty() {
        "HEAD~1"
    } else {
        &state.base_sha
    };
    let diff_out = ctx.git.run_in(wt_path, &["diff", "--stat", diff_target, "HEAD"]);
    let stat_str = diff_out
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let diff_files = ctx.git.run_in(wt_path, &["diff", "--name-only", diff_target, "HEAD"]);
    let files_str = diff_files
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let mut top_dirs = BTreeSet::new();
    for f in files_str.lines() {
        if let Some(idx) = f.find('/') {
            top_dirs.insert(&f[..idx]);
        }
    }

    let mut changed_lines = 0;
    for part in stat_str.split_whitespace() {
        if part.ends_with('+') || part.ends_with('-') {
            continue;
        }
        if let Ok(n) = part.parse::<usize>() {
            changed_lines = n;
        }
    }

    let run_review = match state.review.as_str() {
        "always" => true,
        "skip" => false,
        _ => changed_lines > ctx.review_lines || top_dirs.len() > 1,
    };

    if !run_review {
        let reason = if state.review == "skip" {
            "--review skip"
        } else {
            "small single-area change"
        };
        pcontent.push_str(&format!("- review skipped ({reason})\n"));
        let _ = std::fs::write(&ctx.pfile, pcontent);
        return ctx.finish(state, "clean", 0, None);
    }

    let _ = std::fs::write(&ctx.pfile, pcontent);
    state.phase = "review".to_string();
    state.round = 1;
    state.prompted = 0;
    state.reviewer_spawned = 0;
    let _ = state.save(&ctx.state_file);
    Ok(StepResult::Continue)
}
