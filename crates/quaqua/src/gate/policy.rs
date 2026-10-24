//! Port of the `herdr-gate.sh` checker (`check_text` / `check_words` /
//! `_gw_keyword`), including the herdr `--help`/`-h` positions and the prefix
//! wrappers, plus the outward-action classifier: `git push` and the gh write
//! subcommands are `Ask` (the user confirms via Claude Code's native prompt),
//! force pushes / pushes to main or master / `gh pr merge <main>` are `Deny`
//! (AGENTS.md rule 3).
//!
//! Two retro slips are enforced here too (issue #55): `gh pr merge` asks with
//! the reason when the PR body/comments still say WIP, "cancelled mid-run" or
//! carry an unchecked `- [ ]` item (and asks when the lookup itself fails,
//! never silently allowing), and a call to a removed `scripts/worker/*` path
//! is denied with the `quaqua` command that replaced it.

use super::tokenizer::tokenize;
use std::cell::OnceCell;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// What a PR lookup returns: the body and every comment body, the two places
/// a stale "work in progress" marker hides (issue #55 item 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrFacts {
    pub body: String,
    pub comments: Vec<String>,
}

/// The `gh pr view --json body,comments` boundary. Kept behind a trait so the
/// policy tests never touch the network, and so a failed lookup is a value the
/// gate can turn into `Ask` instead of letting the merge through silently.
pub trait PrLookup {
    fn view(&self, selector: Option<&str>, cwd: Option<&str>) -> Result<PrFacts, String>;
}

/// In-process default: no lookup available, so a merge cannot be verified and
/// the gate asks instead of silently allowing it.
pub struct OfflinePrLookup;

impl PrLookup for OfflinePrLookup {
    fn view(&self, _selector: Option<&str>, _cwd: Option<&str>) -> Result<PrFacts, String> {
        Err("PR lookup unavailable in this context".to_string())
    }
}

const NO_PR_LOOKUP: OfflinePrLookup = OfflinePrLookup;

/// The herdr pane metadata boundary for CoS pane protection.
pub trait PaneLookup {
    fn is_cos_pane(&self, pane: &str) -> bool;
}

/// In-process default: no pane lookup available, so CoS pane protection falls back to offline.
pub struct OfflinePaneLookup;

impl PaneLookup for OfflinePaneLookup {
    fn is_cos_pane(&self, _pane: &str) -> bool {
        false
    }
}

const NO_PANE_LOOKUP: OfflinePaneLookup = OfflinePaneLookup;

/// Real herdr lookup: `herdr pane get <pane>`, reads tokens to detect `grid_role=cos`.
pub struct RealHerdrPaneLookup {
    pub bin: String,
}

impl RealHerdrPaneLookup {
    pub fn from_env() -> Self {
        let bin = crate::herdr::bin_path();
        RealHerdrPaneLookup { bin }
    }

    fn resolve_current(&self) -> String {
        let out = Command::new(&self.bin)
            .args(["pane", "current"])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    if let Some(pid) = v.pointer("/result/pane/pane_id").and_then(|p| p.as_str()) {
                        return pid.to_string();
                    }
                }
            }
        }
        String::new()
    }
}

impl PaneLookup for RealHerdrPaneLookup {
    fn is_cos_pane(&self, pane: &str) -> bool {
        let target_pane = if pane.is_empty() || pane == "--current" || pane == "-c" {
            if let Ok(env_p) = std::env::var("HERDR_PANE_ID") {
                let trimmed = env_p.trim();
                if !trimmed.is_empty() {
                    trimmed.to_string()
                } else {
                    self.resolve_current()
                }
            } else {
                self.resolve_current()
            }
        } else {
            pane.to_string()
        };

        if target_pane.is_empty() {
            return false;
        }

        let out = Command::new(&self.bin)
            .args(["pane", "get", &target_pane])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    if let Some(tokens) = v.pointer("/result/pane/tokens") {
                        return crate::cos::is_cos_tokens(tokens);
                    }
                }
            }
        }
        false
    }
}

/// The real lookup: `gh pr view`, bounded by a timeout so a hung network call
/// cannot hang the PreToolUse hook.
pub struct RealGh {
    pub bin: String,
    pub timeout: Duration,
}

