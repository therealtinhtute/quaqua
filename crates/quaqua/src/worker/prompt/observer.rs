//! Observer for worker pane text, status, liveness, and git diff.

use crate::git::GitOps;
use crate::hash::md5_hex;
use crate::herdr::HerdrOps;
use crate::util::is_pid_alive;
use crate::worker::cleanup::default_runs_dir;
use serde_json::Value;
use std::path::Path;

pub struct Observation {
    pub status: String,
    pub ctx: String,
    pub last_tool: String,
    pub diffstat: String,
    pub pane_text: String,
    pub fp: String,
    pub diff_hash: String,
    pub watcher_pane: String,
    pub hook_active: bool,
    pub harness: String,
    pub session_id: String,
    pub cwd: String,
}

pub fn question_seen(pane_text: &str) -> bool {
    let mut clean_lines: Vec<&str> = Vec::new();
    for line in pane_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_decorative = trimmed.chars().all(|c| {
            "─│|═━┈┉┄┅┌┐└┘├┤┬┴┼╭╮╯╰ 0123456789%·".contains(c)
        });
        if is_decorative
            || trimmed.contains('👾')
            || trimmed.contains('✦')
            || trimmed.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c))
        {
            continue;
        }
        clean_lines.push(trimmed);
    }

    let tail_count = clean_lines.len().min(3);
    let tail = &clean_lines[clean_lines.len() - tail_count..];

    for line in tail {
        let l = line.to_lowercase();
        if l.contains("type something")
            || l.contains("esc to abandon")
            || l.contains("esc to cancel")
            || l.contains("esc to undo")
            || l.contains("do you want to")
            || l.contains("needs approval")
            || l.contains("needs permission")
            || l.contains("requires approval")
            || l.contains("requires permission")
            || l.contains("[y/n]")
        {
            return true;
        }
        // [y/N] or y/n bounded by non-alphabetic
        if let Some(idx) = l.find("y/n") {
            let before = if idx == 0 { ' ' } else { l.chars().nth(idx - 1).unwrap_or(' ') };
            let after = l.chars().nth(idx + 3).unwrap_or(' ');
            if !before.is_alphabetic() && !after.is_alphabetic() {
                return true;
            }
        }
    }
    false
}

pub fn is_provider_timeout(pane_text: &str) -> bool {
    let clean_lines: Vec<&str> = pane_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let tail_count = clean_lines.len().min(5);
    let tail = &clean_lines[clean_lines.len().saturating_sub(tail_count)..];
    for line in tail {
        let lower = line.to_lowercase();
        if lower.contains("timed out waiting for response headers")
            || lower.contains("timed out waiting for response data")
        {
            return true;
        }
    }
    false
}

