//! Positive identity and depth resolution from pane tokens and owner chains.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use serde_json::Value;
use crate::herdr::HerdrOps;

/// Positive caller roles mapped from pane tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Peer,
    Lead,
    Cos,
    Unknown,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Peer => "peer",
            Role::Lead => "lead",
            Role::Cos => "cos",
            Role::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Errors during depth walk over `.owner` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepthError {
    Cycle(Vec<String>),
    NotFound(String),
}

impl DepthError {
    pub fn is_cycle(&self) -> bool {
        matches!(self, DepthError::Cycle(_))
    }
}

impl fmt::Display for DepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DepthError::Cycle(cycle) => {
                write!(f, "cycle detected in owner chain: {}", cycle.join(" -> "))
            }
            DepthError::NotFound(name) => {
                write!(f, "owner file for '{name}' not found")
            }
        }
    }
}

impl std::error::Error for DepthError {}

/// How the caller identity was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    PaneTokens,
    EnvFallback,
    Unknown,
}

impl fmt::Display for IdentitySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentitySource::PaneTokens => write!(f, "tokens"),
            IdentitySource::EnvFallback => write!(f, "env"),
            IdentitySource::Unknown => write!(f, "unknown"),
        }
    }
}

/// Resolved identity of a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub name: String,
    pub role: Role,
    pub depth: u32,
    pub source: IdentitySource,
}

impl fmt::Display for Caller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "name={}, role={}, depth={}, source={}",
            self.name, self.role, self.depth, self.source
        )
    }
}

/// Positive resolution of role from a map of pane tokens.
/// - `grid_role == "worker"` -> Role::Peer
/// - `grid_role == "orchestrator"` -> Role::Lead
/// - `grid_role == "cos"` -> Role::Cos
/// - absent, empty, or anything else ("none") -> Role::Unknown
/// NEVER defaults to Cos.
pub fn resolve(tokens: &HashMap<String, String>) -> Role {
    match tokens.get("grid_role").map(|s| s.trim()) {
        Some("worker") => Role::Peer,
        Some("orchestrator") => Role::Lead,
        Some("cos") => Role::Cos,
        _ => Role::Unknown,
    }
}

/// Resolve role from optional tokens map. Returns Role::Unknown when tokens is None.
pub fn resolve_opt(tokens: Option<&HashMap<String, String>>) -> Role {
    tokens.map(resolve).unwrap_or(Role::Unknown)
}

/// Walk .owner files starting from `start` in `runs_dir` until reaching "cos",
/// counting steps as depth.
///
/// Stops and returns `DepthError::Cycle` if a cycle is detected.
/// Returns `DepthError::NotFound` if an intermediate `.owner` file is missing.
///
/// - "cos" has depth 0.
/// - an entity whose owner is "cos" has depth 1.
/// - an entity whose owner's owner is "cos" has depth 2.
pub fn depth_walk(runs_dir: &Path, start: &str) -> Result<u32, DepthError> {
    let start = start.trim();
    if start.is_empty() {
        return Err(DepthError::NotFound(String::new()));
    }
    if start == "cos" {
        return Ok(0);
    }

    let mut current = start.to_string();
    let mut visited: Vec<String> = Vec::new();
    let mut depth: u32 = 0;

    loop {
        if current == "cos" {
            return Ok(depth);
        }
        if visited.contains(&current) {
            visited.push(current);
            return Err(DepthError::Cycle(visited));
        }
        visited.push(current.clone());

        let owner_path = runs_dir.join(format!("{current}.owner"));
        let content = match fs::read_to_string(&owner_path) {
            Ok(c) => c,
            Err(_) => return Err(DepthError::NotFound(current)),
        };

        let parent = content.trim();
        if parent.is_empty() {
            return Err(DepthError::NotFound(current));
        }

        current = parent.to_string();
        depth += 1;
    }
}

pub use depth_walk as walk_depth;

