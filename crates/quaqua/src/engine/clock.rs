//! Production system clock implementation for the Quaqua workflow engine.
//!
//! Provides [`SystemClock`], exposing real-time timestamps (`now_ms`)
//! derived from the system clock since Unix epoch for production workflow execution.

use std::time::{SystemTime, UNIX_EPOCH};

/// Real-time clock backed by [`SystemTime`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl SystemClock {
    /// Creates a new system clock instance.
    pub fn new() -> Self {
        Self
    }

    /// Returns the current system time in milliseconds since the Unix epoch.
    pub fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Returns the current system time as [`SystemTime`].
    pub fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_system_clock_now_ms_positive_and_advancing() {
        let clock = SystemClock::new();
        let t1 = clock.now_ms();
        assert!(t1 > 0, "system clock now_ms must be positive");

        sleep(Duration::from_millis(5));
        let t2 = clock.now_ms();
        assert!(t2 >= t1, "system clock now_ms must advance or stay equal");
    }

    #[test]
    fn test_system_clock_now_system_time() {
        let clock = SystemClock::new();
        let now = clock.now();
        let elapsed = SystemTime::now()
            .duration_since(now)
            .expect("now() must be in past or current");
        assert!(elapsed < Duration::from_secs(5));
    }

    #[test]
    fn test_system_clock_default() {
        let clock = SystemClock::default();
        assert!(clock.now_ms() > 0);
    }
}
