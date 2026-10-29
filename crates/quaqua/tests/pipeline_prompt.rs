//! A pipeline prompt that fails before it is sent must leave the run
//! resumable: `pipeline --resume` has to re-send the brief, not wait forever
//! for a report no worker is producing (live case: a pre-prompt grid-check
//! error left `prompted=1` in `runs/pipeline-<task>.state`, so `--resume` took
//! the wait-only path and the watchdog exited 3 while the worker sat idle).
//!
//! In-process prompt dispatch and pre-prompt grid check are covered with no real
//! herdr/worker/git via fake `herdr`:
//! - pre-prompt grid-check failure rejects the run before dispatch (exit 1 before send);
//! - successful dispatch and report consumption;
//! - watchdog exit keeps dispatched prompt marked sent.
//!
//! The pipeline runs from a hand-written state file in phase `build`, so a
//! `--resume` goes straight to the prompt step.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use quaqua::engine::journal::Journal;
use quaqua::engine::journal_sqlite::SqliteJournal;

#[path = "common/gitfix.rs"]
mod gitfix;

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    runs: PathBuf,
    repo: PathBuf,
    brief: PathBuf,
    scripts: PathBuf,
    bin: PathBuf,
    calls: PathBuf,
    fail: PathBuf,
    fail_after_dispatch: PathBuf,
    /// Playbook index for briefs that carry a `Playbook:` header.
    playbooks: PathBuf,
    principles: PathBuf,
    /// When present the fake herdr copies it over the child's report.
    report_template: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "quaqua-pipeline-prompt-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let f = Fixture {
            home: root.join("home"),
            runs: root.join("home/quaqua/runs"),
            repo: root.join("repo"),
            brief: root.join("brief.md"),
            scripts: root.join("scripts"),
            bin: root.join("bin"),
            calls: root.join("calls"),
            fail: root.join("fail-prompt"),
            fail_after_dispatch: root.join("fail-after-dispatch"),
            playbooks: root.join("playbooks"),
            principles: root.join("principles"),
            report_template: root.join("report-template.md"),
            root,
        };
        for dir in [
            &f.runs,
            &f.repo,
            &f.scripts,
            &f.bin,
            &f.playbooks,
            &f.principles,
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(
            &f.brief,
            "Goal: exercise the pipeline prompt step\n\
             Hard constraints: none\n\
             Current design (open to challenge): fixture\n\
             Unknowns: none\n\
             Evidence: this file\n",
        )
        .unwrap();
        fs::write(&f.calls, "").unwrap();
        f
    }

    fn repo_arg(&self) -> String {
        fs::canonicalize(&self.repo)
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    /// A brief that names a playbook, plus the index entry it resolves in.
    fn write_playbook_brief(&self) {
        fs::write(
            &self.brief,
            "Playbook: fixture\n\
             Context: tests/pipeline_prompt.rs\n\
             Goal: exercise the pipeline prompt step\n\
             Hard constraints: none\n\
             Current design (open to challenge): fixture\n\
             Unknowns: none\n\
             Evidence: this file\n",
        )
        .unwrap();
        fs::write(
            self.playbooks.join("fixture.md"),
            "---\nname: fixture\nwhen: A fixture playbook.\n---\n\n## Orchestrator\n\n1. Approve the plan.\n\n## Worker\n\n1. Reproduce the failure before fixing.\n2. Fix the root cause.\n",
        )
        .unwrap();
    }

    /// A git worktree the build gate can inspect (clean, one commit).
    /// Returns the commit sha the worktree sits at.
    fn init_wt(&self, task: &str) -> String {
        let wt = self.root.join(format!(".wt/sude-{task}"));
        gitfix::init_repo(&wt)
    }

    /// The spawn base the build gate compares HEAD against.
    fn set_base_sha(&self, task: &str, sha: &str) {
        let path = self.runs.join(format!("pipeline-{task}.state"));
        let content = fs::read_to_string(&path)
            .unwrap()
            .replace("base_sha=\n", &format!("base_sha={sha}\n"));
        fs::write(&path, content).unwrap();
    }

    /// A resumable `build`-phase state: the builder exists, and this phase's
    /// prompt has not been sent yet.
    fn write_state(&self, task: &str) {
        let state = format!(
            "repo={}\nbrief={}\nbuilder=sude\nreviewer=suhuynh\nreview=always\nrounds=2\n\
             base=\nwt={}/.wt/sude-{task}\nbase_sha=\nhead_sha=\nplanfile=\ncommit=\n\
             phase=build\nround=0\nprompted=0\nprompt_ts=0\nrounds_run=0\nreview_run=skip\n\
             review_changed=false\nlast_findings=0\nfinished=0\nstart_elapsed=0\n\
             builder_spawned=1\nreviewer_spawned=0\nlead=\nlead_pane=\npipeline_pid=0\n",
            self.repo_arg(),
            self.brief.display(),
            self.root.display(),
        );
        fs::write(self.runs.join(format!("pipeline-{task}.state")), state).unwrap();
    }

    /// The same resumable state in phase `plan`: the plan prompt has not been
    /// sent yet.
    fn write_plan_state(&self, task: &str) {
        self.write_state(task);
        let path = self.runs.join(format!("pipeline-{task}.state"));
        let content = fs::read_to_string(&path)
            .unwrap()
            .replace("phase=build\n", "phase=plan\n");
        fs::write(&path, content).unwrap();
    }

    fn write_spawn_state(&self, task: &str, lead: &str) {
        let state = format!(
            "repo={}\nbrief={}\nbuilder=sude\nreviewer=suhuynh\nreview=always\nrounds=2\n\
             base=\nwt={}/.wt/sude-{task}\nbase_sha=\nhead_sha=\nplanfile=\ncommit=\n\
             phase=spawn\nround=0\nprompted=0\nprompt_ts=0\nrounds_run=0\nreview_run=skip\n\
             review_changed=false\nlast_findings=0\nfinished=0\nstart_elapsed=0\n\
             builder_spawned=1\nreviewer_spawned=0\nlead={lead}\nlead_pane=\npipeline_pid=0\n",
            self.repo_arg(),
            self.brief.display(),
            self.root.display(),
        );
        fs::write(self.runs.join(format!("pipeline-{task}.state")), state).unwrap();
        fs::write(
            self.runs.join(format!("pipeline-{task}.md")),
            format!("# pipeline {task}\n"),
        )
        .unwrap();
        let wt = self.root.join(format!(".wt/sude-{task}"));
        gitfix::git_ok(&self.repo, &["init", "-q", "-b", "main"]);
        gitfix::init_repo(&wt);
    }

    fn state_field(&self, task: &str, key: &str) -> String {
        let content = fs::read_to_string(self.runs.join(format!("pipeline-{task}.state"))).unwrap();
        let prefix = format!("{key}=");
        content
            .lines()
            .find_map(|l| l.strip_prefix(prefix.as_str()))
            .unwrap_or_default()
            .to_string()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.calls)
            .unwrap()
            .lines()
            .map(|l| l.to_string())
            .collect()
    }

    /// Fake `prompt-worker.sh`: a fresh call (`<worker> <prompt>`) "sends" the
    /// prompt by writing the worker's report; `--resume` only waits and writes
    /// nothing. Each call is logged as one line `<worker> <fresh|resume>
    /// <phase>`. The fail marker makes a fresh call exit 1 like the real
    /// command does when its pre-prompt grid check rejects the run — before any
    /// prompt leaves the pipeline; the dispatched-fail marker makes it exit 3
    /// like a watchdog on a prompt that did go out.
    fn install_herdr(&self, owner: &str) -> PathBuf {
        let calls = self.calls.display();
        let fail = self.fail.display();
        let fail_after = self.fail_after_dispatch.display();
        let runs = self.runs.display();
        let report_template = self.report_template.display();
        let snapshot_json = format!(
            r#"{{"snapshot":{{"workspaces":[{{"workspace_id":"ws1","label":"q"}}],
 "tabs":[{{"tab_id":"t1","workspace_id":"ws1","label":"lead-tab","pane_count":3}}],
 "panes":[
   {{"pane_id":"%lead","tab_id":"t1","workspace_id":"ws1","agent_status":"idle",
    "tokens":{{"grid_role":"lead","grid_owner":"{owner}","grid_ready":"1"}}}},
   {{"pane_id":"%a","tab_id":"t1","workspace_id":"ws1","agent_status":"working",
    "tokens":{{"grid_role":"worker","grid_owner":"{owner}","grid_group":"repo","grid_ready":"1"}}}},
   {{"pane_id":"%b","tab_id":"t1","workspace_id":"ws1","agent_status":"idle",
    "tokens":{{"grid_role":"worker","grid_owner":"{owner}","grid_group":"repo","grid_ready":"1"}}}}
 ],
 "layouts":[{{"tab_id":"t1","workspace_id":"ws1","area":{{"x":0,"y":0,"width":200,"height":50}},
             "panes":[{{"pane_id":"%lead"}},{{"pane_id":"%a"}},{{"pane_id":"%b"}}]}}],
 "agents":[{{"name":"{owner}","pane_id":"%lead","agent_status":"idle"}}]}}}}"#
        );
        let script = format!(
            r#"#!/usr/bin/env bash
cmd="${{1:-}} ${{2:-}}"

if [[ "$cmd" == "api snapshot" ]]; then
  if [[ -e "{fail}" ]]; then
    echo "fake herdr: backend down" >&2
    exit 1
  fi
  cat << 'EOF'
{snapshot_json}
EOF
  exit 0
fi

if [[ "$cmd" == "pane move" ]]; then
  printf '%s\n' "$*" >> "{calls}"
  echo '{{"result":{{"move_result":{{"changed":true,"pane":{{"pane_id":"%b","tab_id":"t2"}}}}}}}}'
  exit 0
fi

if [[ "$cmd" == "agent prompt" ]]; then
  worker="$3"
  phase="none"
  case "$*" in
    *'Pipeline phase: BUILD'*) phase="BUILD" ;;
    *'Pipeline phase: PLAN'*) phase="PLAN" ;;
    *'Pipeline phase: REVIEW'*) phase="REVIEW" ;;
    *'Pipeline phase: FIX'*) phase="FIX" ;;
  esac
  printf '%s\n' "$worker fresh $phase" >> "{calls}"
  printf '%s' "$4" > "{runs}/prompt-$worker.txt"
  if [[ -e "{fail_after}" ]]; then
    exit 0
  fi
  if [[ -e "{report_template}" ]]; then
    cp "{report_template}" "{runs}/$worker.md"
    exit 0
  fi
  if [[ "$phase" == "PLAN" ]]; then
    printf '%s\n' 'Outcome: COMPLETE' '# plan' '' 'Goal: test' '' '## Lessons' '- none' > "{runs}/$worker.md"
  else
    printf '%s\n' 'Outcome: BLOCKED' 'fake build report' '' '## Blockers' \
      '- fake builder stopped early: this report only proves the prompt was sent' '' \
      '## Lessons' '- none' > "{runs}/$worker.md"
  fi
  exit 0
