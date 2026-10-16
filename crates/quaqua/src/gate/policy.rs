//! Port of the `herdr-gate.sh` checker (`check_text` / `check_words` /
//! `_gw_keyword`), including the herdr `--help`/`-h` positions and the prefix
//! wrappers.

use super::tokenizer::tokenize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block,
}

/// `check_text <shell-source>`: tokenize, then check every simple command.
pub fn check(command: &str) -> Verdict {
    check_segments(&tokenize(command))
}

pub fn check_segments(segs: &[Vec<String>]) -> Verdict {
    for seg in segs {
        if check_words(seg) == Verdict::Block {
            return Verdict::Block;
        }
    }
    Verdict::Allow
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
fn check_words(w: &[String]) -> Verdict {
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
    match base {
        "herdr" => check_herdr(a),
        "bash" | "sh" | "dash" | "zsh" => {
            for k in 0..a.len() {
                if is_c_flag(&a[k]) && k + 1 < a.len() {
                    // the -c payload; trailing args are $0/$1
                    return check(&a[k + 1]);
                }
            }
            Verdict::Allow
        }
        "eval" => {
            if a.is_empty() {
                Verdict::Allow
            } else {
                check(&a.join(" "))
            }
        }
        "sudo" | "env" | "command" | "exec" | "nice" | "time" | "nohup" | "setsid" | "stdbuf"
        | "xargs" | "builtin" | "chrt" | "ionice" | "taskset" => wrapper(base, a),
        _ => Verdict::Allow,
    }
}

/// The `herdr` arm: help handling and the guarded verb pair.
fn check_herdr(a: &[String]) -> Verdict {
    // `herdr --help` / `herdr -h` (top-level help) exits before any verb runs.
    if let Some(first) = a.first() {
        if first == "--help" || first == "-h" {
            return Verdict::Allow;
        }
    }
    let mut first: Option<usize> = None;
    let mut k = 0;
    while k + 1 < a.len() {
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
    // `herdr <verb> --help` / `-h` prints usage and exits before the verb runs.
    if verb
        && first + 2 < a.len()
        && (a[first + 2] == "--help" || a[first + 2] == "-h")
    {
        return Verdict::Allow;
    }
    Verdict::Block
}

/// Prefix wrappers: the real command follows their options and `VAR=` prefixes.
/// Value-taking flags differ per wrapper; for the rest a `-*` word is only
/// dropped, never allowed to swallow the next word.
fn wrapper(base: &str, a: &[String]) -> Verdict {
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
        check_words(rest)
    }
}
