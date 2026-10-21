//! Harness registry: the closed facts about each agent harness quaqua launches
//! (name, effort vocabulary, stop keys, idle detection) in one place, so adding
//! a harness edits this table instead of string lists across the crate.
//!
//! Launch args, auditor and usage CLI calls stay with their callers.

/// An agent harness quaqua can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Opencode,
    Omp,
}

impl Harness {
    pub const ALL: [Harness; 3] = [Harness::Opencode, Harness::Omp, Harness::Claude];

    /// The harness named exactly `s` (team.toml `harness`, herdr `agent`).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Opencode => "opencode",
            Harness::Omp => "omp",
        }
    }

    /// The effort values this harness accepts. opencode effort is a model *variant*
    /// (`opencode --help` lists no `--model`; the interactive TUI reads the model
    /// from config as `provider/model#variant`, and the variant vocabulary is the
    /// `ReasoningEffort` set). omp's is `omp --help` `--thinking`. claude's is
    /// `claude --help` `--effort <level> (low, medium, high, xhigh, max)`.
    pub fn effort_allowed(self, effort: &str) -> bool {
        let allowed: &[&str] = match self {
            Harness::Opencode => &["none", "minimal", "low", "medium", "high", "xhigh", "max"],
            Harness::Omp => &[
                "off", "minimal", "low", "medium", "high", "xhigh", "max", "auto",
            ],
            Harness::Claude => &["low", "medium", "high", "xhigh", "max"],
        };
        allowed.contains(&effort)
    }

    /// Keys that interrupt a busy worker of this harness, in order — never
    /// `ctrl+c` (it kills opencode). `None` = no verified sequence.
    pub fn stop_keys(self) -> Option<&'static [&'static str]> {
        match self {
            Harness::Opencode => Some(&["esc", "esc"]),
            Harness::Omp => Some(&["esc"]),
            Harness::Claude => None,
        }
    }

    /// True when herdr's hook-based idle signal is not trusted for this harness,
    /// so the watchdog uses the longer fallback idle limit.
    pub fn fallback_idle(self) -> bool {
        matches!(self, Harness::Opencode | Harness::Omp)
    }
}

/// True when `s` mentions any fallback-idle harness by name (substring match on
/// a kind or label, as the watchdog callers have always done).
pub fn mentions_fallback_idle(s: &str) -> bool {
    Harness::ALL
        .into_iter()
        .any(|h| h.fallback_idle() && s.contains(h.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_harness() {
        for h in Harness::ALL {
            assert_eq!(Harness::parse(h.as_str()), Some(h));
        }
        assert_eq!(Harness::parse("aider"), None);
        assert_eq!(Harness::parse(""), None);
        assert_eq!(Harness::parse("Claude"), None);
    }

    #[test]
    fn effort_sets_per_harness() {
        assert!(Harness::Opencode.effort_allowed("none"));
        assert!(!Harness::Opencode.effort_allowed("auto"));
        assert!(Harness::Omp.effort_allowed("auto"));
        assert!(Harness::Omp.effort_allowed("off"));
        assert!(Harness::Claude.effort_allowed("xhigh"));
        assert!(!Harness::Claude.effort_allowed("minimal"));
    }

    #[test]
    fn fallback_idle_is_opencode_and_omp() {
        assert!(Harness::Opencode.fallback_idle());
        assert!(Harness::Omp.fallback_idle());
        assert!(!Harness::Claude.fallback_idle());
        assert!(mentions_fallback_idle("opencode"));
        assert!(mentions_fallback_idle("x-omp-y"));
        assert!(!mentions_fallback_idle("tieude"));
        assert!(!mentions_fallback_idle("claude"));
    }
}