fi

if [[ "$cmd" == "agent get" ]]; then
  if [[ -e "{fail_after}" ]]; then
    echo '{{"error":{{"code":"agent_not_found"}}}}'
    exit 0
  fi
  echo '{{"result":{{"agent":{{"agent_status":"working","cwd":"","pane_id":"%b"}}}}}}'
  exit 0
fi

if [[ "$cmd" == "agent read" ]]; then
  exit 0
fi

if [[ "$cmd" == "agent send-keys" ]]; then
  exit 0
fi

printf '%s\n' "$*" >> "{calls}"
exit 0
"#
        );
        let path = self.bin.join("herdr");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn fake_prompt_worker(&self) {
        if !self.bin.join("herdr").exists() {
            self.install_herdr("cos");
        }
    }

    fn fake_herdr(&self) -> PathBuf {
        let calls = self.calls.display();
        let script = format!(
            "#!/usr/bin/env bash\n\
             printf '%s\\n' \"$*\" >> {calls}\n\
             if [[ \"${{1:-}} ${{2:-}}\" == 'api snapshot' ]]; then\n\
             \x20 echo 'fake herdr: backend down' >&2\n\
             \x20 exit 1\n\
             fi\n\
             exit 0\n"
        );
        let path = self.bin.join("herdr");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn fake_herdr_grid(&self, owner: &str) -> PathBuf {
        self.install_herdr(owner)
    }

    /// The pipeline under test (the built binary) in a clean environment: only
    /// the temp home/runs and, when asked, the fake shim/bin dirs are
    /// reachable, so grid skips, fake clocks or team config from the caller
    /// never leak in.
    fn pipeline(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quaqua"));
        cmd.arg("pipeline")
            .current_dir(&self.repo)
            .env_clear()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin.display(),
                    std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string())
                ),
            )
            .env("HERDR_BIN_PATH", self.bin.join("herdr"))
            .env("HOME", &self.home)
            .env("QUAQUA_HOME", self.home.join("quaqua"))
            .env("QUAQUA_RUNS_DIR", &self.runs)
            .env("QUAQUA_PLAYBOOKS_DIR", &self.playbooks)
            .env("QUAQUA_PRINCIPLES_DIR", &self.principles)
            .env("PROMPT_WORKER_AGENT_GONE_SEC", "0")
            .env("PROMPT_WORKER_POLL_SEC", "1")
            .env("GRID_CHECK_TIMEOUT_SEC", "0");
        cmd
    }

    fn resume(&self, task: &str, script_dir: bool) -> Output {
        let mut cmd = self.pipeline();
        if script_dir {
            cmd.env("QUAQUA_SCRIPT_DIR", &self.scripts);
        }
        cmd.args(["--resume", task, &self.repo_arg()])
            .output()
            .expect("failed to spawn the quaqua binary")
    }
}

