//! `quaqua gate` policy tests — every case from `tests/test_herdr_gate.sh`,
//! checked in-process against the ported tokenizer + policy, plus one test that
//! drives the built binary through the stdin hook-JSON contract.

use quaqua::gate::{check, Verdict};
use std::io::Write;
use std::process::{Command, Stdio};

fn assert_cases(cases: &[&str], want: Verdict, group: &str) {
    for c in cases {
        assert_eq!(check(c), want, "group `{group}`: case {c:?}");
    }
}

#[test]
fn direct_herdr_calls_are_blocked() {
    assert_cases(
        &[
            "herdr pane split --current --direction right",
            "herdr pane move w1:p5 --new-tab --label x",
            "herdr tab create --label x --no-focus",
            "herdr agent start sude-x --kind pi --pane w1:p5",
            "herdr workspace create --label baro --no-focus",
            "herdr workspace close w7",
            "cd ~/quaqua && herdr pane split --current",
            "X=$(herdr tab create --label y)",
        ],
        Verdict::Block,
        "direct",
    );
}

#[test]
fn equivalent_spellings_are_blocked() {
    assert_cases(
        &[
            "\"herdr\" pane split --current",
            "'herdr' pane move w1:p5 --new-tab",
            "h\\e\\rdr tab create --label x",
            "/usr/local/bin/herdr workspace close w7",
            "./herdr tab create --label x",
            "bash -c 'herdr agent start sude-x --kind pi'",
            "sh -c \"herdr workspace create --label b\"",
            "bash -c 'cd /tmp && herdr pane split'",
            "eval 'herdr pane split --current'",
            "sudo herdr pane split --current",
            "sudo -u root herdr workspace close w7",
            "env FOO=1 herdr workspace close w7",
            "FOO=bar herdr tab create",
            "command herdr agent start sude-x",
            "xargs herdr pane split",
            "time herdr pane move w1:p5 --new-tab",
            "if herdr workspace close w7; then echo x; fi",
            "echo `herdr tab create`",
            "y=\"$(herdr agent start z)\"",
            "herdr >&2 pane split",
        ],
        Verdict::Block,
        "spellings",
    );
}

#[test]
fn scripts_and_read_only_herdr_calls_are_allowed() {
    assert_cases(
        &[
            "quaqua worker spawn sude-x sude ~/repo",
            "quaqua grid reconcile --tab x-workers --dry-run",
            "herdr agent list",
            "herdr agent read sude-x --lines 80",
            "herdr pane send-keys w1:p5 enter",
            "herdr tab list",
            "herdr workspace list",
            "herdr workspace get w7",
            "quaqua lead cleanup baro",
            "grep -n \"herdr pane split\" crates/quaqua/src/lib.rs",
            "quaqua worker spawn sude-x sude ~/repo",
            "bash -c 'echo no herdr here'",
            "echo $(herdr agent list)",
        ],
        Verdict::Allow,
        "allowed",
    );
}

#[test]
fn argument_text_and_heredocs_are_allowed() {
    assert_cases(
        &[
            "printf '%s\\n' 'example: herdr pane split --current'",
            "printf \"%s\\n\" \"example: herdr pane split --current\"",
            "echo \"see herdr tab create in docs\"",
            "note='herdr agent start'",
            "cat <<< \"herdr pane split\"",
            "var=$(printf \"herdr agent start\")",
            "herdr agent prompt sude-x \"please run herdr pane split\"",
            "echo hi >&2 && herdr agent list",
            "cat <<EOF\nuse herdr pane split to arrange\nEOF",
            "cat <<-EOF\n\therdr agent start sude-x\n\tEOF",
        ],
        Verdict::Allow,
        "argument-text",
    );
    // a real command beside a heredoc is still blocked
    assert_eq!(
        check("cat <<EOF; herdr pane split\nbody\nEOF"),
        Verdict::Block,
        "heredoc with a real command beside it",
    );
}

#[test]
fn help_positions_are_allowed() {
    assert_cases(
        &[
            "herdr pane split --help",
            "herdr pane split -h",
            "herdr pane move --help",
            "herdr pane move -h",
            "herdr tab create --help",
            "herdr tab create -h",
            "herdr workspace create --help",
            "herdr workspace create -h",
            "herdr workspace close --help",
            "herdr workspace close -h",
            "herdr agent start --help",
            "herdr agent start -h",
            "herdr --help",
            "herdr -h",
            "herdr --help pane split",
            "herdr -h tab create",
            "herdr --session quaqua pane split --help",
            "herdr --session quaqua workspace close -h",
            "herdr pane split --help --current",
            "bash -c 'herdr agent start --help'",
            "sudo herdr tab create -h",
            "X=$(herdr pane move -h)",
        ],
        Verdict::Allow,
        "help-position",
    );
}

#[test]
fn help_shaped_words_outside_a_help_position_are_blocked() {
    assert_cases(
        &[
            "herdr pane split --pane --help",
            "herdr pane split --current --help",
            "herdr pane split nosuchpane -h",
            "herdr tab create --label --help",
            "herdr tab create --workspace x -h",
            "herdr workspace close --group --help",
            "herdr agent start --kind --help --pane x",
            "herdr agent start sude-x --kind pi --pane w1:p5 --help",
            "herdr pane split --hel",
            "herdr pane split --helpful",
            "herdr pane split -H",
            "herdr pane split -help",
            "herdr pane split -hx",
            "herdr pane split x -h",
        ],
        Verdict::Block,
        "help-shaped",
    );
}

#[test]
fn empty_substitution_words_still_resolve_the_executable() {
    // Finding 1 (runs/suhuynh-r5r1.md): bash joins words with TAB and re-splits
    // with IFS, so an empty word from an unquoted command substitution vanishes;
    // the executable is still `herdr` and the call is blocked.
    assert_cases(
        &[
            "$(true) herdr pane split",
            "`:` herdr tab create --label x",
            "sudo $(true) herdr workspace close w7",
            "X=1 $(true) herdr agent start x",
        ],
        Verdict::Block,
        "empty-word",
    );
}

#[test]
fn binary_follows_the_hook_json_contract() {
    let bin = env!("CARGO_BIN_EXE_quaqua");
    let run = |json: &str| -> Option<i32> {
        let mut child = Command::new(bin)
            .arg("gate")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn the quaqua binary");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(json.as_bytes())
            .expect("write hook json");
        child.wait().expect("wait").code()
    };
    assert_eq!(
        run(r#"{"tool_name":"Bash","tool_input":{"command":"herdr pane split"}}"#),
        Some(2),
        "blocked command must exit 2",
    );
    assert_eq!(
        run(r#"{"tool_name":"Bash","tool_input":{"command":"herdr agent list"}}"#),
        Some(0),
        "allowed command must exit 0",
    );
    assert_eq!(run(""), Some(0), "empty stdin must exit 0");
    assert_eq!(run("   \n"), Some(0), "whitespace-only stdin must exit 0");
    assert_eq!(
        run("not json"),
        Some(2),
        "unparseable non-empty stdin must fail closed (finding 2)",
    );
}
