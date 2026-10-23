//! Agent liveness with "absence ≠ broken" (issue #69).
//!
//! One failed `herdr agent get` proves nothing: a timeout, a socket error or
//! an empty `agent list` is `Unknown`, never `Down`. `Down` needs two
//! independent checks that agree: `agent get` says `agent_not_found` and
//! `agent list` parses, is non-empty, and does not name the agent.

use crate::herdr::HerdrOps;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    /// `agent get` returned `result.agent`; carries the raw body.
    Alive(String),
    /// Confirmed gone by `agent get` and `agent list`.
    Down,
    /// Could not prove either; carries the reason.
    Unknown(String),
}

pub fn liveness<H: HerdrOps>(herdr: &H, name: &str) -> Liveness {
    let (ok, body) = match herdr.agent_get(name) {
        Ok(r) => r,
        Err(e) => return Liveness::Unknown(format!("herdr agent get failed: {e}")),
    };
    let json = serde_json::from_str::<Value>(&body).ok();
    if ok
        && json
            .as_ref()
            .and_then(|j| j.pointer("/result/agent"))
            .is_some()
    {
        return Liveness::Alive(body);
    }
    let not_found = json
        .as_ref()
        .and_then(|j| j.pointer("/error/code"))
        .and_then(Value::as_str)
        == Some("agent_not_found")
        || body.contains("agent_not_found");
    if !not_found {
        return Liveness::Unknown(format!("herdr agent get failed: {}", body.trim()));
    }
    // Second, independent check before calling it gone.
    let list = match herdr.agent_list() {
        Ok(s) => s,
        Err(e) => {
            return Liveness::Unknown(format!("agent_not_found, but herdr agent list failed: {e}"))
        }
    };
    let names: Option<Vec<String>> = serde_json::from_str::<Value>(&list)
        .ok()
        .and_then(|v| {
            v.pointer("/result/agents")
                .and_then(Value::as_array)
                .cloned()
        })
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        });
    match names {
        None => {
            Liveness::Unknown("agent_not_found, but herdr agent list is unreadable".to_string())
        }
        Some(n) if n.is_empty() => {
            Liveness::Unknown("agent_not_found, but herdr agent list is empty".to_string())
        }
        Some(n) if n.iter().any(|x| x == name) => {
            Liveness::Unknown("agent_not_found, but herdr agent list still names it".to_string())
        }
        Some(_) => Liveness::Down,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::process::Output;

    struct Fake {
        get: io::Result<(bool, String)>,
        list: io::Result<String>,
    }

    fn clone_res<T: Clone>(r: &io::Result<T>) -> io::Result<T> {
        match r {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
        }
    }

    impl HerdrOps for Fake {
        fn run(&self, _: &[&str]) -> io::Result<Output> {
            unimplemented!()
        }
        fn agent_list(&self) -> io::Result<String> {
            clone_res(&self.list)
        }
        fn agent_get(&self, _: &str) -> io::Result<(bool, String)> {
            clone_res(&self.get)
        }
        fn agent_read(&self, _: &str, _: &str, _: u32) -> io::Result<String> {
            unimplemented!()
        }
        fn agent_start(&self, _: &str, _: &str, _: &str, _: u32, _: &[&str]) -> io::Result<Output> {
            unimplemented!()
        }
        fn pane_split(&self, _: &str, _: &str, _: &[(&str, &str)]) -> io::Result<Output> {
            unimplemented!()
        }
        fn pane_close(&self, _: &str) -> io::Result<Output> {
            unimplemented!()
        }
        fn pane_report_metadata(&self, _: &str, _: &[&str]) -> io::Result<Output> {
            unimplemented!()
        }
        fn pane_wait_output(&self, _: &str, _: &str, _: u32) -> bool {
            unimplemented!()
        }
        fn pane_read(&self, _: &str, _: &str, _: u32) -> io::Result<String> {
            unimplemented!()
        }
        fn pane_send_keys(&self, _: &str, _: &str) -> io::Result<Output> {
            unimplemented!()
        }
        fn agent_send_keys(&self, _: &str, _: &str) -> io::Result<Output> {
            unimplemented!()
        }
        fn agent_wait_idle(&self, _: &str, _: u32) -> io::Result<Output> {
            unimplemented!()
        }
        fn agent_prompt_nowait(&self, _: &str, _: &str) -> io::Result<Output> {
            unimplemented!()
        }
        fn agent_prompt_wait_background(
            &self,
            _: &str,
            _: &str,
            _: u32,
            _: std::fs::File,
        ) -> io::Result<std::process::Child> {
            unimplemented!()
        }
    }

    const ALIVE: &str = r#"{"result":{"agent":{"name":"w","pane_id":"%1"}}}"#;
    const NOT_FOUND: &str = r#"{"error":{"code":"agent_not_found"}}"#;
    const LIST_OTHER: &str = r#"{"result":{"agents":[{"name":"cos"}]}}"#;
    const LIST_W: &str = r#"{"result":{"agents":[{"name":"w"}]}}"#;
    const LIST_EMPTY: &str = r#"{"result":{"agents":[]}}"#;

    fn fake(get: io::Result<(bool, String)>, list: io::Result<String>) -> Fake {
        Fake { get, list }
    }

    #[test]
    fn alive_when_get_returns_agent() {
        let f = fake(Ok((true, ALIVE.into())), Ok(LIST_W.into()));
        assert_eq!(liveness(&f, "w"), Liveness::Alive(ALIVE.into()));
    }

    #[test]
    fn down_only_when_list_confirms_not_found() {
        let f = fake(Ok((false, NOT_FOUND.into())), Ok(LIST_OTHER.into()));
        assert_eq!(liveness(&f, "w"), Liveness::Down);
    }

    #[test]
    fn not_found_but_listed_is_unknown() {
        let f = fake(Ok((false, NOT_FOUND.into())), Ok(LIST_W.into()));
        assert!(matches!(liveness(&f, "w"), Liveness::Unknown(_)));
    }

    #[test]
    fn get_timeout_is_unknown() {
        let f = fake(
            Err(io::Error::new(io::ErrorKind::TimedOut, "timed out")),
            Ok(LIST_OTHER.into()),
        );
        assert!(matches!(liveness(&f, "w"), Liveness::Unknown(_)));
    }

    #[test]
    fn empty_or_broken_list_is_unknown() {
        for list in [
            Ok(LIST_EMPTY.to_string()),
            Ok("garbage".to_string()),
            Err(io::Error::new(io::ErrorKind::Other, "socket")),
        ] {
            let f = fake(Ok((false, NOT_FOUND.into())), list);
            assert!(matches!(liveness(&f, "w"), Liveness::Unknown(_)));
        }
    }

    #[test]
    fn other_get_error_is_unknown() {
        let f = fake(
            Ok((false, r#"{"error":{"code":"socket_error"}}"#.into())),
            Ok(LIST_OTHER.into()),
        );
        assert!(matches!(liveness(&f, "w"), Liveness::Unknown(_)));
    }
}