impl RealGh {
    /// `QUAQUA_GH_BIN` overrides the binary (tests use a stub); an empty value
    /// counts as unset. `QUAQUA_GATE_PR_TIMEOUT_MS` overrides the timeout.
    pub fn from_env() -> Self {
        let bin = std::env::var("QUAQUA_GH_BIN")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gh".to_string());
        let timeout = std::env::var("QUAQUA_GATE_PR_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(3000));
        RealGh { bin, timeout }
    }
}

impl PrLookup for RealGh {
    fn view(&self, selector: Option<&str>, cwd: Option<&str>) -> Result<PrFacts, String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["pr", "view"]);
        if let Some(s) = selector {
            cmd.arg(s);
        }
        cmd.args(["--json", "body,comments"]);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let out = run_with_timeout(cmd, self.timeout, &self.bin)?;
        parse_pr_json(&out.stdout)
    }
}

/// Run a command, killing it after `timeout`. `std::process` has no
/// wait-timeout, so poll `try_wait`; any failure (spawn, non-zero exit,
/// timeout) is the lookup's error reason.
fn run_with_timeout(mut cmd: Command, timeout: Duration, what: &str) -> Result<Output, String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{what} timed out after {}ms", timeout.as_millis()));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("waiting for {what} failed: {e}")),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("reading {what} output failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let err = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(format!("{what} failed: {err}"));
    }
    Ok(out)
}

fn parse_pr_json(raw: &[u8]) -> Result<PrFacts, String> {
    let v: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| format!("invalid gh JSON: {e}"))?;
    let body = v
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or_default()
        .to_string();
    let comments = v
        .get("comments")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("body").and_then(|b| b.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(PrFacts { body, comments })
}

/// The gate's decision for one command.
///
/// `Ask` and `Deny` carry the reason the hook reports
/// (`permissionDecisionReason`); `Block` is the herdr spawn+move guard, kept
/// as its own variant because it exits 2 with stderr instead of JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Ask(String),
    Deny(String),
    Block,
}

/// Deny > Block > Ask > Allow: a hard refusal anywhere wins, a herdr block
/// outranks a prompt (an approval must not let a pane/workspace spawn through
/// as a side effect of a compound command), and a prompt outranks plain runs.
fn rank(v: &Verdict) -> u8 {
    match v {
        Verdict::Allow => 0,
        Verdict::Ask(_) => 1,
        Verdict::Block => 2,
        Verdict::Deny(_) => 3,
    }
}

fn worst(a: Verdict, b: Verdict) -> Verdict {
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

/// `check_text <shell-source>`: tokenize, then check every simple command.
/// Offline: a merge whose PR state would need `gh` asks instead of shelling out
/// (the hook binary uses [`check_with_pr`] with the real lookup).
pub fn check(command: &str) -> Verdict {
    check_with_lookups(command, None, &OfflinePrLookup, &OfflinePaneLookup)
}

/// `check_text` with the hook's `cwd`, needed only to resolve the branch a
/// bare `git push` (no refspec in the command text) would push.
pub fn check_ctx(command: &str, cwd: Option<&str>) -> Verdict {
    check_with_lookups(command, cwd, &OfflinePrLookup, &OfflinePaneLookup)
}

/// `check_text` with an explicit PR lookup — the hook passes the real `gh`
/// one; tests pass a stub.
pub fn check_with_pr(command: &str, cwd: Option<&str>, pr: &dyn PrLookup) -> Verdict {
    check_with_lookups(command, cwd, pr, &OfflinePaneLookup)
}

/// `check_text` with explicit PR and pane lookups.
pub fn check_with_lookups(
    command: &str,
    cwd: Option<&str>,
    pr: &dyn PrLookup,
    pane: &dyn PaneLookup,
) -> Verdict {
    let ctx = Ctx::with_lookups(cwd, pr, pane);
    check_segments_ctx(&tokenize(command), &ctx)
}

pub fn check_segments(segs: &[Vec<String>]) -> Verdict {
    check_segments_ctx(segs, &Ctx::none())
}

fn check_segments_ctx(segs: &[Vec<String>], ctx: &Ctx) -> Verdict {
    let mut v = Verdict::Allow;
    for seg in segs {
        v = worst(v, check_words(seg, ctx));
    }
    v
}

/// Per-invocation context. `branch_of` runs `git symbolic-ref` only for a
/// refspec-less `git push`, memoized for the hook's own cwd. `pr` resolves the
/// PR a `gh pr merge` would merge.
struct Ctx<'a> {
    cwd: Option<&'a str>,
    branch: OnceCell<Option<String>>,
    pr: &'a dyn PrLookup,
    pane: &'a dyn PaneLookup,
}

