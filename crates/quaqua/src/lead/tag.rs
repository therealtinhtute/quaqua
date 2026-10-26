//! Project Lead pane tagging and token healing.
//!
//! Mirrors `quaqua cos tag` (`crates/quaqua/src/cos.rs`) for project Leads.
//! Tags the target pane with `grid_role=orchestrator` and `grid_owner=<slug>-lead`,
//! healing pre-P3 Leads so they resolve positive Lead identity and attribute
//! spawned worker slots to the Lead cap rather than CoS.

use crate::herdr::HerdrOps;

pub fn get_agent_list_json<H: HerdrOps>(herdr: &H) -> Option<serde_json::Value> {
    if let Ok(out) = herdr.agent_list() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&out) {
            return Some(val);
        }
    }
    if let Ok(out) = herdr.run(&["agent", "list"]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                return Some(val);
            }
        }
    }
    None
}

pub fn tag_lead_pane<H: HerdrOps>(herdr: &H, slug: &str, pane: &str) -> Result<String, String> {
    let clean_slug = slug.strip_suffix("-lead").unwrap_or(slug);
    let lead_name = format!("{clean_slug}-lead");
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
    let clean_slug = slug.strip_suffix("-lead").unwrap_or(slug);
    let expected_owner = format!("{clean_slug}-lead");
    if let Ok(out) = herdr.run(&["pane", "get", pane]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(tokens) = val.pointer("/result/pane/tokens") {
                    let has_role =
                        tokens.get("grid_role").and_then(|v| v.as_str()) == Some("orchestrator");
                    let has_owner = tokens.get("grid_owner").and_then(|v| v.as_str())
                        == Some(expected_owner.as_str());
                    if has_role && has_owner {
                        return Ok(());
                    }
                }
            }
        }
    }
    tag_lead_pane(herdr, clean_slug, pane)?;
    Ok(())
}

pub fn repair_lead<H: HerdrOps>(
    herdr: &H,
    slug: &str,
    explicit_pane: Option<&str>,
) -> Result<String, String> {
    let clean_slug = slug.strip_suffix("-lead").unwrap_or(slug);
    let lead_name = format!("{clean_slug}-lead");

    // Check projects/INDEX.md registration if a workspace can be resolved
    if let Ok(root) = crate::workspace::workspace_root_checked() {
        let index_file = root.join("projects/INDEX.md");
        if let Ok(content) = std::fs::read_to_string(&index_file) {
            let rows = crate::worker::leadgate::index_rows(&content);
            if let Some(row) = rows.iter().find(|r| r.slug == clean_slug) {
                if row.owner != lead_name {
                    return Err(format!(
                        "project '{clean_slug}' owner in projects/INDEX.md is '{}', not '{lead_name}'",
                        row.owner
                    ));
                }
            } else {
                return Err(format!(
                    "unknown project slug '{clean_slug}' — no row in {}",
                    index_file.display()
                ));
            }
        }
    }

    let pane = if let Some(p) = explicit_pane.filter(|s| !s.trim().is_empty()) {
        p.to_string()
    } else {
        // Look up agent in herdr agent list
        let agents_val = get_agent_list_json(herdr)
            .ok_or_else(|| "cannot query herdr agent list".to_string())?;
        let agents = agents_val
            .pointer("/result/agents")
            .and_then(|a| a.as_array())
            .ok_or_else(|| "no agents array in herdr agent list response".to_string())?;
        let mut matched_pane = None;
        for agent in agents {
            let name = agent.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == lead_name {
                if let Some(p) = agent.get("pane_id").and_then(|p| p.as_str()) {
                    if !p.is_empty() {
                        matched_pane = Some(p.to_string());
                        break;
                    }
                }
            }
        }
        matched_pane
            .ok_or_else(|| format!("agent '{lead_name}' is not registered or running in herdr"))?
    };

    // Check if pane is already correctly tagged
    if let Ok(out) = herdr.run(&["pane", "get", &pane]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(tokens) = val.pointer("/result/pane/tokens") {
                    let has_role =
                        tokens.get("grid_role").and_then(|v| v.as_str()) == Some("orchestrator");
                    let has_owner =
                        tokens.get("grid_owner").and_then(|v| v.as_str()) == Some(&lead_name);
                    if has_role && has_owner {
                        return Ok(format!(
                            "lead '{clean_slug}' (pane {pane}) already tagged as grid_role=orchestrator grid_owner={lead_name}"
                        ));
                    }
                }
            }
        }
    }

    tag_lead_pane(herdr, clean_slug, &pane)?;
    Ok(format!(
        "repaired lead '{clean_slug}' (pane {pane}): tagged grid_role=orchestrator grid_owner={lead_name}"
    ))
}

pub fn repair_all_leads<H: HerdrOps>(herdr: &H) -> Result<Vec<String>, String> {
    let root = crate::workspace::workspace_root_checked()
        .map_err(|(_, e)| format!("cannot resolve workspace root: {e}"))?;
    let index_file = root.join("projects/INDEX.md");
    let content = std::fs::read_to_string(&index_file)
        .map_err(|e| format!("cannot read {}: {e}", index_file.display()))?;
    let rows = crate::worker::leadgate::index_rows(&content);
    let lead_rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r.owner.ends_with("-lead"))
        .collect();

    if lead_rows.is_empty() {
        return Ok(vec![
            "no registered Leads found in projects/INDEX.md".to_string()
        ]);
    }

    let mut results = Vec::new();
    for row in lead_rows {
        match repair_lead(herdr, &row.slug, None) {
            Ok(msg) => results.push(msg),
            Err(err) => results.push(format!("lead '{}': {err}", row.slug)),
        }
    }
    Ok(results)
}

pub fn handle_repair<H: HerdrOps>(args: &[String], herdr: &H) -> Result<(), (i32, String)> {
    if args.is_empty() {
        return Err((
            1,
            "Usage: quaqua lead repair <slug> [--pane <pane>] | --all\n".to_string(),
        ));
    }
    if args[0] == "--all" {
        let results = repair_all_leads(herdr).map_err(|e| (1, format!("{e}\n")))?;
        for line in results {
            println!("{line}");
        }
        return Ok(());
    }

    let slug = &args[0];
    let explicit_pane = if args.len() >= 3 && args[1] == "--pane" {
        Some(args[2].as_str())
    } else {
        None
    };

    let msg = repair_lead(herdr, slug, explicit_pane).map_err(|e| (1, format!("{e}\n")))?;
    println!("{msg}");
    Ok(())
}

pub fn handle_tag<H: HerdrOps>(args: &[String], herdr: &H) -> Result<(), (i32, String)> {
    handle_repair(args, herdr)
}
