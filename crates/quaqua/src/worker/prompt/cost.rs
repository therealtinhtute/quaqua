//! Worker cost tracking and budget resolution via opencode API and sqlite database.

use crate::engine::activity::{ActivityResult, CostResult, ReportFileResult};
use crate::state::WorkerMetadata;
use crate::worker::cleanup::default_runs_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Token counts for a worker session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    #[serde(rename = "in")]
    pub in_tokens: u64,
    #[serde(rename = "out")]
    pub out_tokens: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.in_tokens += other.in_tokens;
        self.out_tokens += other.out_tokens;
        self.reasoning += other.reasoning;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// Spend results (dollars, tokens, source, model) for a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spend {
    pub usd: Option<f64>,
    pub tokens: Option<Tokens>,
    pub spend_src: String,
    pub model: Option<String>,
}

/// Parses an ISO-8601/RFC-3339 timestamp string into epoch milliseconds.
pub fn parse_iso_timestamp_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date_part, time_and_tz) = s.split_once('T')?;
    let mut date_segs = date_part.split('-');
    let y: i32 = date_segs.next()?.parse().ok()?;
    let m: u32 = date_segs.next()?.parse().ok()?;
    let d: u32 = date_segs.next()?.parse().ok()?;

    let (time_part, tz_offset_ms) = if let Some(stripped) = time_and_tz.strip_suffix('Z') {
        (stripped, 0i64)
    } else if let Some(idx) = time_and_tz.rfind(['+', '-']) {
        if idx > 0 && time_and_tz.as_bytes()[idx - 1] != b'T' {
            let (time_str, tz_str) = time_and_tz.split_at(idx);
            let sign = if tz_str.starts_with('+') { 1i64 } else { -1i64 };
            let tz_digits = &tz_str[1..];
            let (tz_h, tz_m) = if let Some((h_str, m_str)) = tz_digits.split_once(':') {
                (h_str.parse::<i64>().ok()?, m_str.parse::<i64>().ok()?)
            } else {
                (tz_digits.parse::<i64>().ok()?, 0i64)
            };
            (time_str, sign * (tz_h * 3600 + tz_m * 60) * 1000)
        } else {
            (time_and_tz, 0i64)
        }
    } else {
        (time_and_tz, 0i64)
    };

    let (time_main, frac) = time_part.split_once('.').unwrap_or((time_part, "0"));
    let mut time_segs = time_main.split(':');
    let h: i64 = time_segs.next()?.parse().ok()?;
    let min: i64 = time_segs.next()?.parse().ok()?;
    let sec: i64 = time_segs.next()?.parse().ok()?;

    let mut millis = 0i64;
    let frac = frac.trim();
    if !frac.is_empty() {
        let frac_3: String = frac.chars().take(3).collect();
        if let Ok(v) = frac_3.parse::<i64>() {
            millis = match frac_3.len() {
                1 => v * 100,
                2 => v * 10,
                _ => v,
            };
        }
    }

    let days = crate::usage::days_from_civil(y, m, d);
    let total_ms = days * 86_400_000 + (h * 3600 + min * 60 + sec) * 1000 + millis - tz_offset_ms;
    if total_ms >= 0 {
        Some(total_ms as u64)
    } else {
        None
    }
}