#[test]
fn pipeline_resume_resends_a_prompt_that_never_went_out() {
    let f = Fixture::new("resend");
    f.write_state("pfx1");
    f.fake_prompt_worker();

    // 1) The prompt command fails before dispatch (pre-prompt grid check).
    fs::write(&f.fail, "").unwrap();
    let out = f.resume("pfx1", true);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failed prompt must stop the pipeline blocked (rc 1): {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        f.state_field("pfx1", "prompted"),
        "0",
        "a prompt that never went out must leave prompted=0 — otherwise --resume only waits",
    );

    // 2) The cause is fixed -> --resume must send the brief again (a fresh
    // prompt, not the wait-only --resume form) and act on the report.
    fs::remove_file(&f.fail).unwrap();
    let out = f.resume("pfx1", true);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--resume must re-send the prompt: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("status=blocked"),
        "the report written by the re-sent prompt must be consumed: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    let calls = f.calls();
    assert_eq!(
        calls,
        vec!["sude-pfx1 fresh BUILD"],
        "the re-sent prompt must be a fresh BUILD prompt",
    );
    assert!(
        f.runs.join("sude-pfx1.build.md").exists(),
        "the re-sent prompt's report must have been parked as the build report",
    );
}

#[test]
fn pipeline_in_process_prompt_check_keeps_the_state_resumable() {
    let f = Fixture::new("inproc");
    f.write_state("pfx2");
    // No prompt-worker.sh anywhere on the search path -> the pipeline prompts
    // in-process; the fake herdr fails the pre-prompt grid check before
    // dispatch, exactly like a mis-arranged pane population.
    let herdr = f.fake_herdr();
    let mut cmd = f.pipeline();
    cmd.env("HERDR_BIN_PATH", &herdr)
        .env("GRID_CHECK_TIMEOUT_SEC", "0");
    let out = cmd
        .args(["--resume", "pfx2", &f.repo_arg()])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(
        out.status.code(),
        Some(1),
        "a pre-prompt grid-check failure must stop the pipeline blocked (rc 1): {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        f.calls().iter().any(|c| c == "api snapshot"),
        "the in-process grid check must be the failure being exercised: {:?}",
        f.calls(),
    );
    assert_eq!(
        f.state_field("pfx2", "prompted"),
        "0",
        "the in-process path must roll the prompt marker back too, so --resume re-sends",
    );
}

