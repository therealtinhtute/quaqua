//! Review phase implementation.

use super::{log_challenge, PipelineContext, StepResult};
use crate::git::GitOps;
use crate::herdr::HerdrOps;
use crate::pipeline::state::PipelineState;
use std::path::Path;

pub fn run_review<G: GitOps, H: HerdrOps>(
    state: &mut PipelineState,
    ctx: &PipelineContext<'_, G, H>,
) -> Result<StepResult, (i32, String)> {
    let bname = format!("{}-{}", state.builder, ctx.task);
    let revname = format!("{}-{}r{}", state.reviewer, ctx.task, state.round);

    if state.reviewer_spawned == 0 {
        let rev_report = ctx.runs_dir.join(format!("{revname}.md"));
        if rev_report.is_file() {
            ctx.archive_stale(&rev_report);
        }

        let sargs = vec![
            "--review".to_string(),
            "--base".to_string(),
            state.head_sha.clone(),
            revname.clone(),
            state.reviewer.clone(),
            state.repo.clone(),
        ];

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
            let mut opts = crate::worker::spawn::parse_spawn_args(&sargs)?;
            opts.approve_kinds = state.approve_kinds.clone();
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
                "worker spawn failed for reviewer {revname} — fix the cause, then 'quaqua pipeline --resume {} {}'",
                ctx.task, state.repo
            );
            return ctx.finish(state, "blocked", 3, Some(&b));
        }
        state.reviewer_spawned = 1;
        let _ = state.save(&ctx.state_file);

        let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
        pcontent.push_str(&format!(
            "- review round {}: {revname} spawned on {} (quaqua/{bname} tip)\n",
            state.round, state.head_sha
        ));
        let _ = std::fs::write(&ctx.pfile, pcontent);
    }

    if !ctx.run_grid_hook() {
        let b = format!(
            "grid check failed for {revname} — arrange the panes per rules/_all/fanout-agent-grid.md, then 'quaqua pipeline --resume {} {}'",
            ctx.task, state.repo
        );
        let _ = state.save(&ctx.state_file);
        return ctx.finish(state, "blocked", 5, Some(&b));
    }

    let review_prompt = format!(
        "Goal: review the diff for task '{}' and report findings with file:line evidence\nHard constraints: read-only — do not edit or create files outside ~/quaqua/runs; every finding must cite file:line in the diff.\nCurrent design (open to challenge): builder {bname} implemented the task on branch quaqua/{bname} — your worktree is on its tip; run 'git diff {}' for the full change. Original brief: {} (readable).\nUnknowns: whether the change meets the brief — verify correctness, missing tests, and scope creep against the diff, not the report.\nEvidence: a findings list, then the runs file MUST end with exactly one line: 'Verdict: clean' or 'Verdict: <N> findings'. Write to a temp file and rename it to ~/quaqua/runs/{revname}.md — publish atomically; reply only its path.\nPipeline phase: REVIEW (round {} of {})",
        ctx.task, state.base_sha, state.brief, state.round, state.rounds
    );

    let rev_report = ctx.runs_dir.join(format!("{revname}.md"));
    let rev_already_delivered = rev_report.is_file() && {
        let content = std::fs::read_to_string(&rev_report).unwrap_or_default();
        content.lines().any(|l| {
            l.trim_start()
                .to_ascii_lowercase()
                .starts_with("verdict:")
                || l.trim().starts_with("## Challenge")
        })
    };

    if !rev_already_delivered {
        ctx.wait_file(state, &revname, &review_prompt, ctx.review_budget)?;
    }
    state.rounds_run = state.round;

    let rev_content = std::fs::read_to_string(&rev_report).unwrap_or_default();

    let cleanup_reviewer = || {
        let prev_pt = std::env::var("PIPELINE_TASK").ok();
        let prev_gs = std::env::var("GRID_SKIP").ok();
        std::env::set_var("PIPELINE_TASK", &ctx.task);
        std::env::set_var("GRID_SKIP", "1");
        let _ = crate::worker::cleanup::cleanup_worker(
            &revname,
            Path::new(&state.repo),
            ctx.git,
            ctx.herdr,
        );
        if let Some(pt) = prev_pt {
            std::env::set_var("PIPELINE_TASK", pt);
        } else {
            std::env::remove_var("PIPELINE_TASK");
        }
        if let Some(gs) = prev_gs {
            std::env::set_var("GRID_SKIP", gs);
        } else {
            std::env::remove_var("GRID_SKIP");
        }
        let _ = ctx.run_grid_hook();
    };

    if rev_content
        .lines()
        .any(|l| l.trim().starts_with("## Challenge"))
    {
        log_challenge(&rev_report, &ctx.pfile);
        let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
        pcontent.push_str(&format!(
            "- review round {}: {revname} challenged the premise\n",
            state.round
        ));
        let _ = std::fs::write(&ctx.pfile, pcontent);
        cleanup_reviewer();
        let b = format!("reviewer {revname} challenged the premise — Lead resolves the Challenge above");
        return ctx.finish(state, "disputed", 0, Some(&b));
    }

    let mut verdict_line: Option<String> = None;
    for line in rev_content.lines() {
        let t = line.trim();
        if t.starts_with("Verdict:") || t.starts_with("verdict:") {
            verdict_line = Some(t.to_string());
        }
    }

    let mut pcontent = std::fs::read_to_string(&ctx.pfile).unwrap_or_default();
    pcontent.push_str(&format!(
        "- review round {}: {} ({revname}.md)\n",
        state.round,
        verdict_line.as_deref().unwrap_or("NO VERDICT LINE")
    ));
    let _ = std::fs::write(&ctx.pfile, pcontent);

    cleanup_reviewer();

    let v = match verdict_line {
        Some(vl) => vl,
        None => {
            let b = format!("reviewer {revname} runs file has no 'Verdict: clean | <N> findings' line (lock §1) — Lead decides");
            return ctx.finish(state, "blocked", 0, Some(&b));
        }
    };

    let v_lower = v.to_ascii_lowercase();
    if v_lower.contains("clean") {
        state.last_findings = 0;
    } else {
        let mut found_count = false;
        for part in v.split_whitespace() {
            if let Ok(n) = part.parse::<u32>() {
                state.last_findings = n;
                found_count = true;
                break;
            }
        }
        if !found_count {
            let b = format!("reviewer {revname} runs file has no 'Verdict: clean | <N> findings' line (lock §1) — Lead decides");
            return ctx.finish(state, "blocked", 0, Some(&b));
        }
    }

    if state.last_findings == 0 {
        return ctx.finish(state, "clean", 0, None);
    }

    state.review_changed = true;
    if state.round >= state.rounds {
        let b = format!(
            "review rounds exhausted: {} findings still open after {} round(s) — Lead decides",
            state.last_findings, state.rounds
        );
        return ctx.finish(state, "blocked", 0, Some(&b));
    }

    state.phase = "fix".to_string();
    state.prompted = 0;
    let _ = state.save(&ctx.state_file);
    Ok(StepResult::Continue)
}