pub fn brief_cost_budget(text: &str) -> Option<f64> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("cost budget:") {
            let rest = line[idx + 12..].trim();
            let num_str = rest.strip_prefix('$').unwrap_or(rest).trim();
            let num_part: String = num_str
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(val) = num_part.parse::<f64>() {
                if val > 0.0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

pub fn resolve_cost_budget(task: &str) -> Option<f64> {
    if let Ok(env_val) = std::env::var("PROMPT_WORKER_COST_BUDGET") {
        let trimmed = env_val.trim();
        let num_str = trimmed.strip_prefix('$').unwrap_or(trimmed).trim();
        if let Ok(val) = num_str.parse::<f64>() {
            if val > 0.0 {
                return Some(val);
            }
        }
    }
    brief_cost_budget(task)
}

/// True when this worker reported spend through omp session transcripts.
pub fn is_omp_worker(harness: &str, name: &str) -> bool {
    harness == "omp"
        || (harness.is_empty() && (name.starts_with("sude-") || name.starts_with("detu-")))
}

/// True when this worker reported spend through claude session transcripts.
pub fn is_claude_worker(harness: &str, name: &str) -> bool {
    harness == "claude"
        || (harness.is_empty() && (name.starts_with("ni-") || name.starts_with("truonglao-")))
}

/// True when this worker reported spend through opencode sqlite database.
pub fn is_opencode_worker(harness: &str, name: &str) -> bool {
    harness == "opencode"
        || (harness.is_empty() && (name.starts_with("tieude-") || name.starts_with("suhuynh-")))
}

/// omp's session directory name for a worktree path: `$HOME` is stripped, then
/// every `/` becomes `-` (the leading dash is kept).
///
/// `/home/me/.wt/sude-x` -> `-.wt-sude-x`, `/tmp/x` -> `-tmp-x`.
pub fn omp_session_dir_name(cwd: &str, home: &str) -> String {
    let path = Path::new(cwd);
    let rel = if home.is_empty() {
        path
    } else {
        path.strip_prefix(home).unwrap_or(path)
    };
    let rel = rel.to_string_lossy();
    // `strip_prefix` drops the separator; omp keeps it as the leading dash.
    let rel = if rel.starts_with('/') || rel.is_empty() {
        rel.into_owned()
    } else {
        format!("/{rel}")
    };
    rel.replace('/', "-")
}

/// Reads one `cost.total` from a session line, preferring the known
/// `message.usage.cost.total` location and falling back to the first
/// `cost.total` object if omp moves it.
fn session_line_cost(v: &serde_json::Value) -> Option<f64> {
    if let Some(c) = v
        .pointer("/message/usage/cost/total")
        .and_then(|c| c.as_f64())
    {
        return Some(c);
    }
    fn find(v: &serde_json::Value) -> Option<f64> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(c) = map
                    .get("cost")
                    .and_then(|c| c.get("total"))
                    .and_then(|t| t.as_f64())
                {
                    return Some(c);
                }
                map.values().find_map(find)
            }
            serde_json::Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    find(v)
}

/// Reads omp spend from session transcripts in `root`.
pub fn omp_spend_in(root: &Path, cwd: &str, home: &str, since_ms: Option<u64>) -> Option<Spend> {
    use std::io::BufRead;
    let dir = root.join(omp_session_dir_name(cwd, home));
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut total_cost = 0.0;
    let mut tokens = Tokens::default();
    let mut found = false;
    let mut last_model: Option<String> = None;

    for entry in entries.flatten() {
        if entry.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if !line.contains("\"cost\"") && !line.contains("\"usage\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };

            if let Some(fence) = since_ms {
                if let Some(ts_str) = v.get("timestamp").and_then(|t| t.as_str()) {
                    if let Some(ts_ms) = parse_iso_timestamp_ms(ts_str) {
                        if ts_ms < fence {
                            continue;
                        }
                    }
                }
            }

            if let Some(c) = session_line_cost(&v) {
                total_cost += c;
                found = true;
            }

            let usage = v.pointer("/message/usage").or_else(|| v.get("usage"));
            if let Some(u) = usage {
                let tin = u.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
                let tout = u.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
                let trsn = u
                    .get("reasoningTokens")
                    .or_else(|| u.get("reasoning"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let tcr = u.get("cacheRead").and_then(|x| x.as_u64()).unwrap_or(0);
                let tcw = u.get("cacheWrite").and_then(|x| x.as_u64()).unwrap_or(0);

                tokens.in_tokens += tin;
                tokens.out_tokens += tout;
                tokens.reasoning += trsn;
                tokens.cache_read += tcr;
                tokens.cache_write += tcw;
                found = true;
            }

            let model = v
                .pointer("/message/model")
                .or_else(|| v.get("model"))
                .and_then(|m| m.as_str());
            if let Some(m) = model {
                if !m.is_empty() && m != "<synthetic>" {
                    last_model = Some(m.to_string());
                }
            }
        }
    }

    if found {
        Some(Spend {
            usd: Some(total_cost),
            tokens: Some(tokens),
            spend_src: "omp-session".to_string(),
            model: last_model,
        })
    } else {
        None
    }
}

/// Sums `cost.total` over every `.jsonl` line of omp's session directory that matches `cwd`.
pub fn omp_cost_in(root: &Path, cwd: &str, home: &str) -> Option<f64> {
    omp_spend_in(root, cwd, home, None).and_then(|s| s.usd)
}

/// Reads claude spend from transcripts under `projects_root`, deduplicated by `message.id`.
pub fn claude_spend_in(
    projects_root: &Path,
    session_id: &str,
    cwd: &str,
    since_ms: Option<u64>,
) -> Option<Spend> {
    use std::io::BufRead;

    let mut candidate_files = Vec::new();
    if !cwd.is_empty() {
        let slug: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let project_dir = projects_root.join(&slug);
        if !session_id.is_empty() {
            let direct = project_dir.join(format!("{session_id}.jsonl"));
            if direct.is_file() {
                candidate_files.push(direct);
            }
        }
        if candidate_files.is_empty() && project_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&project_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        candidate_files.push(p);
                    }
                }
            }
        }
    }

    if candidate_files.is_empty() && !session_id.is_empty() {
        if let Ok(entries) = std::fs::read_dir(projects_root) {
            for entry in entries.flatten() {
                let candidate = entry.path().join(format!("{session_id}.jsonl"));
                if candidate.is_file() {
                    candidate_files.push(candidate);
                }
            }
        }
    }

    if candidate_files.is_empty() {
        return None;
    }

    // Deduplicate by message.id
    let mut dedup_map: HashMap<String, (Tokens, Option<String>)> = HashMap::new();

    for file_path in candidate_files {
        let Ok(file) = std::fs::File::open(&file_path) else {
            continue;
        };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if !line.contains("\"usage\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };

            if let Some(fence) = since_ms {
                if let Some(ts_str) = v.get("timestamp").and_then(|t| t.as_str()) {
                    if let Some(ts_ms) = parse_iso_timestamp_ms(ts_str) {
                        if ts_ms < fence {
                            continue;
                        }
                    }
                }
            }

            let Some(msg) = v.get("message") else { continue };
            let Some(msg_id) = msg.get("id").and_then(|id| id.as_str()) else { continue };
            let Some(u) = msg.get("usage") else { continue };

            let in_tok = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
            let out_tok = u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
            let cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cache_write = u
                .get("cache_creation_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let reasoning = u
                .get("output_tokens_details")
                .and_then(|d| d.get("thinking_tokens"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            let model = msg.get("model").and_then(|m| m.as_str()).and_then(|m| {
                if m.is_empty() || m == "<synthetic>" {
                    None
                } else {
                    Some(m.to_string())
                }
            });

            dedup_map.insert(
                msg_id.to_string(),
                (
                    Tokens {
                        in_tokens: in_tok,
                        out_tokens: out_tok,
                        reasoning,
                        cache_read,
                        cache_write,
                    },
                    model,
                ),
            );
        }
    }

    if dedup_map.is_empty() {
        return None;
    }

    let mut total_tokens = Tokens::default();
    let mut last_model: Option<String> = None;
    for (tok, model) in dedup_map.values() {
        total_tokens.add(tok);
        if model.is_some() {
            last_model = model.clone();
        }
    }

    Some(Spend {
        usd: None,
        tokens: Some(total_tokens),
        spend_src: "claude-tokens".to_string(),
        model: last_model,
    })
}

/// Reads opencode spend from an sqlite database file, summing over every session in `cwd`.
pub fn opencode_spend_in(
    db_path: &Path,
    session_id: &str,
    cwd: &str,
    since_ms: Option<u64>,
) -> Option<Spend> {
    if !db_path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let (query, param): (&str, &str) = if !cwd.is_empty() {
        (
            "SELECT cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, \
             tokens_cache_write, model, time_created \
             FROM session_v2 WHERE directory = ?1 ORDER BY time_created ASC;",
            cwd,
        )
    } else if !session_id.is_empty() {
        (
            "SELECT cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, \
             tokens_cache_write, model, time_created \
             FROM session_v2 WHERE id = ?1 ORDER BY time_created ASC;",
            session_id,
        )
    } else {
        return None;
    };

    let mut stmt = conn.prepare(query).ok()?;
    let mut rows = stmt.query(rusqlite::params![param]).ok()?;

    let mut total_cost = 0.0;
    let mut tokens = Tokens::default();
    let mut last_model: Option<String> = None;
    let mut found = false;

    while let Ok(Some(row)) = rows.next() {
        let time_created: i64 = row.get(7).unwrap_or(0);
        if let Some(fence) = since_ms {
            if time_created > 0 && (time_created as u64) < fence {
                continue;
            }
        }

        let cost: f64 = row.get(0).unwrap_or(0.0);
        let tin: i64 = row.get(1).unwrap_or(0);
        let tout: i64 = row.get(2).unwrap_or(0);
        let trsn: i64 = row.get(3).unwrap_or(0);
        let tcr: i64 = row.get(4).unwrap_or(0);
        let tcw: i64 = row.get(5).unwrap_or(0);
        let raw_model: Option<String> = row.get(6).ok();

        total_cost += cost;
        tokens.in_tokens += tin.max(0) as u64;
        tokens.out_tokens += tout.max(0) as u64;
        tokens.reasoning += trsn.max(0) as u64;
        tokens.cache_read += tcr.max(0) as u64;
        tokens.cache_write += tcw.max(0) as u64;

        if let Some(m_str) = raw_model {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&m_str) {
                if let Some(id_str) = v.get("id").and_then(|id| id.as_str()) {
                    last_model = Some(id_str.to_string());
                } else {
                    last_model = Some(m_str);
                }
            } else if !m_str.is_empty() {
                last_model = Some(m_str);
            }
        }
        found = true;
    }

    if found {
        Some(Spend {
            usd: Some(total_cost),
            tokens: Some(tokens),
            spend_src: "opencode-db".to_string(),
            model: last_model,
        })
    } else {
        None
    }
}

fn opencode_db_candidates(name: &str) -> Vec<PathBuf> {
    let mut db_candidates = Vec::new();
    if let Ok(env_db) = std::env::var("QUAQUA_COST_DB_PATH") {
        if !env_db.trim().is_empty() {
            db_candidates.push(PathBuf::from(env_db));
        }
    }
    let state_db = default_runs_dir()
        .join(".opencode-state")
        .join(name)
        .join("opencode")
        .join("opencode.db");
    if state_db.is_file() {
        db_candidates.push(state_db);
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_db = PathBuf::from(home).join(".local/share/opencode/opencode.db");
        if home_db.is_file() {
            db_candidates.push(home_db);
        }
    }
    db_candidates
}

/// Reads spend for a worker across opencode, omp, or claude.
pub fn read_worker_spend(
    name: &str,
    session_id: &str,
    cwd: &str,
    harness: &str,
    since_ms: Option<u64>,
) -> Spend {
    if let Ok(cost_file) = std::env::var("PROMPT_WORKER_COST_FILE") {
        if let Ok(content) = std::fs::read_to_string(&cost_file) {
            if let Ok(val) = content.trim().parse::<f64>() {
                return Spend {
                    usd: Some(val),
                    tokens: None,
                    spend_src: "cost-file".to_string(),
                    model: None,
                };
            }
        }
        return Spend {
            usd: None,
            tokens: None,
            spend_src: "cost-file-invalid".to_string(),
            model: None,
        };
    }

    if is_omp_worker(harness, name) {
        let home = std::env::var("HOME").unwrap_or_default();
        let root = match std::env::var("OMP_SESSIONS_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => {
                if home.is_empty() {
                    PathBuf::new()
                } else {
                    Path::new(&home).join(".omp/agent/sessions")
                }
            }
        };
        if let Some(spend) = omp_spend_in(&root, cwd, &home, since_ms) {
            return spend;
        }
        return Spend {
            usd: None,
            tokens: None,
            spend_src: "omp-session-not-found".to_string(),
            model: None,
        };
    }

    if is_claude_worker(harness, name) {
        let home = std::env::var("HOME").unwrap_or_default();
        let root = match std::env::var("CLAUDE_PROJECTS_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => {
                if home.is_empty() {
                    PathBuf::new()
                } else {
                    Path::new(&home).join(".claude/projects")
                }
            }
        };
        if let Some(spend) = claude_spend_in(&root, session_id, cwd, since_ms) {
            return spend;
        }
        return Spend {
            usd: None,
            tokens: None,
            spend_src: "claude-tokens-not-found".to_string(),
            model: None,
        };
    }

    if is_opencode_worker(harness, name) {
        let candidates = opencode_db_candidates(name);
        for db_path in candidates {
            if let Some(spend) = opencode_spend_in(&db_path, session_id, cwd, since_ms) {
                return spend;
            }
        }
        return Spend {
            usd: None,
            tokens: None,
            spend_src: "opencode-db-not-found".to_string(),
            model: None,
        };
    }

    Spend {
        usd: None,
        tokens: None,
        spend_src: if harness.is_empty() {
            "unknown-harness".to_string()
        } else {
            format!("unsupported-harness:{harness}")
        },
        model: None,
    }
}

