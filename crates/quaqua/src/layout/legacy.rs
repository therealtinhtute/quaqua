use crate::config::{Config, DEFAULT_LEAD_MAX_PEERS, ROLE_NAMES};
use crate::layout::model::{Layout, LayoutRole, Naming, RoleClass};
use std::collections::BTreeMap;

impl Layout {
    /// Synthesize an SLP layout from a legacy v2 Config.
    ///
    /// The mapping covers:
    /// - `can_spawn` and `can_prompt`, copied as they are;
    /// - `reports_to`;
    /// - `[limits].depth`;
    /// - `roles.lead.max_peers` -> `lead.max_children`;
    /// - `limits.max_workers_per_orchestrator` -> `cos.max_children`.
    pub fn from_legacy(cfg: &Config) -> Self {
        let mut roles = BTreeMap::new();
        for name in &ROLE_NAMES {
            let name_str = name.to_string();
            let role_cfg = cfg.roles.get(&name_str);
            let class = match *name {
                "cos" => RoleClass::Cos,
                "lead" => RoleClass::Lead,
                "peer" => RoleClass::Peer,
                _ => RoleClass::Peer,
            };
            let can_spawn = role_cfg.map(|r| r.can_spawn.clone()).unwrap_or_default();
            let can_prompt = role_cfg.map(|r| r.can_prompt.clone()).unwrap_or_default();
            let reports_to = role_cfg.and_then(|r| r.reports_to.clone());
            let max_children = match *name {
                "lead" => {
                    let peers = role_cfg.and_then(|r| r.max_peers).unwrap_or(DEFAULT_LEAD_MAX_PEERS);
                    Some(peers)
                }
                "cos" => Some(cfg.limits.max_workers_per_orchestrator),
                _ => None,
            };
            let naming = match *name {
                "lead" => Naming::SlugLead,
                _ => Naming::KindTask,
            };
            roles.insert(
                name_str,
                LayoutRole {
                    class,
                    can_spawn,
                    can_prompt,
                    reports_to,
                    max_children,
                    naming,
                    members: Vec::new(),
                },
            );
        }

        Self {
            name: "slp".to_string(),
            depth: cfg.limits.depth.max(0) as u32,
            roles,
        }
    }
}