#[test]
fn pipeline_watchdog_exit_keeps_a_dispatched_prompt_marked_sent() {
    // The counterpart: once the prompt did go out, a watchdog stop (2–6) must
    // keep prompted=1 so --resume continues that wait instead of re-sending the
    // brief to a worker that is already on it.
    let f = Fixture::new("watchdog");
    f.write_state("pfx3");
    f.fake_prompt_worker();
    fs::write(&f.fail_after_dispatch, "").unwrap();

    let out = f.resume("pfx3", true);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a watchdog stop must propagate (rc 3): {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        f.calls(),
        vec!["sude-pfx3 fresh BUILD"],
        "the prompt must have been sent exactly once",
    );
    assert_eq!(
        f.state_field("pfx3", "prompted"),
        "1",
        "a dispatched prompt must stay marked sent so --resume waits on the live worker",
    );
    assert!(
        f.state_field("pfx3", "prompt_ts") != "0",
        "the recorded prompt time must survive too",
    );
}

#[test]
fn pipeline_grid_hook_uses_lead_default_tab() {
    let f = Fixture::new("leadtab");
    f.write_spawn_state("pfx4", "quaqua-grid-lead");
    let herdr = f.fake_herdr_grid("quaqua-grid-lead");
    f.fake_prompt_worker();

    let mut cmd = f.pipeline();
    cmd.env("HERDR_BIN_PATH", &herdr)
        .env("GRID_CHECK_TIMEOUT_SEC", "0")
        .env("QUAQUA_SCRIPT_DIR", &f.scripts);
    let out = cmd
        .args(["--resume", "pfx4", &f.repo_arg()])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(
        out.status.code(),
        Some(5),
        "pipeline should pause at plan-ready (rc 5): stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let calls = f.calls();
    assert!(
        calls.iter().any(|c| c.contains("pane move %b --new-tab --label quaqua-grid-workers --no-focus")),
        "expected grid hook to arrange workers into Lead's tab 'quaqua-grid-workers', but calls were: {:?}",
        calls,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("PIPELINE plan-ready"),
        "grid hook should have succeeded and pipeline reached plan-ready, but stderr was: {stderr}",
    );
    assert_eq!(f.state_field("pfx4", "phase"), "build");
}

#[test]
fn pipeline_grid_hook_explicit_grid_tab_wins() {
    let f = Fixture::new("exptab");
    f.write_spawn_state("pfx5", "quaqua-grid-lead");
    let herdr = f.fake_herdr_grid("quaqua-grid-lead");
    f.fake_prompt_worker();

    let mut cmd = f.pipeline();
    cmd.env("HERDR_BIN_PATH", &herdr)
        .env("GRID_CHECK_TIMEOUT_SEC", "0")
        .env("GRID_TAB", "custom-tab")
        .env("QUAQUA_SCRIPT_DIR", &f.scripts);
    let out = cmd
        .args(["--resume", "pfx5", &f.repo_arg()])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(out.status.code(), Some(5));
    let calls = f.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("pane move %b --new-tab --label custom-tab --no-focus")),
        "explicit GRID_TAB must override lead default tab: {:?}",
        calls,
    );
}