impl<'a> Ctx<'a> {
    fn none() -> Self {
        Ctx {
            cwd: None,
            branch: OnceCell::new(),
            pr: &NO_PR_LOOKUP,
            pane: &NO_PANE_LOOKUP,
        }
    }


    fn with_lookups(cwd: Option<&'a str>, pr: &'a dyn PrLookup, pane: &'a dyn PaneLookup) -> Self {
        Ctx {
            cwd,
            branch: OnceCell::new(),
            pr,
            pane,
        }
    }


    /// The checked-out branch of `dir` (or of the hook's cwd), `None` when it
    /// cannot be determined (no cwd, detached HEAD, not a repository).
    fn branch_of(&self, dir: Option<&str>) -> Option<String> {
        if dir.is_none() {
            return self
                .branch
                .get_or_init(|| self.repo_dir(None).and_then(|d| symbolic_ref(&d)))
                .clone();
        }
        self.repo_dir(dir).and_then(|d| symbolic_ref(&d))
    }

    /// `git -C <dir>` when the command carries one, the hook's cwd otherwise.
    fn repo_dir(&self, dir: Option<&str>) -> Option<String> {
        match dir {
            None => self.cwd.map(String::from),
            Some("~") => std::env::var("HOME").ok(),
            Some(d) => {
                if let Some(rest) = d.strip_prefix("~/") {
                    std::env::var("HOME").ok().map(|h| format!("{h}/{rest}"))
                } else if d.starts_with('/') {
                    Some(d.to_string())
                } else {
                    self.cwd.map(|c| format!("{c}/{d}"))
                }
            }
        }
    }
}

/// `git -C <dir> symbolic-ref --short -q HEAD` — read-only; quiet, and a
/// non-zero exit (detached HEAD, not a repo) means "unknown".
fn symbolic_ref(dir: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if b.is_empty() {
        None
    } else {
        Some(b)
    }
}

fn ask(what: &str) -> Verdict {
    Verdict::Ask(format!("{what} — needs anh iu approval (AGENTS.md rule 3)"))
}

fn deny(what: &str, why: &str) -> Verdict {
    Verdict::Deny(format!("{what}: {why} (AGENTS.md rule 3)"))
}

/// The bash wrappers removed in a35a54d and the binary command that replaced
/// each (issue #55 item 3). The whole `scripts/worker/` directory is gone, so a
/// path below it that is not in this table still gets the generic hint.
const REMOVED_WORKER_SCRIPTS: &[(&str, &str)] = &[
    ("spawn-worker.sh", "quaqua worker spawn"),
    ("prompt-worker.sh", "quaqua worker prompt"),
    ("cleanup-worker.sh", "quaqua worker cleanup"),
    ("grid-workers.sh", "quaqua grid reconcile"),
    ("worker-cap.sh", "quaqua cap"),
];

const REMOVED_WORKER_HINT: &str =
    "quaqua worker {spawn,prompt,cleanup,stop} / quaqua cap / quaqua grid reconcile";

/// The replacement command when `word` names a removed `scripts/worker/*`
/// script (any prefix: `./`, `~/quaqua/`, absolute).
fn removed_worker_script(word: &str) -> Option<&'static str> {
    let path = word.replace('\\', "/");
    let name = path.rsplit('/').next().unwrap_or(&path);
    if let Some((_, cmd)) = REMOVED_WORKER_SCRIPTS.iter().find(|(n, _)| *n == name) {
        return Some(cmd);
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .windows(2)
        .any(|w| w[0] == "scripts" && w[1] == "worker")
    {
        return Some(REMOVED_WORKER_HINT);
    }
    None
}

fn deny_removed(word: &str, replacement: &str) -> Verdict {
    deny(
        word,
        &format!("that script was removed — use `{replacement}`"),
    )
}

/// `_gw_keyword`: compound-shell keywords. `time` is deliberately absent — it is
/// a prefix wrapper (handled in `wrapper`), so `time -p herdr pane split`
/// still resolves `herdr`.
fn keyword(w: &str) -> bool {
    matches!(
        w,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "in"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "select"
            | "function"
            | "{"
            | "}"
            | "!"
            | "coproc"
    )
}

