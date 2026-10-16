//! `team.toml` typed model, project-override resolution, validation and key lookup.
//!
//! The shape mirrors docs/research/herdr-grid-rust-migration-plan.md § `team.toml`.
//! Resolution order is project override -> defaults, then validate. A validation
//! failure is rejected with one clear message (first failure wins).

use crate::models::{parse_models_md, ModelsKind};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 2;
/// AGENTS.md `## Rules`: "Max 4 task workers per orchestrator".
pub const CAP_CEILING: i64 = 4;
/// `can_spawn` / `can_prompt` vocabulary (role classes).
pub const ROLE_CLASSES: [&str; 4] = ["cos", "lead", "peer", "taskforce"];
/// `[roles.<r>]` table keys.
pub const ROLE_NAMES: [&str; 3] = ["cos", "lead", "peer"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub timers: Timers,
    #[serde(default)]
    pub roles: BTreeMap<String, Role>,
    #[serde(default)]
    pub strategy: Strategy,
    #[serde(default)]
    pub kinds: BTreeMap<String, Kind>,
    #[serde(default)]
    pub taskforce: BTreeMap<String, Taskforce>,
    #[serde(default)]
    pub merge: Merge,
    #[serde(default)]
    pub grid: Grid,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema_version: SCHEMA_VERSION,
            limits: Limits::default(),
            timers: Timers::default(),
            roles: BTreeMap::new(),
            strategy: Strategy::default(),
            kinds: BTreeMap::new(),
            taskforce: BTreeMap::new(),
            merge: Merge::default(),
            grid: Grid::default(),
            projects: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_depth")]
    pub depth: i64,
    pub max_workers_per_orchestrator: i64,
    #[serde(default)]
    pub weights: BTreeMap<String, i64>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            depth: default_depth(),
            max_workers_per_orchestrator: CAP_CEILING,
            weights: BTreeMap::new(),
        }
    }
}

