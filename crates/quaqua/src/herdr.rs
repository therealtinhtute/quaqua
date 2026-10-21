//! Herdr execution helpers and HerdrOps trait.

use std::process::{Child, Command, Output};

pub trait HerdrOps {
    fn run(&self, args: &[&str]) -> std::io::Result<Output>;
    fn agent_list(&self) -> std::io::Result<String>;
    fn agent_get(&self, name: &str) -> std::io::Result<(bool, String)>;
    fn agent_read(&self, name: &str, source: &str, lines: u32) -> std::io::Result<String>;
    fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane: &str,
        timeout_ms: u32,
        extra_args: &[&str],
    ) -> std::io::Result<Output>;
    fn pane_split(&self, cwd: &str, owner: &str, envs: &[(&str, &str)]) -> std::io::Result<Output>;
    fn pane_close(&self, pane: &str) -> std::io::Result<Output>;
    fn pane_report_metadata(&self, pane: &str, tokens: &[&str]) -> std::io::Result<Output>;
    fn pane_wait_output(&self, pane: &str, pattern: &str, timeout_ms: u32) -> bool;
    fn pane_read(&self, pane: &str, source: &str, lines: u32) -> std::io::Result<String>;
    fn pane_send_keys(&self, pane: &str, key: &str) -> std::io::Result<Output>;
    fn agent_send_keys(&self, name: &str, key: &str) -> std::io::Result<Output>;
    fn agent_wait_idle(&self, name: &str, timeout_ms: u32) -> std::io::Result<Output>;
    fn agent_prompt_nowait(&self, name: &str, prompt: &str) -> std::io::Result<Output>;
    /// One blocking wait. Default implementation drives `run`; the live
    /// `Herdr` also has inherent methods with the same behaviour.
    fn agent_wait(
        &self,
        name: &str,
        until: &[&str],
        timeout_ms: u64,
    ) -> std::io::Result<Output> {
        if timeout_ms == 0 {
            return self.run(&["agent", "get", name]);
        }
        let timeout = timeout_ms.to_string();
        let mut args = vec!["agent", "wait", name];
        for state in until {
            args.push("--until");
            args.push(state);
        }
        args.push("--timeout");
        args.push(&timeout);
        self.run(&args)
    }
    fn agent_prompt_wait(
        &self,
        name: &str,
        prompt: &str,
        until: &[&str],
        timeout_ms: u64,
    ) -> std::io::Result<Output> {
        let timeout = timeout_ms.to_string();
        let mut args = vec!["agent", "prompt", name, prompt, "--wait"];
        for state in until {
            args.push("--until");
            args.push(state);
        }
        args.push("--timeout");
        args.push(&timeout);
        self.run(&args)
    }
    fn agent_prompt_wait_background(
        &self,
        name: &str,
        prompt: &str,
        timeout_ms: u32,
        log_file: std::fs::File,
    ) -> std::io::Result<Child>;
}

#[derive(Debug, Clone)]
pub struct Herdr {
    pub bin: String,
}

/// The herdr binary every exec site resolves: `HERDR_BIN_PATH` when set (an
/// empty value counts as unset), else `herdr` through `PATH`.
pub fn bin_path() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "herdr".to_string())
}

impl Herdr {
    pub fn new() -> Self {
        Herdr { bin: bin_path() }
    }

    /// One blocking native `agent wait`. `until` empty means herdr's default
    /// set (`idle`, `done`, `blocked`); `timeout_ms == 0` is a plain `agent get`.
    pub fn agent_wait(
        &self,
        name: &str,
        until: &[&str],
        timeout_ms: u64,
    ) -> std::io::Result<Output> {
        if timeout_ms == 0 {
            return self.run(&["agent", "get", name]);
        }
        let timeout = timeout_ms.to_string();
        let mut cmd = Command::new(&self.bin);
        cmd.args(["agent", "wait", name]);
        for state in until {
            cmd.args(["--until", state]);
        }
        cmd.args(["--timeout", &timeout]);
        cmd.output()
    }

    /// Submit a prompt and wait for the first landed state. herdr's own land
    /// check: `agent_prompt_stalled` means the input never reached the turn.
    pub fn agent_prompt_wait(
        &self,
        name: &str,
        prompt: &str,
        until: &[&str],
        timeout_ms: u64,
    ) -> std::io::Result<Output> {
        let timeout = timeout_ms.to_string();
        let mut cmd = Command::new(&self.bin);
        cmd.args(["agent", "prompt", name, prompt, "--wait"]);
        for state in until {
            cmd.args(["--until", state]);
        }
        cmd.args(["--timeout", &timeout]);
        cmd.output()
    }
}

impl Default for Herdr {
    fn default() -> Self {
        Self::new()
    }
}

impl HerdrOps for Herdr {
    fn run(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new(&self.bin).args(args).output()
    }