pub fn read_worker_cost(name: &str, session_id: &str, cwd: &str, harness: &str) -> Option<f64> {
    read_worker_spend(name, session_id, cwd, harness, None).usd
}

/// Milliseconds since the epoch from a file's mtime.
fn file_mtime_ms(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Last write time of an opencode session store row.
///
/// Uses `session_message.time_updated` for the session id (the shared db-wal
/// mtime is not a per-session signal).
fn opencode_session_mtime(session_id: &str) -> Option<u64> {
    let mut db_candidates = Vec::new();
    if let Ok(env_db) = std::env::var("QUAQUA_COST_DB_PATH") {
        if !env_db.is_empty() {
            db_candidates.push(std::path::PathBuf::from(env_db));
        }
    }
    let state_dir = default_runs_dir().join(".opencode-state");
    if let Ok(entries) = std::fs::read_dir(&state_dir) {
        for entry in entries.flatten() {
            let p = entry.path().join("opencode").join("opencode.db");
            if p.is_file() {
                db_candidates.push(p);
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        db_candidates.push(std::path::PathBuf::from(home).join(".local/share/opencode/opencode.db"));
    }
    for db_path in db_candidates {
        if !db_path.is_file() {
            continue;
        }
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        let query =
            "SELECT COALESCE(max(time_updated), 0) FROM session_message WHERE session_id = ?1";
        let Ok(mut stmt) = conn.prepare(query) else {
            continue;
        };
        if let Ok(value) = stmt.query_row(rusqlite::params![session_id], |row| {
            row.get::<_, i64>(0)
        }) {
            if value > 0 {
                return Some(value as u64);
            }
        }
    }
    None
}

/// claude's per-project session transcript: `~/.claude/projects/<slug>/<id>.jsonl`.
fn claude_session_mtime(session_id: &str, cwd: &str) -> Option<u64> {
    let home = std::env::var("HOME").ok()?;
    let projects = Path::new(&home).join(".claude/projects");
    if !cwd.is_empty() {
        let slug: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let direct = projects.join(slug).join(format!("{session_id}.jsonl"));
        if let Some(ms) = file_mtime_ms(&direct) {
            return Some(ms);
        }
    }
    // A cwd that does not map cleanly (symlink, home elsewhere): find the id.
    let entries = std::fs::read_dir(&projects).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join(format!("{session_id}.jsonl"));
        if let Some(ms) = file_mtime_ms(&candidate) {
            return Some(ms);
        }
    }
    None
}

/// Last write time of the harness session store. This is the glyph-free
/// progress signal for workers whose worktree never moves (reviewers).
pub fn session_store_mtime(
    harness: &str,
    session_kind: &str,
    session_value: &str,
    cwd: &str,
) -> Option<u64> {
    if session_value.is_empty() {
        return None;
    }
    if session_kind == "path" {
        return file_mtime_ms(Path::new(session_value));
    }
    match harness {
        "opencode" => opencode_session_mtime(session_value),
        "claude" => claude_session_mtime(session_value, cwd),
        "omp" => {
            let home = std::env::var("HOME").ok()?;
            let root = match std::env::var("OMP_SESSIONS_DIR") {
                Ok(dir) if !dir.trim().is_empty() => std::path::PathBuf::from(dir),
                _ => Path::new(&home).join(".omp/agent/sessions"),
            };
            file_mtime_ms(&root.join(omp_session_dir_name(cwd, &home)).join(session_value))
        }
        _ => file_mtime_ms(Path::new(session_value)),
    }
}

/// Tracks cost budget warnings (80% and 100%) during watchdog execution.
pub struct CostTracker {
    pub cost_cap: Option<f64>,
    pub cost80_warned: bool,
    pub cost100_warned: bool,
}

impl CostTracker {
    pub fn new(runs_dir: &Path, name: &str, cost_cap: Option<f64>) -> Self {
        Self {
            cost_cap,
            cost80_warned: WorkerMetadata::has_cost80(runs_dir, name),
            cost100_warned: WorkerMetadata::has_cost100(runs_dir, name),
        }
    }

    pub fn check(&mut self, runs_dir: &Path, name: &str, session_id: &str, cwd: &str, harness: &str) {
        if let Some(budget_val) = self.cost_cap {
            if !self.cost100_warned || !self.cost80_warned {
                if let Some(current_cost) = read_worker_cost(name, session_id, cwd, harness) {
                    if !self.cost80_warned && current_cost >= budget_val * 0.8 {
                        let _ = WorkerMetadata::record_cost80(runs_dir, name, current_cost);
                        self.cost80_warned = true;
                        eprintln!("WATCHDOG cost 80% | name={name} cost=${current_cost:.4} budget=${budget_val:.4}");
                    }
                    if !self.cost100_warned && current_cost >= budget_val {
                        let _ = WorkerMetadata::record_cost100(runs_dir, name, current_cost);
                        self.cost100_warned = true;
                        eprintln!("WATCHDOG cost 100% | name={name} cost=${current_cost:.4} budget=${budget_val:.4}");
                    }
                }
            }
        }
    }
}

/// Budget warnings emitted during worker execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostWarning {
    /// Worker spend has reached or exceeded 80% of budget.
    Cost80,
    /// Worker spend has reached or exceeded 100% of budget.
    Cost100,
}

/// Tracks budget warning emissions for deterministic workflow state machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CostState {
    pub warned_80: bool,
    pub warned_100: bool,
}