/// `^[A-Za-z_][A-Za-z0-9_]*=`
fn is_assignment(w: &str) -> bool {
    let b = w.as_bytes();
    if b.is_empty() {
        return false;
    }
    if !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return false;
    }
    let mut k = 1;
    while k < b.len() {
        let c = b[k];
        if c == b'=' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return false;
        }
        k += 1;
    }
    false
}

/// `^-[a-z]*c$` — the `bash -c` payload flag.
fn is_c_flag(w: &str) -> bool {
    let b = w.as_bytes();
    b.len() >= 2
        && b[0] == b'-'
        && b[b.len() - 1] == b'c'
        && b[1..b.len() - 1].iter().all(|c| c.is_ascii_lowercase())
}

/// `^-([a-zA-Z])$` — a one-letter wrapper flag.
fn single_letter_flag(w: &str) -> Option<char> {
    let b = w.as_bytes();
    if b.len() == 2 && b[0] == b'-' && b[1].is_ascii_alphabetic() {
        Some(b[1] as char)
    } else {
        None
    }
}

fn guarded_pair(a: &str, b: &str) -> bool {
    matches!(
        (a, b),
        ("pane", "split")
            | ("pane", "move")
            | ("tab", "create")
            | ("workspace", "create")
            | ("workspace", "close")
            | ("agent", "start")
    )
}

/// `check_words <word>...` — resolve the executable and apply the gate.
fn check_words(w: &[String], ctx: &Ctx) -> Verdict {
    let mut i = 0;
    while i < w.len() {
        if keyword(&w[i]) || is_assignment(&w[i]) {
            i += 1;
            continue;
        }
        break;
    }
    if i >= w.len() {
        return Verdict::Allow;
    }
    // executable word: quotes stripped, path removed
    let base = w[i].rsplit('/').next().unwrap_or(w[i].as_str());
    let a: &[String] = if i + 1 < w.len() { &w[i + 1..] } else { &[] };
    // A removed wrapper in executable position is a call to code that no
    // longer exists (issue #55 item 3), never a plain run.
    if let Some(replacement) = removed_worker_script(&w[i]) {
        return deny_removed(&w[i], replacement);
    }
    match base {
        "herdr" => check_herdr(a, ctx),
        "git" => check_git(a, ctx),
        "gh" => check_gh(a, ctx),
        "bash" | "sh" | "dash" | "zsh" => {
            for k in 0..a.len() {
                if is_c_flag(&a[k]) && k + 1 < a.len() {
                    // the -c payload; trailing args are $0/$1
                    return check_segments_ctx(&tokenize(&a[k + 1]), ctx);
                }
            }
            // `bash scripts/worker/prompt-worker.sh …`: the script argument is
            // the command being run, not argument text.
            for arg in a {
                if !arg.starts_with('-') {
                    if let Some(replacement) = removed_worker_script(arg) {
                        return deny_removed(arg, replacement);
                    }
                }
            }
            Verdict::Allow
        }
        "source" | "." => {
            for arg in a {
                if !arg.starts_with('-') {
                    if let Some(replacement) = removed_worker_script(arg) {
                        return deny_removed(arg, replacement);
                    }
                }
            }
            Verdict::Allow
        }
        "eval" => {
            if a.is_empty() {
                Verdict::Allow
            } else {
                check_segments_ctx(&tokenize(&a.join(" ")), ctx)
            }
        }
        "sudo" | "env" | "command" | "exec" | "nice" | "time" | "nohup" | "setsid" | "stdbuf"
        | "xargs" | "builtin" | "chrt" | "ionice" | "taskset" => wrapper(base, a, ctx),
        _ => Verdict::Allow,
    }
}

