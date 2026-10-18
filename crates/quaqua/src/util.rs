use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Read timestamp via shared fake-clock hook (PROMPT_WORKER_CLOCK_FILE), else system time.
pub fn now_sec() -> u64 {
    if let Ok(clock_file) = std::env::var("PROMPT_WORKER_CLOCK_FILE") {
        if let Ok(c) = std::fs::read_to_string(&clock_file) {
            if let Ok(v) = c.trim().parse::<u64>() {
                return v;
            }
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Check if a process with `pid_str` is currently alive.
/// Trims whitespace, validates pid is a positive integer, and runs `kill -0 <pid>`.
pub fn is_pid_alive(pid_str: &str) -> bool {
    let trimmed = pid_str.trim();
    if let Ok(pid) = trimmed.parse::<i32>() {
        if pid > 0 {
            let res = Command::new("kill").args(["-0", trimmed]).output();
            return res.map(|o| o.status.success()).unwrap_or(false);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_sec_real() {
        assert!(now_sec() > 0);
    }

    #[test]
    fn test_is_pid_alive_invalid() {
        assert!(!is_pid_alive(""));
        assert!(!is_pid_alive("   "));
        assert!(!is_pid_alive("abc"));
        assert!(!is_pid_alive("-1"));
        assert!(!is_pid_alive("0"));
    }

    #[test]
    fn test_is_pid_alive_current() {
        let my_pid = std::process::id().to_string();
        assert!(is_pid_alive(&my_pid));
    }
}