impl CostState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Pure helper: checks spend against budget and updates cost state.
///
/// Returns `Some(CostWarning::Cost80)` when `spend >= budget * 0.8` (once).
/// Returns `Some(CostWarning::Cost100)` when `spend >= budget` (once).
///
/// If `spend >= budget` triggers first (e.g. fast jump from <80% straight past 100%),
/// it emits `Cost100` and marks both `warned_80` and `warned_100` to prevent redundant
/// late 80% warnings.
pub fn cost_warning(state: &mut CostState, spend: f64, budget: f64) -> Option<CostWarning> {
    if budget <= 0.0 || !budget.is_finite() || spend.is_nan() {
        return None;
    }
    const EPSILON: f64 = 1e-9;
    if !state.warned_100 && spend >= budget - EPSILON {
        state.warned_80 = true;
        state.warned_100 = true;
        return Some(CostWarning::Cost100);
    }
    if !state.warned_80 && spend >= (budget * 0.8) - EPSILON {
        state.warned_80 = true;
        return Some(CostWarning::Cost80);
    }
    None
}

/// Evaluates a frozen engine `CostResult` against budget.
pub fn cost_warning_from_result(
    state: &mut CostState,
    result: &CostResult,
    budget: f64,
) -> Option<CostWarning> {
    match result {
        CostResult::Spend(spend) => cost_warning(state, *spend, budget),
        CostResult::BudgetExceeded(exceeded) => {
            if *exceeded && !state.warned_100 {
                state.warned_80 = true;
                state.warned_100 = true;
                Some(CostWarning::Cost100)
            } else {
                None
            }
        }
    }
}