/// The `herdr` arm: help handling and the guarded verb pair.
fn check_herdr(a: &[String], ctx: &Ctx) -> Verdict {
    // `herdr --help` / `herdr -h` (top-level help) exits before any verb runs.
    if let Some(first) = a.first() {
        if first == "--help" || first == "-h" {
            return Verdict::Allow;
        }
    }
    let mut first: Option<usize> = None;
    let mut is_close = false;
    let mut k = 0;
    while k + 1 < a.len() {
        if a[k] == "pane" && a[k + 1] == "close" {
            first = Some(k);
            is_close = true;
            break;
        }
        if guarded_pair(&a[k], &a[k + 1]) {
            first = Some(k);
            break;
        }
        k += 1;
    }
    let Some(first) = first else {
        return Verdict::Allow;
    };
    // the pair is the command's verb only when every word before it is a global
    // option or that option's value: `herdr --session x pane split --help` is
    // that verb's help, while in `herdr pane send-keys x pane split --help` the
    // pair is argument text of another verb.
    let mut verb = true;
    for kk in 0..first {
        if !a[kk].starts_with('-') && (kk == 0 || !a[kk - 1].starts_with('-')) {
            verb = false;
            break;
        }
    }
    if !verb {
        return Verdict::Allow;
    }
    // `herdr <verb> --help` / `-h` prints usage and exits before the verb runs.
    if first + 2 < a.len() && (a[first + 2] == "--help" || a[first + 2] == "-h") {
        return Verdict::Allow;
    }

    // CoS pane protection: refuse to close or move a pane carrying grid_role=cos.
    let is_move = a[first] == "pane" && a[first + 1] == "move";
    if is_close || is_move {
        let mut target_pane = "";
        let mut idx = first + 2;
        while idx < a.len() {
            let arg = a[idx].as_str();
            if !arg.starts_with('-') {
                target_pane = arg;
                break;
            } else if arg == "--current" || arg == "-c" {
                target_pane = "--current";
                break;
            }
            idx += 1;
        }

        if ctx.pane.is_cos_pane(target_pane) {
            let verb_str = if is_close { "close" } else { "move" };
            return Verdict::Deny(format!("refusing to {verb_str} CoS pane '{target_pane}' (grid_role=cos)"));
        }
    }

    if is_close {
        // pane close on non-CoS pane is allowed
        Verdict::Allow
    } else {
        Verdict::Block
    }
}

/// `git` arm: only `push` is outward. Every other verb (status, fetch, log,
/// diff, show, …) is left alone, and `git -C <dir>` decides which repository a
/// bare push would push.
fn check_git(a: &[String], ctx: &Ctx) -> Verdict {
    let mut dir: Option<String> = None;
    let mut vidx = 0;
    loop {
        if vidx >= a.len() {
            return Verdict::Allow;
        }
        let w = a[vidx].as_str();
        if let Some(rest) = w.strip_prefix("-C") {
            // -C <dir> or -C<dir>
            if rest.is_empty() {
                if vidx + 1 >= a.len() {
                    return Verdict::Allow;
                }
                dir = Some(a[vidx + 1].clone());
                vidx += 2;
            } else {
                dir = Some(rest.to_string());
                vidx += 1;
            }
            continue;
        }
        if let Some(rest) = w.strip_prefix("-c") {
            // -c name=value or -cname=value
            vidx += if rest.is_empty() { 2 } else { 1 };
            continue;
        }
        if w.starts_with('-') {
            // `git -h` / `git --help` print usage and exit before anything runs
            if w == "--help" || w == "-h" {
                return Verdict::Allow;
            }
            // --git-dir/--work-tree/--namespace/--exec-path take a separate
            // value; every other global option is a boolean or `=`-attached.
            let takes_value = matches!(
                w,
                "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
            );
            vidx += if takes_value { 2 } else { 1 };
            continue;
        }
        break;
    }
    if a[vidx] != "push" {
        return Verdict::Allow;
    }
    check_push(&a[vidx + 1..], ctx, dir.as_deref())
}

/// The destination side of a refspec: `src:dst` (last `:`) or `ref` for the
/// same-named form; a leading `+` is the force marker.
fn refspec_dst(r: &str) -> &str {
    let r = r.strip_prefix('+').unwrap_or(r);
    match r.rsplit_once(':') {
        Some((_, dst)) => dst,
        None => r,
    }
}

/// A pushed or merged-into branch that is main/master, in the spellings a
/// refspec or a branch argument can carry.
fn is_main(s: &str) -> bool {
    matches!(
        s,
        "main" | "master" | "refs/heads/main" | "refs/heads/master" | "heads/main" | "heads/master"
    )
}

