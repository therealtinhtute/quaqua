//! Project Lead pane tagging and token healing.
//!
//! Mirrors `quaqua cos tag` (`crates/quaqua/src/cos.rs`) for project Leads.
//! Tags the target pane with `grid_role=orchestrator` and `grid_owner=<slug>-lead`,
//! healing pre-P3 Leads so they resolve positive Lead identity and attribute
//! spawned worker slots to the Lead cap rather than CoS.

use crate::herdr::HerdrOps;

pub fn tag_lead_pane<H: HerdrOps>(herdr: &H, slug: &str, pane: &str) -> Result<String, String> {
    let lead_name = if slug.ends_with("-lead") {
        slug.to_string()
    } else {
        format!("{slug}-lead")
    };
    let role_tok = "grid_role=orchestrator";
    let owner_tok = format!("grid_owner={lead_name}");
    let out = herdr
        .run(&[
            "pane",
            "report-metadata",
            pane,
            "--source",
            "herdr-quaqua",
            "--token",
            role_tok,
            "--token",
            &owner_tok,
        ])
        .map_err(|e| format!("herdr pane report-metadata failed for {pane}: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("failed to tag lead pane {pane}: {stderr}"));
    }
    Ok(format!(
        "Tagged pane {pane} as grid_role=orchestrator grid_owner={lead_name}"
    ))
}

pub fn ensure_lead_pane_tagged<H: HerdrOps>(
    herdr: &H,
    slug: &str,
    pane: &str,
) -> Result<(), String> {
    if pane.is_empty() || !pane.contains(':') {
        return Ok(());
    }
    if let Ok(out) = herdr.run(&["pane", "get", pane]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(tokens) = val.pointer("/result/pane/tokens") {
                    let has_role =
                        tokens.get("grid_role").and_then(|v| v.as_str()) == Some("orchestrator");
                    let has_owner = tokens
                        .get("grid_owner")
                        .and_then(|v| v.as_str())
                        .map(|s| s.ends_with("-lead"))
                        .unwrap_or(false);
                    if has_role && has_owner {
                        return Ok(());
                    }
                }
            }
        }
    }
    tag_lead_pane(herdr, slug, pane)?;
    Ok(())
}

pub fn handle_tag<H: HerdrOps>(args: &[String], herdr: &H) -> Result<(), (i32, String)> {
    if args.is_empty() {
        return Err((
            1,
            "Usage: quaqua lead tag <slug> [--pane <pane>] | --all\n".to_string(),
        ));
    }
    if args[0] == "--all" {
        if let Ok(out) = herdr.run(&["agent", "list"]) {
            if out.status.success() {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    if let Some(agents) = val.pointer("/result/agents").and_then(|a| a.as_array()) {
                        for agent in agents {
                            let title = agent
                                .get("terminal_title")
                                .and_then(|t| t.as_str())
                                .unwrap_or("");
                            let pane_id =
                                agent.get("pane_id").and_then(|p| p.as_str()).unwrap_or("");
                            let slug = if title.contains("LEAD: Lead mu") {
                                Some("mu")
                            } else if title.contains("LLMHUB: Lead llmhub") {
                                Some("llmhub")
                            } else {
                                None
                            };
                            if let (Some(s), p) = (slug, pane_id) {
                                if !p.is_empty() {
                                    let _ = tag_lead_pane(herdr, s, p);
                                }
                            }
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    let slug = &args[0];
    let explicit_pane = if args.len() >= 3 && args[1] == "--pane" {
        Some(args[2].as_str())
    } else {
        None
    };

    let pane = if let Some(p) = explicit_pane {
        p.to_string()
    } else if let Ok(p) = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or(())
    {
        p
    } else {
        let out = herdr
            .run(&["pane", "current"])
            .map_err(|e| (1, format!("{e}\n")))?;
        let val: serde_json::Value =
            serde_json::from_slice(&out.stdout).map_err(|e| (1, format!("{e}\n")))?;
        val.pointer("/result/pane/pane_id")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| (1, "cannot determine current pane\n".to_string()))?
    };

    tag_lead_pane(herdr, slug, &pane).map_err(|e| (1, format!("{e}\n")))?;
    println!("Tagged pane {pane} as grid_role=orchestrator (lead {slug})");
    Ok(())
}