#[test]
fn pipeline_grid_hook_cos_uses_repo_default_tab() {
    let f = Fixture::new("costab");
    f.write_spawn_state("pfx6", "");
    let herdr = f.fake_herdr_grid("cos");
    f.fake_prompt_worker();

    let mut cmd = f.pipeline();
    cmd.env("HERDR_BIN_PATH", &herdr)
        .env("GRID_CHECK_TIMEOUT_SEC", "0")
        .env("QUAQUA_SCRIPT_DIR", &f.scripts);
    let out = cmd
        .args(["--resume", "pfx6", &f.repo_arg()])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(out.status.code(), Some(5));
    let calls = f.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("pane move %b --new-tab --label repo-workers --no-focus")),
        "cos owner must retain repo default tab: {:?}",
        calls,
    );
}

#[test]
fn pipeline_resume_with_rounds_raises_cap() {
    let f = Fixture::new("capraise");
    f.write_state("pfx7");
    f.fake_prompt_worker();

    let mut cmd = f.pipeline();
    cmd.env("QUAQUA_SCRIPT_DIR", &f.scripts)
        .env("GRID_SKIP", "1")
        .env("QUAQUA_TEST", "1");
    let _ = cmd
        .args(["--resume", "pfx7", &f.repo_arg(), "--rounds", "5"])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(
        f.state_field("pfx7", "rounds"),
        "5",
        "resuming with --rounds 5 must raise state.rounds to 5",
    );
}