/// `git push …`: force, `--all`/`--branches`/`--mirror`, or a refspec whose
/// destination is main/master is denied; every other push asks. With no
/// refspec in the command text the branch the push would send is the currently
/// checked-out branch, resolved from the hook's cwd (`git -C <dir>` when the
/// command carries one) — the upstream ref is not visible to the tokenizer.
fn check_push(a: &[String], ctx: &Ctx, dir: Option<&str>) -> Verdict {
    let mut force = false;
    let mut all = false;
    let mut positionals: Vec<&str> = Vec::new();
    let mut no_flags = false;
    let mut i = 0;
    while i < a.len() {
        let w = a[i].as_str();
        if !no_flags && w == "--" {
            no_flags = true;
            i += 1;
            continue;
        }
        if !no_flags && w.starts_with('-') && w != "-" {
            // `git push -h` / `git push --help` print usage without pushing
            if w == "--help" || w == "-h" {
                return Verdict::Allow;
            }
            if matches!(
                w,
                "-o" | "--push-option" | "--receive-pack" | "--exec" | "--repo"
            ) {
                i += 2; // option and its separate value
                continue;
            }
            if w.starts_with("--") {
                if w.starts_with("--force") && !w.starts_with("--no-force") {
                    force = true;
                }
                if matches!(w, "--all" | "--branches" | "--mirror") || w.starts_with("--branches=")
                {
                    all = true;
                }
            } else {
                // -f / -o<value> / boolean shorts, possibly clustered (-uf)
                for c in w[1..].chars() {
                    if c == 'f' {
                        force = true;
                    }
                    if c == 'o' {
                        break; // the rest of the word is -o's value
                    }
                }
            }
            i += 1;
            continue;
        }
        positionals.push(w);
        i += 1;
    }
    let mut has_refspec = false;
    let mut to_main: Option<&str> = None;
    for (k, r) in positionals.iter().enumerate() {
        // the first positional is git's <repository>; it is a refspec only
        // when it is spelled like one (`HEAD:main`, `+ref` — git then reads it
        // as a repository, but the intent is a push to that ref)
        if k == 0 && !r.contains(':') && !r.starts_with('+') {
            continue;
        }
        has_refspec = true;
        if r.starts_with('+') {
            force = true;
        }
        let dst = refspec_dst(r);
        if is_main(dst) {
            to_main = Some(dst);
        }
    }
    if force {
        return deny("git push", "force push is denied");
    }
    if let Some(dst) = to_main {
        return deny(&format!("git push {dst}"), "push to main/master is denied");
    }
    if all {
        return deny(
            "git push --all/--mirror",
            "may push main/master and is denied",
        );
    }
    if !has_refspec {
        if let Some(branch) = ctx.branch_of(dir) {
            if is_main(&branch) {
                return deny(
                    "git push",
                    "the checked-out branch is main/master — push to main/master is denied",
                );
            }
        }
    }
    ask("git push")
}

/// `gh` arm: only the enumerated outward subcommands ask (or deny). Read-only
/// subcommands (view, list, status, diff, download, clone, …) stay untouched,
/// and `--help`/`-h` in a help position exits before anything runs.
fn check_gh(a: &[String], ctx: &Ctx) -> Verdict {
    let mut i = 0;
    while i < a.len() && a[i].starts_with('-') {
        // global options: -R/--repo take a value, everything else is boolean
        i += if matches!(a[i].as_str(), "-R" | "--repo") {
            2
        } else {
            1
        };
    }
    if i >= a.len() || a[i] == "--help" || a[i] == "-h" {
        return Verdict::Allow;
    }
    let group = a[i].as_str();
    let mut j = i + 1;
    while j < a.len() && a[j].starts_with('-') {
        j += if matches!(a[j].as_str(), "-R" | "--repo") {
            2
        } else {
            1
        };
    }
    if j >= a.len() || a[j] == "--help" || a[j] == "-h" {
        return Verdict::Allow; // `gh pr`, `gh pr -h`: usage only
    }
    let sub = a[j].as_str();
    let rest = &a[j + 1..];
    if !rest.is_empty() && (rest[0] == "--help" || rest[0] == "-h") {
        return Verdict::Allow; // `gh pr create --help`: usage only
    }
    match group {
        "pr" => match sub {
            "create" | "edit" | "close" | "comment" | "ready" => ask(&format!("gh pr {sub}")),
            "merge" => check_merge(rest, ctx),
            _ => Verdict::Allow,
        },
        "issue" => match sub {
            "create" | "edit" | "close" | "comment" => ask(&format!("gh issue {sub}")),
            _ => Verdict::Allow,
        },
        "release" => match sub {
            "create" | "edit" | "delete" | "upload" => ask(&format!("gh release {sub}")),
            _ => Verdict::Allow,
        },
        "repo" => match sub {
            "create" | "delete" | "fork" | "rename" | "edit" | "archive" | "unarchive" | "sync"
            | "set-default" | "transfer" => ask(&format!("gh repo {sub}")),
            _ => Verdict::Allow,
        },
        _ => Verdict::Allow,
    }
}

