//! A pipeline prompt that fails before it is sent must leave the run
//! resumable: `pipeline --resume` has to re-send the brief, not wait forever
//! for a report no worker is producing (live case: a pre-prompt grid-check
//! error left `prompted=1` in `runs/pipeline-<task>.state`, so `--resume` took
//! the wait-only path and the watchdog exited 3 while the worker sat idle).
//!
//! Both prompt paths are covered with no real herdr/worker/git:
//! - the `prompt-worker.sh` subprocess path (a fake shim that fails like the
//!   real pre-prompt grid check does, exit 1 before dispatch);
//! - the in-process `worker prompt` path (a fake `herdr` whose `api snapshot`
//!   fails, so the in-process grid check rejects the run before dispatch).
//!
//! The pipeline runs from a hand-written state file in phase `build`, so a
//! `--resume` goes straight to the prompt step.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

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
            root,
        };
        for dir in [&f.runs, &f.repo, &f.scripts, &f.bin] {
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
        fs::write(self.runs.join(format!("pipeline-{task}.md")), format!("# pipeline {task}\n")).unwrap();
        let wt = self.root.join(format!(".wt/sude-{task}"));
        fs::create_dir_all(&wt).unwrap();
        let _ = Command::new("git").args(["init", "-q"]).current_dir(&self.repo).status();
        let _ = Command::new("git").args(["init", "-q"]).current_dir(&wt).status();
        let _ = Command::new("git").args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init", "--allow-empty"]).current_dir(&wt).status();
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
    fn fake_prompt_worker(&self) {
        let calls = self.calls.display();
        let fail = self.fail.display();
        let fail_after_dispatch = self.fail_after_dispatch.display();
        let runs = self.runs.display();
        let script = format!(
            "#!/usr/bin/env bash\n\
             if [[ ${{1:-}} == --resume ]]; then\n\
             \x20 printf '%s\\n' \"$2 resume\" >> {calls}\n\
             \x20 exit 0\n\
             fi\n\
             phase=none\n\
             case \"${{2:-}}\" in\n\
             \x20 *'Pipeline phase: BUILD'*) phase=BUILD ;;\n\
             \x20 *'Pipeline phase: PLAN'*) phase=PLAN ;;\n\
             \x20 *'Pipeline phase: REVIEW'*) phase=REVIEW ;;\n\
             \x20 *'Pipeline phase: FIX'*) phase=FIX ;;\n\
             esac\n\
             printf '%s\\n' \"$1 fresh $phase\" >> {calls}\n\
             if [[ -e {fail} ]]; then\n\
             \x20 echo \"error: grid check failed for $1 (fake) — arrange panes manually before prompting\" >&2\n\
             \x20 exit 1\n\
             fi\n\
             if [[ -e {fail_after_dispatch} ]]; then\n\
             \x20 echo \"error: no report after the wait (fake watchdog)\" >&2\n\
             \x20 exit 3\n\
             fi\n\
             if [[ \"$phase\" == \"PLAN\" ]]; then\n\
             \x20 printf '%s\\n' '# plan' '' 'Goal: test' '' '## Lessons' '- none' > \"{runs}/$1.md\"\n\
             else\n\
             \x20 printf '%s\\n' 'fake build report' 'Outcome: BLOCKED' '' '## Blockers' \\\n\
             \x20   '- fake builder stopped early: this report only proves the prompt was sent' '' \\\n\
             \x20   '## Lessons' '- none' > \"{runs}/$1.md\"\n\
             fi\n\
             exit 0\n"
        );
        let path = self.scripts.join("prompt-worker.sh");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Fake `herdr` whose `api snapshot` fails, so the in-process pre-prompt
    /// grid check rejects the run the way a mis-arranged pane population does.
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
        let calls = self.calls.display();
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
            "#!/usr/bin/env bash\n\
             printf '%s\\n' \"$*\" >> {calls}\n\
             if [[ \"${{1:-}} ${{2:-}}\" == 'api snapshot' ]]; then\n\
             \x20 cat << 'EOF'\n\
{snapshot_json}\n\
EOF\n\
             \x20 exit 0\n\
             fi\n\
             if [[ \"${{1:-}} ${{2:-}}\" == 'pane move' ]]; then\n\
             \x20 echo '{{\"result\":{{\"move_result\":{{\"changed\":true,\"pane\":{{\"pane_id\":\"%b\",\"tab_id\":\"t2\"}}}}}}}}'\n\
             \x20 exit 0\n\
             fi\n\
             exit 0\n"
        );
        let path = self.bin.join("herdr");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
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
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
            )
            .env("HOME", &self.home)
            .env("QUAQUA_HOME", self.home.join("quaqua"))
            .env("QUAQUA_RUNS_DIR", &self.runs);
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
        vec!["sude-pfx1 fresh BUILD", "sude-pfx1 fresh BUILD"],
        "the failed attempt and its re-send must both be fresh BUILD prompts",
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
        calls.iter().any(|c| c.contains("pane move %b --new-tab --label custom-tab --no-focus")),
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
        calls.iter().any(|c| c.contains("pane move %b --new-tab --label repo-workers --no-focus")),
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
    fs::write(f.runs.join(format!("pipeline-{task}.md")), format!("# pipeline {task}\n")).unwrap();

    // Already-delivered reviewer report with findings, but NO srt.json exists (post-cleanup)
    let revname = format!("suhuynh-{task}r1");
    fs::write(f.runs.join(format!("{revname}.md")), "findings\nVerdict: 2 findings\n").unwrap();
    assert!(!f.runs.join(format!("{revname}.srt.json")).exists());

    // Fake prompt-worker.sh: if wait_file is called on the reviewer (--resume),
    // simulate watchdog exit 6 on missing srt.json post-cleanup.
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