#[test]
fn pipeline_resume_at_phase_review_with_delivered_report_avoids_exit_6() {
    let f = Fixture::new("revdeliv");
    let task = "pfx8";
    let state = format!(
        "repo={}\nbrief={}\nbuilder=sude\nreviewer=suhuynh\nreview=always\nrounds=1\n\
         base=\nwt={}/.wt/sude-{task}\nbase_sha=\nhead_sha=\nplanfile=\ncommit=\n\
         phase=review\nround=1\nprompted=1\nprompt_ts=0\nrounds_run=0\nreview_run=run\n\
         review_changed=false\nlast_findings=0\nfinished=0\nstart_elapsed=0\n\
         builder_spawned=1\nreviewer_spawned=1\nlead=\nlead_pane=\npipeline_pid=0\n",
        f.repo_arg(),
        f.brief.display(),
        f.root.display(),
    );
    fs::write(f.runs.join(format!("pipeline-{task}.state")), state).unwrap();
    fs::write(
        f.runs.join(format!("pipeline-{task}.md")),
        format!("# pipeline {task}\n"),
    )
    .unwrap();

    // Already-delivered reviewer report with findings (post-cleanup)
    let revname = format!("suhuynh-{task}r1");
    fs::write(
        f.runs.join(format!("{revname}.md")),
        "findings\nVerdict: 2 findings\n",
    )
    .unwrap();

    // Fake prompt-worker.sh: if wait_file is called on the reviewer (--resume),
    // simulate watchdog exit 6 on a malformed report post-cleanup.
    let psh = f.scripts.join("prompt-worker.sh");
    let script = "#!/usr/bin/env bash\n\
if [ \"$1\" = \"--resume\" ]; then\n\
    exit 6\n\
fi\n\
exit 0\n";
    fs::write(&psh, script).unwrap();
    fs::set_permissions(&psh, fs::Permissions::from_mode(0o755)).unwrap();

    // Resuming must bypass wait_file, consume the delivered report, and exit 0 (rounds exhausted),
    // never exit 6 (watchdog).
    let mut cmd = f.pipeline();
    cmd.env("QUAQUA_SCRIPT_DIR", &f.scripts)
        .env("GRID_SKIP", "1")
        .env("QUAQUA_TEST", "1");
    let out = cmd
        .args(["--resume", task, &f.repo_arg()])
        .output()
        .expect("failed to spawn the quaqua binary");

    assert_eq!(
        out.status.code(),
        Some(0),
        "resuming with delivered report must consume report and exit 0: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        f.state_field(task, "last_findings"),
        "2",
        "findings count from delivered report must be recorded",
    );
    assert_eq!(
        f.state_field(task, "rounds_run"),
        "1",
        "rounds_run must be recorded as 1",
    );
    let pfile = fs::read_to_string(f.runs.join(format!("pipeline-{task}.md"))).unwrap();
    assert!(
        pfile.contains("- review round 1: Verdict: 2 findings"),
        "pipeline file must record the delivered review verdict: {pfile}",
    );
}

#[test]
fn pipeline_per_phase_budget_reset_grants_fresh_allowance() {
    let f = Fixture::new("budget-reset");
    let task = "pfx9";
    f.write_state("pfx9");
    f.fake_prompt_worker();
    let state = format!(
        "repo={}\nbrief={}\nbuilder=sude\nreviewer=suhuynh\nreview=always\nrounds=2\n\
         base=\nwt={}/.wt/sude-{task}\nbase_sha=\nhead_sha=\nplanfile=\ncommit=\n\
         phase=build\nround=0\nprompted=0\nprompt_ts=0\nrounds_run=0\nreview_run=skip\n\
         review_changed=false\nlast_findings=0\nfinished=0\nstart_elapsed=0\n\
         builder_spawned=1\nreviewer_spawned=0\nlead=\nlead_pane=\npipeline_pid=0\n",
        f.repo_arg(),
        f.brief.display(),
        f.root.display(),
    );
    fs::write(f.runs.join(format!("pipeline-{task}.state")), state).unwrap();
    fs::write(
        f.runs.join(format!("pipeline-{task}.md")),
        format!("# pipeline {task}\n"),
    )
    .unwrap();

    // Resume pipeline at phase build
    let out = f.resume(task, false);

    // Verify per-phase budget reset:
    // Fresh phase receives its own fresh budget allowance (2700s default) without inheriting starving budget
    eprintln!(
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("budget 2700s"),
        "child input must receive fresh phase budget (2700s), got: {stderr}"
    );
}