/// `gh pr merge`: deny main/master first (no lookup needed), then ask — with
/// the reason when the PR still says WIP / "cancelled mid-run" / has an
/// unchecked `- [ ]`, or when the lookup itself failed. A clean PR or an
/// unreachable `gh` both ask; neither ever allows (issue #55 item 2).
fn check_merge(rest: &[String], ctx: &Ctx) -> Verdict {
    let target = merge_target(rest);
    if let Some(t) = target {
        if is_main(t) {
            return deny(
                &format!("gh pr merge {t}"),
                "merging to main/master is denied",
            );
        }
    }
    let what = match target {
        Some(t) => format!("gh pr merge {t}"),
        None => "gh pr merge".to_string(),
    };
    match ctx.pr.view(target, ctx.cwd) {
        Ok(facts) => match pr_problem(&facts) {
            Some(reason) => Verdict::Ask(format!(
                "{what} — {reason}; needs anh iu approval (AGENTS.md rule 3)"
            )),
            None => ask(&what),
        },
        Err(e) => Verdict::Ask(format!(
            "{what} — could not verify the PR state ({e}); needs anh iu approval (AGENTS.md rule 3)"
        )),
    }
}

/// `gh pr merge [<number> | <url> | <branch>] [flags]`: the first positional
/// after the value-taking flags. `None` means gh resolves the current branch.
fn merge_target(rest: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < rest.len() {
        let w = rest[i].as_str();
        if w == "--" {
            return rest.get(i + 1).map(|s| s.as_str());
        }
        if w.starts_with('-') {
            i += if matches!(
                w,
                "-b" | "--body"
                    | "-F"
                    | "--body-file"
                    | "-t"
                    | "--subject"
                    | "--match-head-commit"
                    | "-R"
                    | "--repo"
            ) {
                2
            } else {
                1
            };
            continue;
        }
        return Some(w);
    }
    None
}

/// The first stale-work marker in the PR, as the reason to show. Body first,
/// then each comment in order.
fn pr_problem(facts: &crate::gate::policy::PrFacts) -> Option<String> {
    if let Some(why) = text_problem(&facts.body) {
        return Some(format!("the PR body {why}"));
    }
    for (i, comment) in facts.comments.iter().enumerate() {
        if let Some(why) = text_problem(comment) {
            return Some(format!("PR comment #{} {why}", i + 1));
        }
    }
    None
}

fn text_problem(text: &str) -> Option<&'static str> {
    // "WIP" as a word: `WIP:`, `[wip]`, `wip —` all match; "swipe" does not.
    if text
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("wip"))
    {
        return Some("contains WIP");
    }
    if text.to_lowercase().contains("cancelled mid-run") {
        return Some("contains \"cancelled mid-run\"");
    }
    if text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("- [ ]") || t.starts_with("* [ ]")
    }) {
        return Some("has an unchecked '- [ ]' item");
    }
    None
}

/// Prefix wrappers: the real command follows their options and `VAR=` prefixes.
/// Value-taking flags differ per wrapper; for the rest a `-*` word is only
/// dropped, never allowed to swallow the next word.
fn wrapper(base: &str, a: &[String], ctx: &Ctx) -> Verdict {
    let vf: &str = match base {
        "sudo" => " u g h p C T U R t D ",
        "env" => " u C S ",
        "nice" => " n ",
        _ => " ",
    };
    let mut rest: &[String] = &[];
    let mut k = 0;
    while k < a.len() {
        if let Some(flag) = single_letter_flag(&a[k]) {
            if vf.contains(&format!(" {flag} ")) {
                k += 2; // value-taking flag: consume its value too
                continue;
            }
        }
        if a[k].starts_with('-') {
            k += 1; // boolean flags
            continue;
        }
        if !is_assignment(&a[k]) {
            rest = &a[k..];
            break;
        }
        k += 1;
    }
    if rest.is_empty() {
        Verdict::Allow
    } else {
        check_words(rest, ctx)
    }
}