/// Resolves the caller identity from herdr pane tokens, environment fallback, and owner chains.
pub fn resolve_caller<H: HerdrOps>(herdr: &H, runs_dir: &Path) -> Caller {
    if let Some((name, role, tokens)) = caller_from_pane(herdr) {
        let depth = match role {
            Role::Cos => 0,
            Role::Lead => depth_walk(runs_dir, &name).unwrap_or(1),
            Role::Peer => {
                let owner = tokens.get("grid_owner").map(|s| s.as_str()).unwrap_or("cos");
                depth_walk(runs_dir, owner).map(|d| d + 1).unwrap_or(2)
            }
            Role::Unknown => 0,
        };
        return Caller {
            name,
            role,
            depth,
            source: IdentitySource::PaneTokens,
        };
    }

    // Fallback: QUAQUA_OWNER environment variable
    let env_owner = std::env::var("QUAQUA_OWNER").ok().filter(|s| !s.trim().is_empty());
    if let Some(owner) = env_owner {
        let role = if owner == "cos" {
            Role::Cos
        } else if crate::cap::is_lead_owner(&owner) {
            Role::Lead
        } else {
            Role::Unknown
        };
        let depth = depth_walk(runs_dir, &owner).unwrap_or(if role == Role::Lead { 1 } else { 0 });
        Caller {
            name: owner,
            role,
            depth,
            source: IdentitySource::EnvFallback,
        }
    } else {
        Caller {
            name: "unknown".to_string(),
            role: Role::Unknown,
            depth: 0,
            source: IdentitySource::Unknown,
        }
    }
}