/// § S4 parity: a pipeline-spawned builder gets the same playbook path as
/// `worker prompt` — the pinned steps reach the prompt it is sent, and
/// `runs/.<name>.playbook` records what that worker actually received.
#[test]
fn pipeline_builder_prompt_injects_and_pins_the_brief_playbook() {
    let f = Fixture::new("pbpin");
    let task = "pbpin";
    f.write_playbook_brief();
    f.write_state(task);
    f.fake_prompt_worker();

    let out = f.resume(task, false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the playbook path must not change the pipeline exit: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let body = fs::read_to_string(f.runs.join(format!("prompt-sude-{task}.txt")))
        .expect("the fake herdr must have received the prompt body");
    assert!(
        body.contains("## Worker (playbook: fixture)"),
        "the builder must be sent the steps of the playbook its brief names: {body}"
    );
    assert!(
        body.contains("1. Reproduce the failure before fixing."),
        "prompt: {body}"
    );

    assert_eq!(
        fs::read_to_string(f.runs.join(format!(".sude-{task}.playbook")))
            .ok()
            .as_deref(),
        Some("name=fixture\nids=1,2\n"),
        "the pipeline builder must pin the playbook it was given",
    );
}

/// § S5 parity: an accepted pipeline builder report that does not account for
/// the pinned steps journals one `playbook_gap` — warn only, the pipeline
/// still finishes clean.
#[test]
fn pipeline_builder_gap_is_warned_on_acceptance() {
    let f = Fixture::new("pbgap");
    let task = "pbgap";
    f.write_playbook_brief();
    f.write_state(task);
    let base = f.init_wt(task);
    f.set_base_sha(task, &base);
    fs::write(
        &f.report_template,
        "# Build\n\nOutcome: no changes\n\n## Changes\n\nnothing to change\n\n## Lessons\nnone\n",
    )
    .unwrap();
    fs::write(
        f.runs.join(format!("pipeline-{task}.md")),
        format!("# pipeline {task}\n"),
    )
    .unwrap();
    f.fake_prompt_worker();

    let out = f.resume(task, false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a playbook gap must not change the pipeline exit: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let journal = SqliteJournal::open(&f.runs.join("quaqua.db")).expect("pipeline journal");
    let gaps: Vec<serde_json::Value> = journal
        .load_events(&format!("worker:sude-{task}"), 1)
        .expect("events")
        .into_iter()
        .filter(|e| e.kind == "playbook_gap")
        .map(|e| serde_json::from_str(&e.payload_json).expect("gap payload"))
        .collect();
    assert_eq!(
        gaps.len(),
        1,
        "the accepted builder report's gap must be journaled: {gaps:?}"
    );
    assert_eq!(gaps[0]["name"], "fixture");
    assert_eq!(gaps[0]["missing"], serde_json::json!([1, 2]));
}

/// § S4/S5 plan side: a plan worker writes only the plan, so its prompt carries
/// no `## Worker (playbook: …)` steps and nothing is pinned for it — the same
/// skip reviewers get. The build phase still gets both.
#[test]
fn pipeline_plan_prompt_carries_no_worker_steps_and_no_pin() {
    let f = Fixture::new("pbplan");
    let task = "pbplan";
    f.write_playbook_brief();
    f.write_plan_state(task);
    f.fake_prompt_worker();

    let out = f.resume(task, false);
    assert_eq!(
        out.status.code(),
        Some(5),
        "the plan must still pause at plan-ready: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        f.state_field(task, "phase"),
        "build",
        "plan-ready must hand over to the build phase",
    );

    let body = fs::read_to_string(f.runs.join(format!("prompt-sude-{task}.txt")))
        .expect("the fake herdr must have received the plan prompt body");
    assert!(
        body.contains("Pipeline phase: PLAN"),
        "the prompt under test must be the plan prompt: {body}"
    );
    assert!(
        !body.contains("## Worker (playbook: fixture)"),
        "a plan prompt must not carry the worker steps: {body}"
    );
    assert!(
        !body.contains("1. Reproduce the failure before fixing."),
        "plan prompt: {body}"
    );
    assert!(
        !f.runs.join(format!(".sude-{task}.playbook")).exists(),
        "a plan worker must not be pinned a playbook: S5 would then demand steps it never got",
    );
}