/// Evaluates a top-level engine `ActivityResult` against budget.
pub fn cost_warning_from_activity(
    state: &mut CostState,
    result: &ActivityResult,
    budget: f64,
) -> Option<CostWarning> {
    match result {
        ActivityResult::Cost(cost_res) => cost_warning_from_result(state, cost_res, budget),
        _ => None,
    }
}

/// Recorded sample of report file metadata for stability checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportSample {
    pub mtime_ms: u64,
}

impl ReportSample {
    pub fn new(mtime_ms: u64) -> Self {
        Self { mtime_ms }
    }
}

/// Report file watchdog status decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportState {
    /// Report file does not exist, mtime is 0, mtime is unstable, or content is empty.
    Pending,
    /// Report file is stable and contains valid required headings.
    Ready,
    /// Report content exists and is stable, but is missing required headings.
    Malformed(String),
}

/// Checks whether builder report content contains a valid `## Lessons` section.
pub fn is_builder_report_valid(content: &str) -> bool {
    crate::worker::prompt::watchdog::validate_report_content(content, false).is_ok()
}

/// Checks whether reviewer report content contains a valid `Verdict:` line.
pub fn is_reviewer_report_valid(content: &str) -> bool {
    crate::worker::prompt::watchdog::validate_report_content(content, true).is_ok()
}

/// Pure helper: evaluates report state using builder validation (`## Lessons`).
pub fn report_state(
    prev_sample: Option<ReportSample>,
    next_result: &ReportFileResult,
    content: Option<&str>,
) -> ReportState {
    report_state_validated(prev_sample, next_result, content, false)
}

/// Configurable report state evaluation supporting builder and reviewer validation.
pub fn report_state_validated(
    prev_sample: Option<ReportSample>,
    next_result: &ReportFileResult,
    content: Option<&str>,
    expect_verdict: bool,
) -> ReportState {
    let validate = |c: &str| -> ReportState {
        if c.trim().is_empty() {
            ReportState::Pending
        } else {
            match crate::worker::prompt::watchdog::validate_report_content(c, expect_verdict) {
                Ok(()) => ReportState::Ready,
                Err(missing) => ReportState::Malformed(format!("missing {missing}")),
            }
        }
    };

    match next_result {
        ReportFileResult::Exists(false) => ReportState::Pending,
        ReportFileResult::Exists(true) => match content {
            Some(c) => validate(c),
            None => ReportState::Pending,
        },
        ReportFileResult::MtimeMs(mtime) => {
            if *mtime == 0 {
                return ReportState::Pending;
            }
            let is_stable = match prev_sample {
                Some(prev) => prev.mtime_ms > 0 && prev.mtime_ms == *mtime,
                None => false,
            };
            if !is_stable {
                return ReportState::Pending;
            }
            match content {
                Some(c) => validate(c),
                None => ReportState::Pending,
            }
        }
        ReportFileResult::Content(c) => validate(c),
        ReportFileResult::Written => ReportState::Pending,
    }
}

