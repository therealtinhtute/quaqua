//! Herdr execution helpers. Commands execute the `herdr` binary found in PATH.

use std::process::{Command, Output};

#[derive(Debug, Clone, Default)]
pub struct Herdr;

impl Herdr {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new("herdr").args(args).output()
    }

    pub fn agent_list(&self) -> std::io::Result<String> {
        let out = self.run(&["agent", "list"])?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn agent_get(&self, name: &str) -> std::io::Result<(bool, String)> {
        let out = self.run(&["agent", "get", name])?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let combined = if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        Ok((out.status.success(), combined))
    }

    pub fn agent_read(&self, name: &str, source: &str, lines: u32) -> std::io::Result<String> {
        let lines_str = lines.to_string();
        let out = self.run(&["agent", "read", name, "--source", source, "--lines", &lines_str])?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn pane_split(&self, cwd: &str, owner: &str, envs: &[(&str, &str)]) -> std::io::Result<Output> {
        let mut cmd = Command::new("herdr");
        cmd.args(["pane", "split", "--current", "--direction", "right", "--no-focus", "--cwd", cwd]);
        cmd.args(["--env", "QUAQUA_SANDBOX=1"]);
        let owner_env = format!("QUAQUA_OWNER={}", owner);
        cmd.args(["--env", &owner_env]);
        for (k, v) in envs {
            let e = format!("{}={}", k, v);
            cmd.args(["--env", &e]);
        }
        cmd.output()
    }

    pub fn pane_close(&self, pane: &str) -> std::io::Result<Output> {
        self.run(&["pane", "close", pane])
    }

    pub fn pane_report_metadata(&self, pane: &str, tokens: &[&str]) -> std::io::Result<Output> {
        let mut cmd = Command::new("herdr");
        cmd.args(["pane", "report-metadata", pane, "--source", "herdr-grid"]);
        for token in tokens {
            cmd.args(["--token", token]);
        }
        cmd.output()
    }

    pub fn pane_wait_output(&self, pane: &str, pattern: &str, timeout_ms: u32) -> bool {
        let t_str = timeout_ms.to_string();
        match self.run(&["pane", "wait-output", pane, "--match", pattern, "--timeout", &t_str]) {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    pub fn pane_send_keys(&self, pane: &str, key: &str) -> std::io::Result<Output> {
        self.run(&["pane", "send-keys", pane, key])
    }

    pub fn agent_wait_idle(&self, name: &str, timeout_ms: u32) -> std::io::Result<Output> {
        let t_str = timeout_ms.to_string();
        self.run(&["agent", "wait", name, "--until", "idle", "--timeout", &t_str])
    }

    pub fn agent_prompt_nowait(&self, name: &str, prompt: &str) -> std::io::Result<Output> {
        self.run(&["agent", "prompt", name, prompt])
    }
}
