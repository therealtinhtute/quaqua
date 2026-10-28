//! Worker report outcome enum and parsing helper.
//!
//! Enforces the report contract defined in `templates/workspace/rules/_all/brief.md`.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// The recognized outcome levels a builder report may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OutcomeLevel {
    Complete,
    NoChanges,
    Blocked,
    Disputed,
    ReopenRequest,
    DependencyRequest,
}

impl OutcomeLevel {
    /// The six outcome levels allowed in a report contract.
    pub const ALL: [OutcomeLevel; 6] = [
        OutcomeLevel::Complete,
        OutcomeLevel::NoChanges,
        OutcomeLevel::Blocked,
        OutcomeLevel::Disputed,
        OutcomeLevel::ReopenRequest,
        OutcomeLevel::DependencyRequest,
    ];

    /// Canonical uppercase token string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "COMPLETE",
            Self::NoChanges => "NO_CHANGES",
            Self::Blocked => "BLOCKED",
            Self::Disputed => "DISPUTED",
            Self::ReopenRequest => "REOPEN_REQUEST",
            Self::DependencyRequest => "DEPENDENCY_REQUEST",
        }
    }

    /// Pipe-joined vocabulary quoted in `rules/_all/brief.md`.
    pub fn vocabulary() -> String {
        Self::ALL
            .iter()
            .map(|o| o.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Comma-joined allowed list for user/worker diagnostics.
    pub fn allowed_list() -> String {
        Self::ALL
            .iter()
            .map(|o| o.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Parses an outcome token case-insensitively from a string.
    /// Handles variants such as "no changes", "NO_CHANGES", "reopen_request", etc.
    pub fn parse_token(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }

        let lower = trimmed.to_ascii_lowercase();
        if lower == "no changes"
            || (lower.starts_with("no changes")
                && !lower.chars().nth(10).is_some_and(|c| c.is_alphanumeric()))
        {
            return Some(Self::NoChanges);
        }

        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        let clean =
            first_word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
        match clean.to_ascii_uppercase().as_str() {
            "COMPLETE" => Some(Self::Complete),
            "NO_CHANGES" | "NO-CHANGES" => Some(Self::NoChanges),
            "BLOCKED" => Some(Self::Blocked),
            "DISPUTED" => Some(Self::Disputed),
            "REOPEN_REQUEST" | "REOPEN-REQUEST" => Some(Self::ReopenRequest),
            "DEPENDENCY_REQUEST" | "DEPENDENCY-REQUEST" => Some(Self::DependencyRequest),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutcomeLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OutcomeLevel {
    type Err = OutcomeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_token(s).ok_or_else(|| OutcomeParseError::Invalid(s.to_string()))
    }
}

/// Errors when parsing an Outcome from report content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeParseError {
    Missing,
    Invalid(String),
}

impl std::fmt::Display for OutcomeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "missing 'Outcome:' line"),
            Self::Invalid(val) => write!(f, "invalid 'Outcome:' value '{val}'"),
        }
    }
}

impl std::error::Error for OutcomeParseError {}

/// Parses the `Outcome:` line from report content.
///
/// Rules:
/// - Must begin at column 1 with `Outcome:` or `outcome:` (no leading whitespace or markdown list marker).
/// - The token is parsed case-insensitively using [`OutcomeLevel::parse_token`].
/// - Returns `Err(OutcomeParseError::Missing)` if no column-1 Outcome line exists.
/// - Returns `Err(OutcomeParseError::Invalid)` if the line exists but the token is unrecognized.
pub fn parse_report_outcome(report_content: &str) -> Result<OutcomeLevel, OutcomeParseError> {
    for line in report_content.lines() {
        if let Some(rest) = line
            .strip_prefix("Outcome:")
            .or_else(|| line.strip_prefix("outcome:"))
        {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                return Err(OutcomeParseError::Invalid(String::new()));
            }
            return OutcomeLevel::parse_token(trimmed)
                .ok_or_else(|| OutcomeParseError::Invalid(trimmed.to_string()));
        }
    }
    Err(OutcomeParseError::Missing)
}