pub fn lead_waiting(runs_dir: &Path, name: &str, watcher_pane: &str) -> bool {
    if !name.ends_with("-lead") {
        return false;
    }
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.starts_with("pipeline-") && fname.ends_with(".state") {
                if let Ok(c) = std::fs::read_to_string(&path) {
                    if !c.lines().any(|l| l.trim() == "finished=0") {
                        continue;
                    }
                    let mut p_lead = "";
                    let mut p_pane = "";
                    let mut p_pid = "";
                    for line in c.lines() {
                        let l = line.trim();
                        if let Some(v) = l.strip_prefix("lead=") {
                            p_lead = v;
                        } else if let Some(v) = l.strip_prefix("lead_pane=") {
                            p_pane = v;
                        } else if let Some(v) = l.strip_prefix("pipeline_pid=") {
                            p_pid = v;
                        }
                    }
                    if !p_pid.is_empty() {
                        let is_alive = is_pid_alive(p_pid);
                        if !is_alive {
                            continue;
                        }
                        if p_lead == name || (!watcher_pane.is_empty() && p_pane == watcher_pane) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

pub fn report_stat(path: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let len = md.len();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((len, md.mtime() as u64))
    }
    #[cfg(not(unix))]
    {
        Some((len, 0))
    }
}

pub fn report_ready(path: &Path) -> bool {
    if let Some((len1, mtime1)) = report_stat(path) {
        if len1 > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if let Some((len2, mtime2)) = report_stat(path) {
                return len1 == len2 && mtime1 == mtime2;
            }
        }
    }
    false
}

pub fn observe<G: GitOps, H: HerdrOps>(herdr: &H, git: &G, name: &str) -> Option<Observation> {
    let mut status = "unknown".to_string();
    let mut ctx = "?".to_string();
    let mut last_tool = String::new();
    let mut diffstat = String::new();
    let mut watcher_pane = String::new();
    let mut cwd = String::new();
    let mut hook_active = false;
    let mut harness = String::new();
    let mut session_id = String::new();
    let (_ok, get_info) = herdr.agent_get(name).unwrap_or((false, String::new()));
    if let Ok(json) = serde_json::from_str::<Value>(&get_info) {
        if let Some(agent) = json.get("result").and_then(|r| r.get("agent")) {
            status = agent.get("agent_status").and_then(|s| s.as_str()).unwrap_or("unknown").to_string();
            cwd = agent.get("cwd").and_then(|s| s.as_str()).unwrap_or("").to_string();
            watcher_pane = agent.get("pane_id").and_then(|s| s.as_str()).unwrap_or("").to_string();
            hook_active = agent.get("screen_detection_skipped").and_then(|b| b.as_bool()).unwrap_or(false);
            harness = agent.get("agent").and_then(|s| s.as_str()).unwrap_or("").to_string();
            if let Some(session) = agent.get("agent_session") {
                session_id = session.get("value").and_then(|s| s.as_str()).unwrap_or("").to_string();
            }
        } else if let Some(err) = json.get("error") {
            if err.get("code").and_then(|c| c.as_str()) == Some("agent_not_found") {
                status = "gone".to_string();
            }
        }
    }
    if status == "unknown" && get_info.contains("agent_not_found") {
        status = "gone".to_string();
    }
    if harness.is_empty() {
        if let Ok(c) = std::fs::read_to_string(default_runs_dir().join(format!("{name}.srt.json"))) {
            if let Ok(j) = serde_json::from_str::<Value>(&c) {
                if let Some(h) = j.get("running_harness").and_then(|s| s.as_str()).or_else(|| j.get("harness").and_then(|s| s.as_str())) {
                    harness = h.to_string();
                }
            }
        }
        if harness.is_empty() {
            if name.starts_with("tieude-") {
                harness = "opencode".to_string();
            } else if name.starts_with("sude-") || name.starts_with("detu-") {
                harness = "omp".to_string();
            }
        }
    }

    let mut pane_text = herdr
        .agent_read(name, "recent-unwrapped", 80)
        .unwrap_or_default();
    if pane_text.trim().is_empty() || pane_text.contains("{\"error\"") {
        pane_text = herdr
            .agent_read(name, "visible", 80)
            .unwrap_or_default();
    }
    if pane_text.contains("{\"error\"") {
        pane_text.clear();
    }

    if get_info.is_empty() && pane_text.is_empty() {
        return None;
    }

    // Body fingerprint: filter spinner/status rows, collapse digits to 'N'
    let mut body_lines = Vec::new();
    for line in pane_text.lines() {
        let t = line.trim();
        let is_sep = t.chars().all(|c| "─│| ".contains(c));
        if is_sep || t.contains('👾') || t.contains('✦') || t.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c)) {
            continue;
        }
        let mut collapsed = String::new();
        let mut in_digits = false;
        for c in line.chars() {
            if c.is_ascii_digit() {
                if !in_digits {
                    collapsed.push('N');
                    in_digits = true;
                }
            } else {
                in_digits = false;
                collapsed.push(c);
            }
        }
        body_lines.push(collapsed);
    }
    let body_text = body_lines.join("\n");
    let body_hash = md5_hex(body_text.as_bytes());

    for line in pane_text.lines() {
        if let Some(idx) = line.find('%') {
            let mut num = String::new();
            for c in line[..idx].chars().rev() {
                if c.is_ascii_digit() {
                    num.push(c);
                } else {
                    break;
                }
            }
            if !num.is_empty() {
                let n: String = num.chars().rev().collect();
                ctx = format!("{n}%");
                break;
            }
        }
    }

    for line in pane_text.lines().rev() {
        if line.contains('▸') || line.contains('⏺') {
            let t = line.trim_start();
            last_tool = t.chars().take(90).collect();
            break;
        }
    }

    let mut dh = String::new();
    if !cwd.is_empty() && git.is_git_dir(Path::new(&cwd)) {
        let (d_hash, d_stat) = git.diff_and_status(Path::new(&cwd));
        dh = d_hash;
        diffstat = d_stat;
    }

    let fp = format!("{body_hash}|{dh}");

    Some(Observation {
        status,
        ctx,
        last_tool,
        diffstat,
        pane_text,
        fp,
        diff_hash: dh,
        watcher_pane,
        hook_active,
        harness,
        session_id,
        cwd,
    })
}