fn caller_from_pane<H: HerdrOps>(herdr: &H) -> Option<(String, Role, HashMap<String, String>)> {
    let pane_id = std::env::var("HERDR_PANE_ID").ok()?;
    if pane_id.trim().is_empty() {
        return None;
    }
    let out = herdr.run(&["pane", "get", &pane_id]).ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let tokens_val = v.pointer("/result/pane/tokens")?.as_object()?;
    let mut tokens = HashMap::new();
    for (k, val) in tokens_val {
        if let Some(s) = val.as_str() {
            tokens.insert(k.clone(), s.to_string());
        }
    }
    let role = resolve(&tokens);
    let name = match role {
        Role::Cos => "cos".to_string(),
        Role::Lead => tokens.get("grid_owner").cloned().unwrap_or_else(|| "cos".to_string()),
        Role::Peer => tokens
            .get("grid_name")
            .cloned()
            .or_else(|| tokens.get("grid_owner").cloned())
            .unwrap_or_else(|| "unknown".to_string()),
        Role::Unknown => tokens.get("grid_owner").cloned().unwrap_or_else(|| "unknown".to_string()),
    };
    Some((name, role, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TestDir(std::path::PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_resolve_token_cases() {
        // 1. worker -> Peer
        let mut worker_tokens = HashMap::new();
        worker_tokens.insert("grid_role".to_string(), "worker".to_string());
        assert_eq!(resolve(&worker_tokens), Role::Peer);

        // 2. orchestrator -> Lead
        let mut orch_tokens = HashMap::new();
        orch_tokens.insert("grid_role".to_string(), "orchestrator".to_string());
        assert_eq!(resolve(&orch_tokens), Role::Lead);

        // 3. cos -> Cos
        let mut cos_tokens = HashMap::new();
        cos_tokens.insert("grid_role".to_string(), "cos".to_string());
        assert_eq!(resolve(&cos_tokens), Role::Cos);

        // 4. none / missing grid_role -> Unknown (NEVER default to Cos!)
        let empty_tokens = HashMap::new();
        assert_eq!(resolve(&empty_tokens), Role::Unknown);

        // 4b. explicit "none" -> Unknown
        let mut none_tokens = HashMap::new();
        none_tokens.insert("grid_role".to_string(), "none".to_string());
        assert_eq!(resolve(&none_tokens), Role::Unknown);

        // 4c. unrecognized role -> Unknown
        let mut other_tokens = HashMap::new();
        other_tokens.insert("grid_role".to_string(), "random".to_string());
        assert_eq!(resolve(&other_tokens), Role::Unknown);

        // 4d. resolve_opt(None) -> Unknown
        assert_eq!(resolve_opt(None), Role::Unknown);
    }

    #[test]
    fn test_depth_walk_cycle_detection() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runs_dir = std::env::temp_dir().join(format!("quaqua-identity-test-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_dir);
        fs::create_dir_all(&runs_dir).unwrap();
        let _guard = TestDir(runs_dir.clone());

        // Chain: worker -> lead -> cos
        fs::write(runs_dir.join("worker.owner"), "lead\n").unwrap();
        fs::write(runs_dir.join("lead.owner"), "cos\n").unwrap();

        assert_eq!(depth_walk(&runs_dir, "cos"), Ok(0));
        assert_eq!(depth_walk(&runs_dir, "lead"), Ok(1));
        assert_eq!(depth_walk(&runs_dir, "worker"), Ok(2));

        // Self cycle: a -> a
        fs::write(runs_dir.join("self.owner"), "self\n").unwrap();
        let res = depth_walk(&runs_dir, "self");
        assert!(res.is_err(), "self cycle should return Err");
        let err = res.unwrap_err();
        assert!(err.is_cycle(), "expected cycle error, got: {:?}", err);

        // 2-node cycle: a -> b -> a
        fs::write(runs_dir.join("cyc_a.owner"), "cyc_b\n").unwrap();
        fs::write(runs_dir.join("cyc_b.owner"), "cyc_a\n").unwrap();
        let res = depth_walk(&runs_dir, "cyc_a");
        assert!(res.is_err(), "2-node cycle should return Err");
        assert!(res.unwrap_err().is_cycle());

        // 3-node cycle: c1 -> c2 -> c3 -> c1
        fs::write(runs_dir.join("c1.owner"), "c2\n").unwrap();
        fs::write(runs_dir.join("c2.owner"), "c3\n").unwrap();
        fs::write(runs_dir.join("c3.owner"), "c1\n").unwrap();
        let res = depth_walk(&runs_dir, "c1");
        assert!(res.is_err(), "3-node cycle should return Err");
        assert!(res.unwrap_err().is_cycle());

        // Missing owner file
        let res = depth_walk(&runs_dir, "missing_entity");
        assert_eq!(res, Err(DepthError::NotFound("missing_entity".to_string())));
    }
    struct MockHerdrWithTokens {
        tokens_json: String,
    }
    impl HerdrOps for MockHerdrWithTokens {
        fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
            let stdout = if args.first() == Some(&"pane") && args.get(1) == Some(&"get") {
                format!(r#"{{"result":{{"pane":{{"pane_id":"w1:p1","tokens":{}}}}}}}"#, self.tokens_json)
            } else {
                r#"{"result":{}}"#.to_string()
            };
            Ok(std::process::Output {
                status: std::os::unix::process::ExitStatusExt::from_raw(0),
                stdout: stdout.into_bytes(),
                stderr: Vec::new(),
            })
        }
        fn agent_list(&self) -> std::io::Result<String> { unimplemented!() }
        fn agent_get(&self, _name: &str) -> std::io::Result<(bool, String)> { unimplemented!() }
        fn agent_read(&self, _n: &str, _s: &str, _l: u32) -> std::io::Result<String> { unimplemented!() }
        fn agent_start(&self, _n: &str, _k: &str, _p: &str, _t: u32, _a: &[&str]) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn pane_split(&self, _c: &str, _o: &str, _e: &[(&str, &str)]) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn pane_close(&self, _p: &str) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn pane_report_metadata(&self, _p: &str, _t: &[&str]) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn pane_wait_output(&self, _p: &str, _m: &str, _t: u32) -> bool { unimplemented!() }
        fn pane_read(&self, _p: &str, _s: &str, _l: u32) -> std::io::Result<String> { unimplemented!() }
        fn pane_send_keys(&self, _p: &str, _k: &str) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn agent_send_keys(&self, _n: &str, _key: &str) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn agent_wait_idle(&self, _n: &str, _t: u32) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn agent_prompt_nowait(&self, _n: &str, _p: &str) -> std::io::Result<std::process::Output> { unimplemented!() }
        fn agent_prompt_wait_background(&self, _n: &str, _p: &str, _t: u32, _f: std::fs::File) -> std::io::Result<std::process::Child> { unimplemented!() }
    }

    #[test]
    fn test_resolve_caller_with_pane_tokens() {
        let _lock = crate::test_env::lock_env();
        let _pane_guard = crate::test_env::TestEnvVarGuard::set("HERDR_PANE_ID", "w1:p1");

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runs_dir = std::env::temp_dir().join(format!("quaqua-identity-pane-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_dir);
        fs::create_dir_all(&runs_dir).unwrap();
        let _guard = TestDir(runs_dir.clone());

        fs::write(runs_dir.join("my-lead.owner"), "cos\n").unwrap();

        // CoS pane
        let cos_herdr = MockHerdrWithTokens {
            tokens_json: r#"{"grid_role":"cos"}"#.to_string(),
        };
        let caller = resolve_caller(&cos_herdr, &runs_dir);
        assert_eq!(caller.role, Role::Cos);
        assert_eq!(caller.name, "cos");
        assert_eq!(caller.depth, 0);
        assert_eq!(caller.source, IdentitySource::PaneTokens);

        // Lead pane
        let lead_herdr = MockHerdrWithTokens {
            tokens_json: r#"{"grid_role":"orchestrator","grid_owner":"my-lead"}"#.to_string(),
        };
        let caller = resolve_caller(&lead_herdr, &runs_dir);
        assert_eq!(caller.role, Role::Lead);
        assert_eq!(caller.name, "my-lead");
        assert_eq!(caller.depth, 1);
        assert_eq!(caller.source, IdentitySource::PaneTokens);

        // Peer pane
        let peer_herdr = MockHerdrWithTokens {
            tokens_json: r#"{"grid_role":"worker","grid_owner":"my-lead","grid_name":"sude-task"}"#.to_string(),
        };
        let caller = resolve_caller(&peer_herdr, &runs_dir);
        assert_eq!(caller.role, Role::Peer);
        assert_eq!(caller.name, "sude-task");
        assert_eq!(caller.depth, 2);
        assert_eq!(caller.source, IdentitySource::PaneTokens);
    }

    #[test]
    fn test_resolve_caller_env_fallback() {
        let _lock = crate::test_env::lock_env();
        let _pane_guard = crate::test_env::TestEnvVarGuard::set("HERDR_PANE_ID", "");

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runs_dir = std::env::temp_dir().join(format!("quaqua-identity-env-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_dir);
        fs::create_dir_all(&runs_dir).unwrap();
        let _guard = TestDir(runs_dir.clone());
        fs::write(runs_dir.join("env-lead.owner"), "cos\n").unwrap();

        let dummy_herdr = MockHerdrWithTokens {
            tokens_json: "{}".to_string(),
        };

        // QUAQUA_OWNER = "cos"
        {
            let _owner_guard = crate::test_env::TestEnvVarGuard::set("QUAQUA_OWNER", "cos");
            let caller = resolve_caller(&dummy_herdr, &runs_dir);
            assert_eq!(caller.role, Role::Cos);
            assert_eq!(caller.name, "cos");
            assert_eq!(caller.depth, 0);
            assert_eq!(caller.source, IdentitySource::EnvFallback);
        }

        // QUAQUA_OWNER = "env-lead"
        {
            let _owner_guard = crate::test_env::TestEnvVarGuard::set("QUAQUA_OWNER", "env-lead");
            let caller = resolve_caller(&dummy_herdr, &runs_dir);
            assert_eq!(caller.role, Role::Lead);
            assert_eq!(caller.name, "env-lead");
            assert_eq!(caller.depth, 1);
            assert_eq!(caller.source, IdentitySource::EnvFallback);
        }

        // QUAQUA_OWNER unset
        {
            let _owner_guard = crate::test_env::TestEnvVarGuard::set("QUAQUA_OWNER", "");
            let caller = resolve_caller(&dummy_herdr, &runs_dir);
            assert_eq!(caller.role, Role::Unknown);
            assert_eq!(caller.name, "unknown");
            assert_eq!(caller.depth, 0);
            assert_eq!(caller.source, IdentitySource::Unknown);
        }
    }

    #[test]
    fn test_depth_walk_whitespace_and_longer_cycle() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runs_dir = std::env::temp_dir().join(format!("quaqua-identity-edge-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_dir);
        fs::create_dir_all(&runs_dir).unwrap();
        let _guard = TestDir(runs_dir.clone());

        // Trailing whitespace and newline in owner files
        fs::write(runs_dir.join("w1.owner"), "  l1  \n\n").unwrap();
        fs::write(runs_dir.join("l1.owner"), "\ncos\r\n").unwrap();
        assert_eq!(depth_walk(&runs_dir, "  w1  "), Ok(2));
        assert_eq!(depth_walk(&runs_dir, "l1"), Ok(1));

        // Cycle with lead-in: x -> y -> a -> b -> a
        fs::write(runs_dir.join("x.owner"), "y\n").unwrap();
        fs::write(runs_dir.join("y.owner"), "a\n").unwrap();
        fs::write(runs_dir.join("a.owner"), "b\n").unwrap();
        fs::write(runs_dir.join("b.owner"), "a\n").unwrap();
        let res = depth_walk(&runs_dir, "x");
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.is_cycle());
        assert_eq!(err, DepthError::Cycle(vec!["x".into(), "y".into(), "a".into(), "b".into(), "a".into()]));
    }
}