fn default_depth() -> i64 {
    2
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Timers {
    pub budget_default: String,
    pub hard_cap: String,
    pub stall: String,
    pub idle: String,
    pub nudge_at: f64,
    pub grace: f64,
    pub pilot_budget: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Role {
    #[serde(default)]
    pub can_spawn: Vec<String>,
    #[serde(default)]
    pub can_prompt: Vec<String>,
    #[serde(default)]
    pub reports_to: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strategy {
    pub default_kind: String,
    #[serde(default)]
    pub default_reviewer: Vec<String>,
    #[serde(default = "default_differ")]
    pub builder_reviewer_differ: String,
}

impl Default for Strategy {
    fn default() -> Self {
        Strategy {
            default_kind: String::new(),
            default_reviewer: Vec::new(),
            builder_reviewer_differ: default_differ(),
        }
    }
}

fn default_differ() -> String {
    "harness".to_string()
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Kind {
    pub harness: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub can_review: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Taskforce {
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    pub integ_branch: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub workers_tab: String,
    #[serde(default)]
    pub deferred_statuses: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectOverride {
    pub max_workers_per_orchestrator: Option<i64>,
}

/// Read and parse a team.toml. Parse errors name the file.
pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Apply a project override over the defaults. Unknown project is a hard error.
pub fn resolve(mut cfg: Config, project: Option<&str>) -> Result<Config, String> {
    if let Some(slug) = project {
        let ov = cfg
            .projects
            .get(slug)
            .ok_or_else(|| format!("unknown project '{slug}'"))?
            .clone();
        if let Some(cap) = ov.max_workers_per_orchestrator {
            cfg.limits.max_workers_per_orchestrator = cap;
        }
    }
    Ok(cfg)
}

/// Validate the resolved config. `models` is the models.md routing table when the
/// file is available; `None` skips the model-drift rule (a project repo may not
/// ship models.md).
pub fn validate(cfg: &Config, models: Option<&BTreeMap<String, ModelsKind>>) -> Result<(), String> {
    if cfg.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} (want {SCHEMA_VERSION})",
            cfg.schema_version
        ));
    }

    let cap = cfg.limits.max_workers_per_orchestrator;
    if cap < 1 || cap > CAP_CEILING {
        return Err(format!("workers over the cap: {cap} (max {CAP_CEILING})"));
    }
    for (name, w) in &cfg.limits.weights {
        if *w < 1 || *w > cap {
            return Err(format!("weight {name}={w} exceeds cap {cap}"));
        }
    }
    for (slug, ov) in &cfg.projects {
        if let Some(p) = ov.max_workers_per_orchestrator {
            if p < 1 {
                return Err(format!("project {slug} cap {p} is below 1"));
            }
            if p > cap {
                return Err(format!("project {slug} cap {p} exceeds global cap {cap}"));
            }
        }
    }

    for (name, role) in &cfg.roles {
        if !ROLE_NAMES.contains(&name.as_str()) {
            return Err(format!("unknown role '{name}'"));
        }
        for cls in role.can_spawn.iter().chain(role.can_prompt.iter()) {
            if !ROLE_CLASSES.contains(&cls.as_str()) {
                return Err(format!("unknown role-class '{cls}'"));
            }
        }
        if let Some(to) = &role.reports_to {
            let known = ROLE_NAMES.contains(&to.as_str()) || to == "owner";
            if !known {
                return Err(format!("unknown role in reports_to '{to}'"));
            }
        }
    }

    if !["none", "harness", "kind"].contains(&cfg.strategy.builder_reviewer_differ.as_str()) {
        return Err(format!(
            "unknown differ policy '{}'",
            cfg.strategy.builder_reviewer_differ
        ));
    }

    if !cfg.kinds.contains_key(&cfg.strategy.default_kind) {
        return Err(format!("unknown kind '{}'", cfg.strategy.default_kind));
    }
    if cfg.strategy.default_reviewer.is_empty() {
        return Err("strategy.default_reviewer is empty: no reviewer to fall back to".to_string());
    }
    for r in &cfg.strategy.default_reviewer {
        if !cfg.kinds.contains_key(r) {
            return Err(format!("unknown kind '{r}'"));
        }
    }
    for (name, tf) in &cfg.taskforce {
        for m in &tf.members {
            if !cfg.kinds.contains_key(m) {
                return Err(format!("taskforce {name}: unknown kind '{m}'"));
            }
        }
    }

    // builder/reviewer harness rule (strategy.builder_reviewer_differ = "harness")
    if cfg.strategy.builder_reviewer_differ == "harness" {
        let b = &cfg.kinds[&cfg.strategy.default_kind];
        for r in &cfg.strategy.default_reviewer {
            let rk = &cfg.kinds[r];
            if rk.harness == b.harness {
                return Err(format!(
                    "builder '{}' and reviewer '{}' share harness '{}'",
                    cfg.strategy.default_kind, r, b.harness
                ));
            }
        }
    }

    for r in &cfg.strategy.default_reviewer {
        if !cfg.kinds[r].can_review {
            return Err(format!("reviewer '{r}' has can_review = false"));
        }
    }

    if !cfg.merge.integ_branch.contains("{slug}") {
        return Err(format!(
            "integ_branch '{}' needs a {{slug}} placeholder",
            cfg.merge.integ_branch
        ));
    }

    if let Some(models) = models {
        for (name, kind) in &cfg.kinds {
            match models.get(name) {
                None => return Err(format!("kind '{name}' drift: absent from models.md")),
                Some(m) => {
                    let got = (kind.harness.as_str(), kind.model.as_str(), kind.effort.as_deref());
                    let want = (m.harness.as_str(), m.model.as_str(), m.effort.as_deref());
                    if got != want {
                        return Err(format!(
                            "kind '{name}' drift: team.toml {}/{}/{} != models.md {}/{}/{}",
                            got.0, got.1, got.2.unwrap_or("-"),
                            want.0, want.1, want.2.unwrap_or("-"),
                        ));
                    }
                }
            }
        }
        // Reverse: every models.md kind must have a [kinds.*] table too.
        for name in models.keys() {
            if !cfg.kinds.contains_key(name) {
                return Err(format!(
                    "kind '{name}' drift: in models.md but absent from team.toml"
                ));
            }
        }
    }

    Ok(())
}

/// Path of the models.md to check drift against: explicit `--models`, else a
/// sibling of the team.toml. A missing file yields `None` (drift skipped).
pub fn load_models(explicit: Option<&Path>, team_file: &Path) -> Result<Option<BTreeMap<String, ModelsKind>>, String> {
    let path: PathBuf = match explicit {
        Some(p) => p.to_path_buf(),
        None => match team_file.parent() {
            Some(dir) => dir.join("models.md"),
            None => PathBuf::from("models.md"),
        },
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(parse_models_md(&text))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// Resolve one dotted key to its printable value. Arrays print one item per line.
pub fn get(raw: &Config, key: &str, project: Option<&str>) -> Result<String, String> {
    let cfg = resolve(raw.clone(), project)?;
    let unknown = || format!("unknown key '{key}'");
    let parts: Vec<&str> = key.split('.').collect();
    let out = match parts.as_slice() {
        ["schema_version"] => cfg.schema_version.to_string(),
        ["limits", "depth"] => cfg.limits.depth.to_string(),
        ["limits", "max_workers_per_orchestrator"] => {
            cfg.limits.max_workers_per_orchestrator.to_string()
        }
        ["limits", "weights", name] => cfg
            .limits
            .weights
            .get(*name)
            .map(i64::to_string)
            .ok_or_else(unknown)?,
        ["timers", "budget_default"] => cfg.timers.budget_default.clone(),
        ["timers", "hard_cap"] => cfg.timers.hard_cap.clone(),
        ["timers", "stall"] => cfg.timers.stall.clone(),
        ["timers", "idle"] => cfg.timers.idle.clone(),
        ["timers", "nudge_at"] => cfg.timers.nudge_at.to_string(),
        ["timers", "grace"] => cfg.timers.grace.to_string(),
        ["timers", "pilot_budget"] => cfg.timers.pilot_budget.clone(),
        ["strategy", "default_kind"] => cfg.strategy.default_kind.clone(),
        ["strategy", "default_reviewer"] => cfg.strategy.default_reviewer.join("\n"),
        ["strategy", "builder_reviewer_differ"] => cfg.strategy.builder_reviewer_differ.clone(),
        ["kinds", k, field] => {
            let kind = cfg.kinds.get(*k).ok_or_else(unknown)?;
            match *field {
                "harness" => kind.harness.clone(),
                "model" => kind.model.clone(),
                "effort" => kind.effort.clone().ok_or_else(|| format!("key '{key}' is unset"))?,
                "can_review" => kind.can_review.to_string(),
                _ => return Err(unknown()),
            }
        }
        ["roles", r, field] => {
            let role = cfg.roles.get(*r).ok_or_else(unknown)?;
            match *field {
                "can_spawn" => role.can_spawn.join("\n"),
                "can_prompt" => role.can_prompt.join("\n"),
                "reports_to" => role.reports_to.clone().unwrap_or_default(),
                _ => return Err(unknown()),
            }
        }
        ["taskforce", t, field] => {
            let tf = cfg.taskforce.get(*t).ok_or_else(unknown)?;
            match *field {
                "members" => tf.members.join("\n"),
                "read_only" => tf.read_only.to_string(),
                _ => return Err(unknown()),
            }
        }
        ["merge", "integ_branch"] => cfg.merge.integ_branch.clone(),
        ["grid", "workers_tab"] => cfg.grid.workers_tab.clone(),
        ["grid", "deferred_statuses"] => cfg.grid.deferred_statuses.join("\n"),
        ["projects", p, "max_workers_per_orchestrator"] => cfg
            .projects
            .get(*p)
            .and_then(|o| o.max_workers_per_orchestrator)
            .map(|v| v.to_string())
            .ok_or_else(unknown)?,
        _ => return Err(unknown()),
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        toml::from_str(
            r#"
schema_version = 2
[limits]
max_workers_per_orchestrator = 4
weights = { default = 1, pipeline = 2 }
[timers]
budget_default = "45m"
hard_cap = "120m"
stall = "10m"
idle = "2m"
nudge_at = 0.8
grace = 0.25
pilot_budget = "30m"
[roles.cos]
can_spawn = ["lead"]
can_prompt = ["lead"]
[strategy]
default_kind = "tieude"
default_reviewer = ["quansu"]
builder_reviewer_differ = "harness"
[kinds.tieude]
harness = "opencode"
model = "m"
can_review = false
[kinds.quansu]
harness = "codex"
model = "m"
effort = "high"
can_review = true
[merge]
integ_branch = "quaqua/{slug}-integ"
[grid]
workers_tab = "workers"
"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_base_passes() {
        assert!(validate(&base(), None).is_ok());
    }

    #[test]
    fn cap_over_is_rejected() {
        let mut c = base();
        c.limits.max_workers_per_orchestrator = 5;
        assert!(validate(&c, None).unwrap_err().contains("over the cap"));
    }

    #[test]
    fn get_array_prints_lines() {
        assert_eq!(get(&base(), "kinds.quansu.effort", None).unwrap(), "high");
        assert!(get(&base(), "strategy.default_reviewer", None).unwrap().contains("quansu"));
        assert!(get(&base(), "nope.nope", None).is_err());
    }
}
