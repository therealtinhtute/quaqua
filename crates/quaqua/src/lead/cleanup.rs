//! Project Lead cleanup logic (`quaqua lead cleanup <slug>`).
//! Reproduces `scripts/lead/cleanup-lead.sh`.

use super::spawn::find_quaqua_root;
use crate::herdr::HerdrOps;
use crate::state::WorkerMetadata;
use serde_json::Value;
use std::path::PathBuf;

pub const USAGE: &str = "\
# Usage: cleanup-lead.sh <slug>
#   Tear down the project Lead <slug>-lead once the project is done. Refuses
#   while any of its Peers is still registered with herdr (runs/<peer>.owner
#   naming the Lead) — clean those up first with cleanup-worker.sh. Closes the
#   Lead's team workspace only when it is the one spawn-lead.sh made (label
#   <slug>) and not the caller's; a Lead living anywhere else (older spawns)
#   gets just its pane closed. Branch quaqua/<slug>-integ is kept; the
#   projects/INDEX.md owner cell returns to cos (serialized with spawn-lead.sh
#   via projects/INDEX.md.lock.d, published by atomic rename). A Peer or Lead
#   whose `herdr agent get` fails for any reason other than a definite
#   agent_not_found is of unknown liveness: cleanup aborts rather than close
#   a workspace it cannot prove is dead.";

fn is_valid_cleanup_slug(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn cleanup_lead<H: HerdrOps>(args: &[String], herdr: &H) -> Result<(), (i32, String)> {
    if args.len() != 1 {
        return Err((1, format!("{}\n", USAGE)));
    }
    let slug = &args[0];
    if !is_valid_cleanup_slug(slug) {
        return Err((1, format!("{}\n", USAGE)));
    }

    let root = find_quaqua_root()?;
    let runs_dir = std::env::var("QUAQUA_RUNS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("runs"));
    let name = format!("{slug}-lead");

    // Check Lead liveness
    let (lead_ok, lead_body) = herdr.agent_get(&name).unwrap_or((false, String::new()));
    let lead_json = serde_json::from_str::<Value>(&lead_body).ok();
    let has_agent = lead_ok && lead_json.as_ref().and_then(|j| j.get("result")).and_then(|r| r.get("agent")).is_some();

    if !has_agent {
        let is_not_found = lead_body.contains("agent_not_found")
            || lead_json.as_ref()
                .and_then(|j| j.get("error"))
                .and_then(|e| e.get("code"))
                .and_then(|c| c.as_str())
                == Some("agent_not_found");
        if is_not_found {
            return Err((1, format!("no live Lead '{name}'\n")));
        } else {
            return Err((
                1,
                format!(
                    "cannot determine the state of Lead '{name}' (herdr agent get failed) — refusing to clean up\n"
                ),
            ));
        }
    }

    let agent_obj = lead_json.as_ref().and_then(|j| j.get("result")).and_then(|r| r.get("agent")).unwrap();
    let pane = agent_obj.get("pane_id").and_then(|p| p.as_str()).unwrap_or("");
    let ws = agent_obj
        .get("workspace_id")
        .and_then(|w| w.as_str())
        .unwrap_or_else(|| {
            if let Some(colon) = pane.find(':') {
                &pane[..colon]
            } else {
                pane
            }
        });

    // Check live Peers
    let mut live_peers = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("owner") {
                let peer = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !peer.is_empty() && WorkerMetadata::load_owner(&runs_dir, peer) == name {
                    let (peer_ok, peer_body) = herdr.agent_get(peer).unwrap_or((false, String::new()));
                    let peer_json = serde_json::from_str::<Value>(&peer_body).ok();
                    let peer_live = peer_ok
                        && peer_json.as_ref()
                            .and_then(|j| j.get("result"))
                            .and_then(|r| r.get("agent"))
                            .is_some();
                    if peer_live {
                        live_peers.push(peer.to_string());
                    } else {
                        let is_peer_not_found = peer_body.contains("agent_not_found")
                            || peer_json.as_ref()
                                .and_then(|j| j.get("error"))
                                .and_then(|e| e.get("code"))
                                .and_then(|c| c.as_str())
                                == Some("agent_not_found");
                                if !is_peer_not_found {
                                    return Err((
                                        1,
                                        format!(
                                            "cannot determine whether Peer '{peer}' is still live (herdr agent get failed) — refusing to clean up '{name}'; retry, or run cleanup-worker.sh if it is gone\n"
                                        ),
                                    ));
                                }
                            }
                        }
            }
        }
    }

    if !live_peers.is_empty() {
        return Err((
            1,
            format!(
                "{name} still owns live Peers: {} — run cleanup-worker.sh first\n",
                live_peers.join(" ")
            ),
        ));
    }

    let caller_pane = match std::env::var("HERDR_PANE_ID") {
        Ok(p) => p,
        Err(_) => {
            let out = herdr
                .run(&["pane", "current"])
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let cur_json = serde_json::from_str::<Value>(&out).ok();
            cur_json
                .as_ref()
                .and_then(|j| j.get("result"))
                .and_then(|r| r.get("pane"))
                .and_then(|p| p.get("pane_id"))
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string()
        }
    };
    let caller_ws = caller_pane.split(':').next().unwrap_or("");

    let ws_out = herdr
        .run(&["workspace", "get", ws])
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let ws_parsed = serde_json::from_str::<Value>(&ws_out).ok();
    let ws_label = ws_parsed
        .as_ref()
        .and_then(|j| j.get("result"))
        .and_then(|r| r.get("workspace"))
        .and_then(|w| w.get("label"))
        .and_then(|w| w.as_str())
        .unwrap_or("");
    let closed = if ws_label == slug && ws != caller_ws {
        let _ = herdr.run(&["workspace", "close", ws]);
        format!("workspace {ws}")
    } else {
        let _ = herdr.run(&["pane", "close", pane]);
        format!("pane {pane}")
    };

    let rf = runs_dir.join(format!("{name}.rules.md"));
    let _ = std::fs::remove_file(rf);

    let index = root.join("projects/INDEX.md");
    if index.is_file() {
        crate::lead::spawn::update_project_index_owner(&root, slug, "cos")?;
    }

    println!("cleaned {name} ({closed} closed; branch quaqua/{slug}-integ kept)");
    Ok(())
}