/// Evaluates report state from a top-level engine `ActivityResult`.
pub fn report_state_from_activity(
    prev_sample: Option<ReportSample>,
    activity_result: &ActivityResult,
    content: Option<&str>,
) -> Option<ReportState> {
    match activity_result {
        ActivityResult::ReportFile(rf) => Some(report_state(prev_sample, rf, content)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::activity::{ActivityResult, CostResult, ReportFileResult};

    #[test]
    fn test_cost_warning_below_threshold() {
        let mut state = CostState::new();
        assert_eq!(cost_warning(&mut state, 1.5, 2.0), None);
        assert!(!state.warned_80);
        assert!(!state.warned_100);
    }

    #[test]
    fn test_cost_warning_80_boundary_and_once() {
        let mut state = CostState::new();
        // 1.6 / 2.0 == 80%
        let res = cost_warning(&mut state, 1.6, 2.0);
        assert_eq!(res, Some(CostWarning::Cost80));
        assert!(state.warned_80);
        assert!(!state.warned_100);

        // Subsequent call between 80% and 100% returns None
        assert_eq!(cost_warning(&mut state, 1.8, 2.0), None);
        assert_eq!(cost_warning(&mut state, 1.99, 2.0), None);
    }

    #[test]
    fn test_cost_warning_100_boundary_and_once() {
        let mut state = CostState::new();
        state.warned_80 = true;
        // 2.0 / 2.0 == 100%
        let res = cost_warning(&mut state, 2.0, 2.0);
        assert_eq!(res, Some(CostWarning::Cost100));
        assert!(state.warned_100);

        // Subsequent call > 100% returns None
        assert_eq!(cost_warning(&mut state, 2.5, 2.0), None);
    }

    #[test]
    fn test_cost_warning_leapfrog_100() {
        let mut state = CostState::new();
        // Jump from <80% straight past 100%
        let res = cost_warning(&mut state, 2.5, 2.0);
        assert_eq!(res, Some(CostWarning::Cost100));
        assert!(state.warned_80);
        assert!(state.warned_100);

        // Ensure 80% is not fired later
        assert_eq!(cost_warning(&mut state, 2.6, 2.0), None);
    }

    #[test]
    fn test_cost_warning_invalid_budget() {
        let mut state = CostState::new();
        assert_eq!(cost_warning(&mut state, 1.0, 0.0), None);
        assert_eq!(cost_warning(&mut state, 1.0, -5.0), None);
        assert_eq!(cost_warning(&mut state, 1.0, f64::NAN), None);
        assert_eq!(cost_warning(&mut state, 1.0, f64::INFINITY), None);
        assert_eq!(cost_warning(&mut state, f64::NAN, 2.0), None);
        assert!(!state.warned_80);
        assert!(!state.warned_100);
    }

    #[test]
    fn test_cost_warning_from_activity_result() {
        let mut state = CostState::new();
        let act80 = ActivityResult::Cost(CostResult::Spend(2.4));
        let act100 = ActivityResult::Cost(CostResult::Spend(3.0));

        assert_eq!(
            cost_warning_from_activity(&mut state, &act80, 3.0),
            Some(CostWarning::Cost80)
        );
        assert_eq!(cost_warning_from_activity(&mut state, &act80, 3.0), None);
        assert_eq!(
            cost_warning_from_activity(&mut state, &act100, 3.0),
            Some(CostWarning::Cost100)
        );
        assert_eq!(cost_warning_from_activity(&mut state, &act100, 3.0), None);

        // Non-cost activity returns None
        let other = ActivityResult::ReportFile(ReportFileResult::Exists(true));
        assert_eq!(cost_warning_from_activity(&mut state, &other, 3.0), None);
    }
    #[test]
    fn test_cost_warning_budget_exceeded() {
        let mut state = CostState::new();

        // BudgetExceeded(false) does not trigger warning
        let not_exceeded = ActivityResult::Cost(CostResult::BudgetExceeded(false));
        assert_eq!(
            cost_warning_from_activity(&mut state, &not_exceeded, 3.0),
            None
        );
        assert!(!state.warned_80);
        assert!(!state.warned_100);

        // BudgetExceeded(true) triggers Cost100 once and marks both 80 and 100 warned
        let exceeded = ActivityResult::Cost(CostResult::BudgetExceeded(true));
        assert_eq!(
            cost_warning_from_activity(&mut state, &exceeded, 3.0),
            Some(CostWarning::Cost100)
        );
        assert!(state.warned_80);
        assert!(state.warned_100);

        // Second call with BudgetExceeded(true) returns None (once-only)
        assert_eq!(cost_warning_from_activity(&mut state, &exceeded, 3.0), None);

        // Subsequent Spend after BudgetExceeded returns None
        let later_spend = ActivityResult::Cost(CostResult::Spend(3.5));
        assert_eq!(
            cost_warning_from_activity(&mut state, &later_spend, 3.0),
            None
        );
    }

    #[test]
    fn test_report_state_mtime_stability() {
        // First sample, no prev
        let res1 = ReportFileResult::MtimeMs(1000);
        assert_eq!(report_state(None, &res1, None), ReportState::Pending);

        // Second sample, mtime changed (unstable)
        let prev1 = Some(ReportSample::new(1000));
        let res2 = ReportFileResult::MtimeMs(1200);
        assert_eq!(report_state(prev1, &res2, None), ReportState::Pending);

        // Third sample, mtime equal (stable) but no content yet -> still Pending
        let prev2 = Some(ReportSample::new(1200));
        let res3 = ReportFileResult::MtimeMs(1200);
        assert_eq!(report_state(prev2, &res3, None), ReportState::Pending);

        // Stable mtime + empty content -> Pending
        assert_eq!(report_state(prev2, &res3, Some("")), ReportState::Pending);

        // Stable mtime + valid content -> Ready
        let valid = "## Lessons\nnone";
        assert_eq!(report_state(prev2, &res3, Some(valid)), ReportState::Ready);

        // Stable mtime + malformed content -> Malformed
        let invalid = "## Summary\nDone";
        assert_eq!(
            report_state(prev2, &res3, Some(invalid)),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );

        // Mtime 0 is never ready
        let prev0 = Some(ReportSample::new(0));
        let res0 = ReportFileResult::MtimeMs(0);
        assert_eq!(report_state(prev0, &res0, None), ReportState::Pending);
        assert_eq!(
            report_state(prev0, &res0, Some(valid)),
            ReportState::Pending
        );
    }

    #[test]
    fn test_report_state_content_lessons_valid() {
        let valid_builder = "# Run Summary\n\n## Outcome\nDone\n\n## Lessons\nnone\n";
        let res = ReportFileResult::Content(valid_builder.to_string());
        assert_eq!(report_state(None, &res, None), ReportState::Ready);

        let valid_numbered = "# Run Summary\n\n## 5. Lessons\n- Note\n";
        let res2 = ReportFileResult::Content(valid_numbered.to_string());
        assert_eq!(report_state(None, &res2, None), ReportState::Ready);
    }

    #[test]
    fn test_report_state_content_lessons_malformed() {
        let missing_lessons = "# Run Summary\n\n## Outcome\nDone\n\n## Changes\nNone\n";
        let res = ReportFileResult::Content(missing_lessons.to_string());
        assert_eq!(
            report_state(None, &res, None),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );

        // Subheading ### Lessons does not count
        let deep_heading = "# Run Summary\n\n### Lessons\nnone\n";
        let res2 = ReportFileResult::Content(deep_heading.to_string());
        assert_eq!(
            report_state(None, &res2, None),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );
    }

    #[test]
    fn test_report_state_empty_content() {
        let empty = ReportFileResult::Content("   \n\t  ".to_string());
        assert_eq!(report_state(None, &empty, None), ReportState::Pending);
    }

    #[test]
    fn test_report_state_exists_variant() {
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(false), None),
            ReportState::Pending
        );
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), None),
            ReportState::Pending
        );
        let valid = "## Lessons\nnone";
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), Some(valid)),
            ReportState::Ready
        );
        let invalid = "## Summary\nDone";
        assert_eq!(
            report_state(None, &ReportFileResult::Exists(true), Some(invalid)),
            ReportState::Malformed("missing '## Lessons' section".to_string())
        );
    }

    #[test]
    fn test_report_state_reviewer_validation() {
        let pass_review = "Overall thoughts\n\nVerdict: PASS\n";
        let res = ReportFileResult::Content(pass_review.to_string());
        assert_eq!(
            report_state_validated(None, &res, None, true),
            ReportState::Ready
        );

        let fail_review = "Overall thoughts\n\nNo verdict provided\n";
        let res2 = ReportFileResult::Content(fail_review.to_string());
        assert_eq!(
            report_state_validated(None, &res2, None, true),
            ReportState::Malformed("missing 'Verdict:' line".to_string())
        );
    }

    #[test]
    fn test_report_state_from_activity_result() {
        let valid = "# Report\n\n## Lessons\nnone";
        let act = ActivityResult::ReportFile(ReportFileResult::Content(valid.to_string()));
        assert_eq!(
            report_state_from_activity(None, &act, None),
            Some(ReportState::Ready)
        );

        let non_report = ActivityResult::Cost(CostResult::Spend(1.0));
        assert_eq!(report_state_from_activity(None, &non_report, None), None);
    }

    #[test]
    fn test_omp_session_dir_name() {
        assert_eq!(
            omp_session_dir_name("/home/me/.wt/sude-x", "/home/me"),
            "-.wt-sude-x"
        );
        assert_eq!(
            omp_session_dir_name("/tmp/opencode/.wt/sude-y", "/home/me"),
            "-tmp-opencode-.wt-sude-y"
        );
        // The home prefix must match on a path boundary, never a string prefix.
        assert_eq!(
            omp_session_dir_name("/home/mel/x", "/home/me"),
            "-home-mel-x"
        );
        assert_eq!(omp_session_dir_name("/tmp/x", ""), "-tmp-x");
    }

    #[test]
    fn test_omp_cost_sums_session_jsonl() {
        let tmp = std::env::temp_dir().join(format!("omp-cost-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("sessions");
        let cwd = tmp.join("worktree");
        let dir = root.join(cwd.to_string_lossy().replace('/', "-"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.jsonl"),
            concat!(
                r#"{"type":"session","cwd":"ignored"}"#,
                "\n",
                r#"{"type":"message","message":{"usage":{"cost":{"input":0.01,"output":0.02,"total":0.03}}}}"#,
                "\n",
                r#"{"type":"message","message":{"usage":{"cost":{"input":0.1,"output":0.2,"total":0.3}}}}"#,
                "\n",
            ),
        )
        .unwrap();
        // A resumed session for the same worktree adds its own transcript.
        std::fs::write(
            dir.join("b.jsonl"),
            r#"{"type":"message","message":{"usage":{"cost":{"total":0.04}}}}"#,
        )
        .unwrap();
        // Non-jsonl siblings are ignored.
        std::fs::write(
            dir.join("a.log"),
            r#"{"type":"message","message":{"usage":{"cost":{"total":99}}}}"#,
        )
        .unwrap();

        let cost = omp_cost_in(&root, cwd.to_str().unwrap(), "").unwrap();
        assert!((cost - 0.37).abs() < 1e-9, "expected 0.37, got {cost}");

        // Unknown worktree -> None, never a silent 0.
        assert_eq!(
            omp_cost_in(&root, tmp.join("other").to_str().unwrap(), ""),
            None
        );

        // Home-relative worktrees resolve to omp's stripped dir name.
        let home = tmp.join("home");
        let hcwd = home.join(".wt/sude-z");
        let hdir = root.join("-.wt-sude-z");
        std::fs::create_dir_all(&hdir).unwrap();
        std::fs::write(
            hdir.join("s.jsonl"),
            r#"{"type":"message","message":{"usage":{"cost":{"total":0.05}}}}"#,
        )
        .unwrap();
        let c = omp_cost_in(&root, hcwd.to_str().unwrap(), home.to_str().unwrap()).unwrap();
        assert!((c - 0.05).abs() < 1e-9, "expected 0.05, got {c}");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
pub(crate) mod spend {
    use super::*;

    #[test]
    fn test_opencode_sum_multiple_sessions() {
        let tmp = std::env::temp_dir().join(format!("opencode-spend-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let db_path = tmp.join("opencode.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2 (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL,
                cost REAL DEFAULT 0 NOT NULL,
                tokens_input INTEGER DEFAULT 0 NOT NULL,
                tokens_output INTEGER DEFAULT 0 NOT NULL,
                tokens_reasoning INTEGER DEFAULT 0 NOT NULL,
                tokens_cache_read INTEGER DEFAULT 0 NOT NULL,
                tokens_cache_write INTEGER DEFAULT 0 NOT NULL,
                model TEXT,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );",
        )
        .unwrap();

        let cwd = "/path/to/worktree/tieude-p74lock";
        // Session 1: older session
        conn.execute(
            "INSERT INTO session_v2 (id, directory, cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, model, time_created, time_updated)
             VALUES ('ses-1', ?1, 0.05, 1000, 200, 50, 5000, 100, '{\"id\":\"deepseek-v4.1-flash\"}', 1000, 1500);",
            rusqlite::params![cwd],
        ).unwrap();

        // Session 2: newer session in same directory (e.g. after respawn)
        conn.execute(
            "INSERT INTO session_v2 (id, directory, cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, model, time_created, time_updated)
             VALUES ('ses-2', ?1, 0.03, 800, 150, 30, 4000, 50, '{\"id\":\"deepseek-v4.1-flash\"}', 2000, 2500);",
            rusqlite::params![cwd],
        ).unwrap();

        // 1. Without fence: SUM over the dir returns 0.08, not 0.03 or 0.05 (fails on old LIMIT 1)
        let spend = opencode_spend_in(&db_path, "", cwd, None).unwrap();
        assert!((spend.usd.unwrap() - 0.08).abs() < 1e-9);
        let tok = spend.tokens.unwrap();
        assert_eq!(tok.in_tokens, 1800);
        assert_eq!(tok.out_tokens, 350);
        assert_eq!(tok.reasoning, 80);
        assert_eq!(tok.cache_read, 9000);
        assert_eq!(tok.cache_write, 150);
        assert_eq!(spend.spend_src, "opencode-db");
        assert_eq!(spend.model.as_deref(), Some("deepseek-v4.1-flash"));

        // 2. Lifecycle fence: since_ms = 1500 ignores session 1 (time_created=1000)
        let spend_fenced = opencode_spend_in(&db_path, "", cwd, Some(1500)).unwrap();
        assert!((spend_fenced.usd.unwrap() - 0.03).abs() < 1e-9);
        assert_eq!(spend_fenced.tokens.unwrap().in_tokens, 800);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_omp_tokens_and_dollars() {
        let tmp = std::env::temp_dir().join(format!("omp-spend-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("sessions");
        let cwd = "/path/to/worktree/.wt/sude-test";
        let dir = root.join("-.wt-sude-test");
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("session-1.jsonl"),
            concat!(
                r#"{"timestamp":"2026-10-21T10:00:00.000Z","message":{"model":"gemini-3.8-flash","usage":{"input":100,"output":20,"reasoningTokens":5,"cacheRead":50,"cacheWrite":10,"cost":{"total":0.015}}}}"#,
                "\n",
                r#"{"timestamp":"2026-10-21T10:05:00.000Z","message":{"model":"gemini-3.8-flash","usage":{"input":200,"output":40,"reasoningTokens":10,"cacheRead":100,"cacheWrite":20,"cost":{"total":0.025}}}}"#,
                "\n",
            ),
        ).unwrap();

        // Without fence
        let spend = omp_spend_in(&root, cwd, "/path/to/worktree", None).unwrap();
        assert!((spend.usd.unwrap() - 0.04).abs() < 1e-9);
        let tok = spend.tokens.unwrap();
        assert_eq!(tok.in_tokens, 300);
        assert_eq!(tok.out_tokens, 60);
        assert_eq!(tok.reasoning, 15);
        assert_eq!(tok.cache_read, 150);
        assert_eq!(tok.cache_write, 30);
        assert_eq!(spend.spend_src, "omp-session");
        assert_eq!(spend.model.as_deref(), Some("gemini-3.8-flash"));

        // With fence: fence at 10:02 ignores the first line
        let fence_ms = parse_iso_timestamp_ms("2026-10-21T10:02:00.000Z").unwrap();
        let spend_fenced = omp_spend_in(&root, cwd, "/path/to/worktree", Some(fence_ms)).unwrap();
        assert!((spend_fenced.usd.unwrap() - 0.025).abs() < 1e-9);
        assert_eq!(spend_fenced.tokens.unwrap().in_tokens, 200);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_claude_dedup_message_ids() {
        let tmp = std::env::temp_dir().join(format!("claude-spend-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let cwd = "/path/to/worktree/truonglao-test";
        let slug = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>();
        let project_dir = tmp.join(&slug);
        std::fs::create_dir_all(&project_dir).unwrap();

        let session_id = "test-session-123";
        let session_file = project_dir.join(format!("{session_id}.jsonl"));

        // Line 1: msg-1 initial emission
        // Line 2: msg-1 repeated emission (same message.id, identical or updated tokens)
        // Line 3: msg-2 distinct message.id
        std::fs::write(
            &session_file,
            concat!(
                r#"{"timestamp":"2026-10-21T12:00:00.000Z","message":{"id":"msg-1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":50,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens_details":{"thinking_tokens":5}}}}"#,
                "\n",
                r#"{"timestamp":"2026-10-21T12:00:01.000Z","message":{"id":"msg-1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":50,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens_details":{"thinking_tokens":5}}}}"#,
                "\n",
                r#"{"timestamp":"2026-10-21T12:01:00.000Z","message":{"id":"msg-2","model":"claude-opus-5-5","usage":{"input_tokens":20,"output_tokens":80,"cache_read_input_tokens":200,"cache_creation_input_tokens":40,"output_tokens_details":{"thinking_tokens":10}}}}"#,
                "\n",
            ),
        ).unwrap();

        let spend = claude_spend_in(&tmp, session_id, cwd, None).unwrap();
        assert_eq!(spend.usd, None, "claude records tokens only, usd must be None");
        assert_eq!(spend.spend_src, "claude-tokens");
        assert_eq!(spend.model.as_deref(), Some("claude-opus-5-5"));

        let tok = spend.tokens.unwrap();
        // msg-1 is deduped: (10 + 20) = 30, NOT 40
        assert_eq!(tok.in_tokens, 30);
        // output: (50 + 80) = 130, NOT 180
        assert_eq!(tok.out_tokens, 130);
        // reasoning: (5 + 10) = 15, NOT 20
        assert_eq!(tok.reasoning, 15);
        // cache_read: (100 + 200) = 300, NOT 400
        assert_eq!(tok.cache_read, 300);
        // cache_write: (20 + 40) = 60, NOT 80
        assert_eq!(tok.cache_write, 60);

        // Lifecycle fence: fence at 12:00:30 ignores msg-1
        let fence_ms = parse_iso_timestamp_ms("2026-10-21T12:00:30.000Z").unwrap();
        let spend_fenced = claude_spend_in(&tmp, session_id, cwd, Some(fence_ms)).unwrap();
        assert_eq!(spend_fenced.tokens.unwrap().in_tokens, 20);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_iso_timestamp_ms() {
        let utc = parse_iso_timestamp_ms("2026-10-21T13:40:00Z").unwrap();
        let vn = parse_iso_timestamp_ms("2026-10-21T20:40:00+07:00").unwrap();
        assert_eq!(utc, vn, "UTC and +07:00 representations must yield identical epoch ms");

        let frac = parse_iso_timestamp_ms("2026-10-21T13:40:00.500Z").unwrap();
        assert_eq!(frac, utc + 500);
    }
}
