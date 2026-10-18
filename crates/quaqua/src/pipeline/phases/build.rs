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
    match state.review.as_str() {
        "always" => state.review_run = "run".to_string(),
        "skip" => state.review_run = "skip".to_string(),
        _ => {
            // auto
            let numstat_out = ctx.git.run_in(
                wt_path,
                &["diff", "--numstat", &state.base_sha, &state.head_sha],
            );
            let mut total_lines = 0usize;
            if let Ok(o) = numstat_out {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let add: usize = parts[0].parse().unwrap_or(0);
                        let del: usize = parts[1].parse().unwrap_or(0);
                        total_lines += add + del;
                    }
                }
            }

            let names_out = ctx.git.run_in(
                wt_path,
                &["diff", "--name-only", &state.base_sha, &state.head_sha],
            );
            let mut dirs: BTreeSet<String> = BTreeSet::new();
            if let Ok(o) = names_out {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let path = line.trim();
                    if let Some(idx) = path.find('/') {
                        dirs.insert(path[..idx].to_string());
                    } else if !path.is_empty() {
                        dirs.insert(".".to_string());
                    }
                }
            }

            let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
            pcontent.push_str(&format!(
                "- review gate: {total_lines} changed lines, {} top-level dirs (run if >{} or >1)\n",
                dirs.len(),
                ctx.review_lines
            ));
            let _ = std::fs::write(&ctx.pfile, pcontent);

            if total_lines > ctx.review_lines || dirs.len() > 1 {
                state.review_run = "run".to_string();
            } else {
                state.review_run = "skip".to_string();
            }
        }
    }

    if state.review_run == "skip" {
        let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
        pcontent.push_str(&format!("- review: skipped (gate: {})\n", state.review));
        let _ = std::fs::write(&ctx.pfile, pcontent);
        return ctx.finish(state, "clean", 0, None);
    }

    state.phase = "review".to_string();
    state.round = 1;
    state.prompted = 0;
    state.reviewer_spawned = 0;
    let _ = state.save(&ctx.state_file);
    Ok(StepResult::Continue)
}
