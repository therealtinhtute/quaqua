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

pub fn tag_lead_pane<H: HerdrOps>(
    herdr: &H,
    slug: &str,
    pane: &str,
    farm: &str,
) -> Result<String, String> {
    let clean_slug = slug.strip_suffix("-lead").unwrap_or(slug);
    let lead_name = format!("{clean_slug}-lead");
    let farm_tok = format!("grid_farm={farm}");
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
            "--token",
            &farm_tok,
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
    farm: &str,
) -> Result<bool, String> {
    if pane.is_empty() || !pane.contains(':') {
        return Ok(false);
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
                    let has_farm = tokens.get("grid_farm").and_then(|v| v.as_str()) == Some(farm);
                    if has_role && has_owner && has_farm {
                        return Ok(false);
                    }
                }
            }
        }
    }
    tag_lead_pane(herdr, clean_slug, pane, farm)?;
    Ok(true)
}

/// Repair a registered Lead's pane tokens.
/// Verifies consistency between herdr agent registry and projects/INDEX.md registration.
pub fn repair_lead<H: HerdrOps>(
    herdr: &H,
    slug: &str,
    explicit_pane: Option<&str>,
) -> Result<String, String> {
    let clean_slug = slug.strip_suffix("-lead").unwrap_or(slug);
    let lead_name = format!("{clean_slug}-lead");

    // Fail closed: projects/INDEX.md must be resolvable and register this lead
    let root = crate::workspace::workspace_root_checked()
        .map_err(|(_, e)| format!("cannot resolve workspace root: {e}"))?;
    let index_file = root.join("projects/INDEX.md");
    let content = std::fs::read_to_string(&index_file)
        .map_err(|e| format!("cannot read {}: {e}", index_file.display()))?;
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

    // Look up agent in herdr agent list
    let agents_val =
        get_agent_list_json(herdr).ok_or_else(|| "cannot query herdr agent list".to_string())?;
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
    let agent_pane = matched_pane
        .ok_or_else(|| format!("agent '{lead_name}' is not registered or running in herdr"))?;

    // Verify explicit pane matches the agent's actual pane
    if let Some(p) = explicit_pane.filter(|s| !s.trim().is_empty()) {
        if p != agent_pane {
            return Err(format!(
                "agent '{lead_name}' is running on pane '{agent_pane}', not requested pane '{p}'"
            ));
        }
    }
    let pane = agent_pane;

    // `runs/<lead>.owner` is the parent link `identity::depth_walk` follows, and
    // only `lead spawn` writes it. A Lead started before that write existed has
    // none, so every spawn/mail from it is denied with `depth.owner_missing`.
    // Restore it here, never overwriting an owner already recorded.
    let runs_dir = root.join("runs");
    let farm = crate::workspace::farm_of_runs_dir(&runs_dir);

    // Read the pane's tag state before restoring the owner: resolving the caller can
    // itself self-heal this pane (an untagged caller pane triggers `identity::heal`),
    // and repair must report what it found, not what that side effect produced.
    let mut already_tagged = false;
    if let Ok(out) = herdr.run(&["pane", "get", &pane]) {
        if out.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(tokens) = val.pointer("/result/pane/tokens") {
                    let has_role =
                        tokens.get("grid_role").and_then(|v| v.as_str()) == Some("orchestrator");
                    let has_owner =
                        tokens.get("grid_owner").and_then(|v| v.as_str()) == Some(&lead_name);
                    let has_farm =
                        tokens.get("grid_farm").and_then(|v| v.as_str()) == Some(farm.as_str());
                    already_tagged = has_role && has_owner && has_farm;
                }
            }
        }
    }

    let owner_path = crate::state::WorkerMetadata::owner_path(&runs_dir, &lead_name);
    let owner_note = if owner_path.is_file() {
        String::new()
    } else {
        let owner = crate::lead::spawn::owner_from_caller(&crate::identity::resolve_caller(
            herdr, &runs_dir,
        ));
        if owner == lead_name {
            return Err(format!(
                "cannot restore {lead_name}.owner from the Lead's own pane: '{owner}' would own itself (depth.cycle) — run `quaqua lead repair {clean_slug}` from the CoS pane"
            ));
        }
        crate::state::WorkerMetadata::save_owner(&runs_dir, &lead_name, &owner)
            .map_err(|e| format!("cannot write {}: {e}", owner_path.display()))?;
        format!("; restored {lead_name}.owner={owner}")
    };

    if already_tagged {
        return Ok(format!(
            "lead '{clean_slug}' (pane {pane}) already tagged as grid_role=orchestrator grid_owner={lead_name} grid_farm={farm}{owner_note}"
        ));
    }

    tag_lead_pane(herdr, clean_slug, &pane, &farm)?;
    Ok(format!(
        "repaired lead '{clean_slug}' (pane {pane}): tagged grid_role=orchestrator grid_owner={lead_name} grid_farm={farm}{owner_note}"
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
        .filter(|r| r.owner == format!("{}-lead", r.slug))
        .collect();

    if lead_rows.is_empty() {
        return Ok(vec![
            "no registered Leads found in projects/INDEX.md".to_string()
        ]);
    }

    let mut results = Vec::new();
    let mut failures = 0;
    for row in lead_rows {
        match repair_lead(herdr, &row.slug, None) {
            Ok(msg) => results.push(msg),
            Err(err) => {
                results.push(format!("lead '{}': {err}", row.slug));
                failures += 1;
            }
        }
    }
    if failures > 0 {
        return Err(format!(
            "repair --all failed for {failures} lead(s):\n{}",
            results.join("\n")
        ));
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