    fn agent_list(&self) -> std::io::Result<String> {
        let out = self.run(&["agent", "list"])?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn agent_get(&self, name: &str) -> std::io::Result<(bool, String)> {
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

    fn agent_read(&self, name: &str, source: &str, lines: u32) -> std::io::Result<String> {
        let lines_str = lines.to_string();
        let out = self.run(&["agent", "read", name, "--source", source, "--lines", &lines_str])?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane: &str,
        timeout_ms: u32,
        extra_args: &[&str],
    ) -> std::io::Result<Output> {
        let mut cmd = Command::new(&self.bin);
        let timeout_str = timeout_ms.to_string();
        cmd.args(["agent", "start", name, "--kind", kind, "--pane", pane, "--timeout", &timeout_str]);
        cmd.args(extra_args);
        cmd.output()
    }

    fn pane_split(&self, cwd: &str, owner: &str, envs: &[(&str, &str)]) -> std::io::Result<Output> {
        let mut cmd = Command::new(&self.bin);
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

    fn pane_close(&self, pane: &str) -> std::io::Result<Output> {
        self.run(&["pane", "close", pane])
    }

    fn pane_report_metadata(&self, pane: &str, tokens: &[&str]) -> std::io::Result<Output> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["pane", "report-metadata", pane, "--source", "herdr-quaqua"]);
        for token in tokens {
            cmd.args(["--token", token]);
        }
        cmd.output()
    }

    fn pane_wait_output(&self, pane: &str, pattern: &str, timeout_ms: u32) -> bool {
        let t_str = timeout_ms.to_string();
        match self.run(&["pane", "wait-output", pane, "--match", pattern, "--timeout", &t_str]) {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }

    fn pane_read(&self, pane: &str, source: &str, lines: u32) -> std::io::Result<String> {
        let lines_str = lines.to_string();
        let out = self.run(&["pane", "read", pane, "--source", source, "--lines", &lines_str])?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn pane_send_keys(&self, pane: &str, key: &str) -> std::io::Result<Output> {
        self.run(&["pane", "send-keys", pane, key])
    }

    fn agent_send_keys(&self, name: &str, key: &str) -> std::io::Result<Output> {
        self.run(&["agent", "send-keys", name, key])
    }

    fn agent_wait_idle(&self, name: &str, timeout_ms: u32) -> std::io::Result<Output> {
        let t_str = timeout_ms.to_string();
        self.run(&["agent", "wait", name, "--until", "idle", "--timeout", &t_str])
    }

    fn agent_prompt_nowait(&self, name: &str, prompt: &str) -> std::io::Result<Output> {
        self.run(&["agent", "prompt", name, prompt])
    }

    fn agent_prompt_wait_background(
        &self,
        name: &str,
        prompt: &str,
        timeout_ms: u32,
        log_file: std::fs::File,
    ) -> std::io::Result<Child> {
        let t_str = timeout_ms.to_string();
        Command::new(&self.bin)
            .arg("agent")
            .arg("prompt")
            .arg(name)
            .arg(prompt)
            .arg("--wait")
            .arg("--timeout")
            .arg(&t_str)
            .stdout(log_file.try_clone().unwrap())
            .stderr(log_file)
            .spawn()
    }
}

pub trait HerdrDriver {
    fn snapshot(&mut self) -> Result<crate::grid::Snapshot, String>;
    fn move_pane(
        &mut self,
        pane: &str,
        target: &crate::grid::Target,
    ) -> Result<crate::grid::MoveResult, String>;
    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String>;
}

impl HerdrDriver for Herdr {
    fn snapshot(&mut self) -> Result<crate::grid::Snapshot, String> {
        let out = Command::new(&self.bin)
            .args(["api", "snapshot"])
            .output()
            .map_err(|e| format!("cannot run {} api snapshot: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("{} api snapshot failed: {err}", self.bin));
        }
        let val: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("invalid snapshot JSON from herdr: {e}"))?;
        crate::grid::types::parse_snapshot_value(&val)
    }

    fn move_pane(
        &mut self,
        pane: &str,
        target: &crate::grid::Target,
    ) -> Result<crate::grid::MoveResult, String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["pane", "move", pane]);
        match target {
            crate::grid::Target::NewTab { label } => {
                cmd.args(["--new-tab", "--label", label, "--no-focus"]);
            }
            crate::grid::Target::Split {
                tab,
                anchor,
                direction,
                ratio,
            } => {
                let ratio_s = format!("{ratio:.1}");
                cmd.args([
                    "--tab",
                    tab,
                    "--split",
                    direction,
                    "--target-pane",
                    anchor,
                    "--ratio",
                    &ratio_s,
                    "--no-focus",
                ]);
            }
        }
        let out = cmd
            .output()
            .map_err(|e| format!("cannot run {} pane move: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("pane move failed: {err}"));
        }
        let val: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("invalid move response JSON: {e}"))?;
        crate::grid::apply::parse_move_response(&val, pane)
    }

    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String> {
        let out = Command::new(&self.bin)
            .args(["pane", "rename", pane, label])
            .output()
            .map_err(|e| format!("cannot run {} pane rename: {e}", self.bin))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("pane rename failed: {err}"));
        }
        Ok(())
    }
}
