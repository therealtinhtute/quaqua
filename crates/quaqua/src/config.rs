//! `team.toml` typed model, project-override resolution, validation and key lookup.
//!
//! The shape mirrors docs/research/herdr-grid-rust-migration-plan.md § `team.toml`.
//! Resolution order is project override -> defaults, then validate. A validation
//! failure is rejected with one clear message (first failure wins).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 2;
/// AGENTS.md `## Rules`: "Max 4 task workers per orchestrator".
pub const CAP_CEILING: i64 = 8;
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

fn default_budget() -> String {
    "45m".to_string()
}
fn default_hard_cap() -> String {
    "120m".to_string()
}
fn default_stall() -> String {
    "10m".to_string()
}
fn default_idle() -> String {
    "2m".to_string()
}
fn default_nudge_at() -> f64 {
    0.8
}
fn default_grace() -> f64 {
    0.25
}
fn default_pilot_budget() -> String {
    "30m".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timers {
    #[serde(default = "default_budget")]
    pub budget_default: String,
    #[serde(default = "default_hard_cap")]
    pub hard_cap: String,
    #[serde(default = "default_stall")]
    pub stall: String,
    #[serde(default = "default_idle")]
    pub idle: String,
    #[serde(default = "default_nudge_at")]
    pub nudge_at: f64,
    #[serde(default = "default_grace")]
    pub grace: f64,
    #[serde(default = "default_pilot_budget")]
    pub pilot_budget: String,
}

impl Default for Timers {
    fn default() -> Self {
        Timers {
            budget_default: default_budget(),
            hard_cap: default_hard_cap(),
            stall: default_stall(),
            idle: default_idle(),
            nudge_at: default_nudge_at(),
            grace: default_grace(),
            pilot_budget: default_pilot_budget(),
        }
    }
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
    #[serde(default)]
    pub lead_kind: Option<String>,
}

impl Default for Strategy {
    fn default() -> Self {
        Strategy {
            default_kind: String::new(),
            default_reviewer: Vec::new(),
            builder_reviewer_differ: default_differ(),
            lead_kind: None,
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
    /// Optional one-hop fallback kind, taken only when the agent fails to start.
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub can_review: bool,
}

/// The effort values a harness accepts. opencode effort is a model *variant*
/// (`opencode --help` lists no `--model`; the interactive TUI reads the model
/// from config as `provider/model#variant`, and the variant vocabulary is the
/// `ReasoningEffort` set). omp's is `omp --help` `--thinking`. claude's is
/// `claude --help` `--effort <level> (low, medium, high, xhigh, max)`.
pub fn effort_allowed(harness: &str, effort: &str) -> bool {
    match harness {
        "opencode" => ["none", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&effort),
        "omp" => ["off", "minimal", "low", "medium", "high", "xhigh", "max", "auto"].contains(&effort),
        "claude" => ["low", "medium", "high", "xhigh", "max"].contains(&effort),
        _ => false,
    }
}

/// The variant vocabulary allowed for known opencode models.
/// Key is the last path segment of the model ID (e.g. "deepseek-v4.1-flash").
pub fn opencode_model_variants(model: &str) -> Option<&'static [&'static str]> {
    let model_id = model.rsplit('/').next().unwrap_or(model);
    match model_id {
        "deepseek-v4.1-flash" => Some(&["low", "high", "max"]),
        _ => None,
    }
}

/// Check if an effort/variant value is allowed for a specific harness and model.
/// opencode models can restrict variant vocabulary (e.g. deepseek-v4.1-flash takes low, high, max).
pub fn model_effort_allowed(harness: &str, model: &str, effort: &str) -> bool {
    if harness != "opencode" {
        return true;
    }
    match opencode_model_variants(model) {
        Some(allowed) => allowed.contains(&effort),
        None => false,
    }
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

/// Parse a duration string like "45m", "120s", "2h" into seconds.
pub fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }
    let split_idx = s.find(|c: char| !c.is_ascii_digit());
    let (num_str, unit_str) = match split_idx {
        Some(idx) => (&s[..idx], &s[idx..]),
        None => return Err(format!("missing unit in duration '{s}' (e.g. 45m, 120s)")),
    };
    if num_str.is_empty() {
        return Err(format!("invalid duration '{s}': missing number"));
    }
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid duration number in '{s}'"))?;
    let multiplier: u64 = match unit_str {
        "s" | "sec" | "secs" => 1,
        "m" | "min" | "mins" => 60,
        "h" | "hr" | "hrs" => 3600,
        _ => return Err(format!("unknown duration unit '{unit_str}' in '{s}'")),
    };
    let total = n
        .checked_mul(multiplier)
        .ok_or_else(|| format!("duration '{s}' overflowed"))?;
    if total == 0 {
        return Err(format!("duration '{s}' must be greater than zero"));
    }
    Ok(total)
}

/// Read and parse a team.toml. Parse errors name the file.
pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}
/// Load team config from environment, relative path, home directory, or embedded fallback.
pub fn load_team_config() -> Result<Config, (i32, String)> {
    if let Ok(path_str) = std::env::var("QUAQUA_TEAM_TOML") {
        return load(Path::new(&path_str)).map_err(|e| (1, format!("error: {e}\n")));
    }
    if Path::new("team.toml").is_file() {
        return load(Path::new("team.toml")).map_err(|e| (1, format!("error: {e}\n")));
    }
    let home_team = crate::workspace::workspace_root().join("team.toml");
    if home_team.is_file() {
        return load(&home_team).map_err(|e| (1, format!("error: {e}\n")));
    }
    const DEFAULT_TOML: &str = include_str!("../../../team.toml");
    toml::from_str(DEFAULT_TOML).map_err(|e| (1, format!("error: failed to parse default team.toml: {e}\n")))
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

/// Validate the resolved config. team.toml is the only source of
/// harness/model/effort, so every kind is checked here; models.md is prose.
pub fn validate(cfg: &Config) -> Result<(), String> {
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
    parse_duration(&cfg.timers.budget_default).map_err(|e| format!("timers.budget_default: {e}"))?;
    parse_duration(&cfg.timers.hard_cap).map_err(|e| format!("timers.hard_cap: {e}"))?;
    parse_duration(&cfg.timers.stall).map_err(|e| format!("timers.stall: {e}"))?;
    parse_duration(&cfg.timers.idle).map_err(|e| format!("timers.idle: {e}"))?;
    parse_duration(&cfg.timers.pilot_budget).map_err(|e| format!("timers.pilot_budget: {e}"))?;
    if cfg.timers.nudge_at <= 0.0 || cfg.timers.nudge_at >= 1.0 || cfg.timers.nudge_at.is_nan() {
        return Err(format!(
            "timers.nudge_at {} must be in (0, 1)",
            cfg.timers.nudge_at
        ));
    }
    if cfg.timers.grace <= 0.0 || cfg.timers.grace >= 1.0 || cfg.timers.grace.is_nan() {
        return Err(format!(
            "timers.grace {} must be in (0, 1)",
            cfg.timers.grace
        ));
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

    // team.toml is the only source of harness/model/effort: fail closed on a kind
    // that cannot launch, or on a fallback that names a missing kind or itself.
    for (name, kind) in &cfg.kinds {
        if !["opencode", "omp", "claude"].contains(&kind.harness.as_str()) {
            return Err(format!(
                "kind '{name}': unsupported harness '{}'",
                kind.harness
            ));
        }
        if kind.model.trim().is_empty() {
            return Err(format!("kind '{name}': model is empty"));
        }
        let effort = match kind.effort.as_deref() {
            Some(e) if !e.trim().is_empty() => e,
            _ => return Err(format!("kind '{name}': effort is missing")),
        };
        if !effort_allowed(&kind.harness, effort) {
            return Err(format!(
                "kind '{name}': harness '{}' does not take effort '{effort}'",
                kind.harness
            ));
        }
        if kind.harness == "opencode" {
            match opencode_model_variants(&kind.model) {
                Some(allowed) => {
                    if !allowed.contains(&effort) {
                        return Err(format!(
                            "kind '{name}': harness 'opencode' model '{}' does not take effort '{effort}' (allowed: {})",
                            kind.model,
                            allowed.join(", ")
                        ));
                    }
                }
                None => {
                    return Err(format!(
                        "kind '{name}': opencode model '{}' is not in allowlist (fix: add model and variants to config.rs model_effort_allowed)",
                        kind.model
                    ));
                }
            }
        }
        if let Some(fb) = kind.fallback.as_deref() {
            if fb.trim().is_empty() {
                return Err(format!("kind '{name}': fallback is empty"));
            }
            if fb == name {
                return Err(format!("kind '{name}': fallback names itself"));
            }
            let fb_kind = match cfg.kinds.get(fb) {
                Some(k) => k,
                None => return Err(format!("kind '{name}': fallback '{fb}' is not a kind")),
            };
            if kind.can_review && !fb_kind.can_review {
                return Err(format!(
                    "kind '{name}': can_review kind has fallback '{fb}' which cannot review"
                ));
            }
        }
    }
    Ok(())
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
        ["strategy", "lead_kind"] => cfg.strategy.lead_kind.clone().unwrap_or_else(|| "tieude".to_string()),
        ["strategy", "default_reviewer"] => cfg.strategy.default_reviewer.join("\n"),
        ["strategy", "builder_reviewer_differ"] => cfg.strategy.builder_reviewer_differ.clone(),
        ["kinds", k, field] => {
            let kind = cfg.kinds.get(*k).ok_or_else(unknown)?;
            match *field {
                "harness" => kind.harness.clone(),
                "model" => kind.model.clone(),
                "effort" => kind.effort.clone().ok_or_else(|| format!("key '{key}' is unset"))?,
                "fallback" => kind.fallback.clone().ok_or_else(|| format!("key '{key}' is unset"))?,
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
default_reviewer = ["suhuynh"]
builder_reviewer_differ = "harness"
[kinds.tieude]
harness = "opencode"
model = "opencode-go/deepseek-v4.1-flash"
effort = "max"
fallback = "suhuynh"
can_review = false
[kinds.suhuynh]
harness = "claude"
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
        assert!(validate(&base()).is_ok());
    }

    #[test]
    fn cap_over_is_rejected() {
        let mut c = base();
        c.limits.max_workers_per_orchestrator = 9;
        assert!(validate(&c).unwrap_err().contains("over the cap"));
    }

    #[test]
    fn get_array_prints_lines() {
        assert_eq!(get(&base(), "kinds.suhuynh.effort", None).unwrap(), "high");
        assert_eq!(get(&base(), "kinds.tieude.fallback", None).unwrap(), "suhuynh");
        assert!(get(&base(), "strategy.default_reviewer", None).unwrap().contains("suhuynh"));
        assert!(get(&base(), "nope.nope", None).is_err());
    }

    #[test]
    fn missing_effort_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().effort = None;
        let err = validate(&c).unwrap_err();
        assert!(err.contains("effort is missing"), "{err}");
    }

    #[test]
    fn empty_model_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().model = "  ".to_string();
        let err = validate(&c).unwrap_err();
        assert!(err.contains("model is empty"), "{err}");
    }

    #[test]
    fn unsupported_effort_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().effort = Some("banana".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("does not take effort 'banana'"), "{err}");
    }

    #[test]
    fn claude_effort_vocabulary_is_enforced() {
        // opencode's "none" is not a claude effort value.
        let mut c = base();
        c.kinds.get_mut("suhuynh").unwrap().effort = Some("none".to_string());
        assert!(validate(&c).is_err());
    }

    #[test]
    fn fallback_to_missing_kind_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().fallback = Some("ghost".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("fallback 'ghost' is not a kind"), "{err}");
    }

    #[test]
    fn fallback_to_self_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().fallback = Some("tieude".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("fallback names itself"), "{err}");
    }

    #[test]
    fn fallback_from_can_review_to_non_review_is_rejected() {
        let mut c = base();
        c.kinds.get_mut("suhuynh").unwrap().fallback = Some("tieude".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("cannot review"), "{err}");
    }

    #[test]
    fn opencode_model_variant_is_enforced() {
        let mut c = base();
        c.kinds.get_mut("tieude").unwrap().model = "opencode-go/deepseek-v4.1-flash".to_string();
        // medium is a valid opencode variant in general, but deepseek-v4.1-flash only takes low, high, max
        c.kinds.get_mut("tieude").unwrap().effort = Some("medium".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("does not take effort 'medium'"), "{err}");

        // openrouter path also matches last segment and rejects medium
        c.kinds.get_mut("tieude").unwrap().model = "openrouter/deepseek/deepseek-v4.1-flash".to_string();
        c.kinds.get_mut("tieude").unwrap().effort = Some("medium".to_string());
        let err = validate(&c).unwrap_err();
        assert!(err.contains("does not take effort 'medium'"), "{err}");

        // unknown opencode model rejected with fix message
        c.kinds.get_mut("tieude").unwrap().model = "opencode-go/unknown-model".to_string();
        c.kinds.get_mut("tieude").unwrap().effort = Some("max".to_string());
        let err = validate(&c).unwrap_err();
        assert!(
            err.contains("is not in allowlist (fix: add model and variants to config.rs model_effort_allowed)"),
            "{err}"
        );

        // low and max work for known models
        c.kinds.get_mut("tieude").unwrap().model = "opencode-go/deepseek-v4.1-flash".to_string();
        c.kinds.get_mut("tieude").unwrap().effort = Some("low".to_string());
        assert!(validate(&c).is_ok());
        c.kinds.get_mut("tieude").unwrap().effort = Some("max".to_string());
        assert!(validate(&c).is_ok());
    }
}
