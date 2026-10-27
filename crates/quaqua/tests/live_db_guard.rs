use std::sync::Mutex;

static GUARD_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let lock = GUARD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let vars = keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        Self { _lock: lock, vars }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[test]
fn test_default_runs_dir_under_test_never_resolves_to_live_workspace() {
    let _guard = EnvGuard::new(&["QUAQUA_RUNS_DIR", "QUAQUA_HOME", "QUAQUA_ROOT"]);
    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_HOME");
    std::env::remove_var("QUAQUA_ROOT");

    // The live workspace is defined by the passwd home, not by `$HOME`: a run
    // with HOME overridden (CI isolation, a fixture) is not the live one.
    let live_runs = quaqua::workspace::live_workspace_runs().expect("system user home");

    let resolved_runs = quaqua::worker::cleanup::default_runs_dir();

    assert_ne!(
        resolved_runs, live_runs,
        "default_runs_dir() under test must resolve to a temp workspace, never live $HOME/quaqua/runs"
    );
}

#[test]
fn test_farm_bus_default_path_never_resolves_to_live_db() {
    let _guard = EnvGuard::new(&["QUAQUA_RUNS_DIR", "QUAQUA_HOME", "QUAQUA_ROOT"]);
    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_HOME");
    std::env::remove_var("QUAQUA_ROOT");

    let live_db = quaqua::workspace::live_workspace_db().expect("system user home");

    let resolved_db = quaqua::bus::FarmBus::default_path();

    assert_ne!(
        resolved_db, live_db,
        "FarmBus::default_path() under test must resolve to a temp workspace, never live $HOME/quaqua/runs/quaqua.db"
    );
}

#[test]
fn test_sqlite_journal_default_path_never_resolves_to_live_db() {
    let _guard = EnvGuard::new(&["QUAQUA_RUNS_DIR", "QUAQUA_HOME", "QUAQUA_ROOT"]);
    std::env::remove_var("QUAQUA_RUNS_DIR");
    std::env::remove_var("QUAQUA_HOME");
    std::env::remove_var("QUAQUA_ROOT");

    let live_db = quaqua::workspace::live_workspace_db().expect("system user home");

    let resolved_db = quaqua::engine::journal_sqlite::SqliteJournal::default_path();

    assert_ne!(
        resolved_db, live_db,
        "SqliteJournal::default_path() under test must resolve to a temp workspace, never live $HOME/quaqua/runs/quaqua.db"
    );
}

#[test]
fn test_live_db_open_refused_under_test() {
    let _guard = EnvGuard::new(&[]);
    let live_db = quaqua::workspace::live_workspace_db().expect("system user home");

    let open_res = quaqua::bus::FarmBus::open(&live_db);
    assert!(
        open_res.is_err(),
        "opening live workspace database at {live_db:?} must be refused under test"
    );
}

#[test]
fn test_live_db_detected_even_when_home_is_overridden_first() {
    let _guard = EnvGuard::new(&["HOME"]);
    let original_home = quaqua::workspace::system_user_home().expect("system user home");
    let live_db = original_home.join("quaqua/runs/quaqua.db");

    // Override HOME to a fake temporary location first
    std::env::set_var("HOME", "/tmp/fake-home-for-order-test");

    // System user home must be independent of the mutated HOME environment variable
    let sys_home =
        quaqua::workspace::system_user_home().expect("system user home must be available");
    assert_eq!(
        sys_home, original_home,
        "system_user_home() must resolve real passwd home regardless of HOME env"
    );
    assert!(
        quaqua::workspace::is_live_workspace_db(&live_db),
        "live DB must still be recognized as live even when HOME was mutated to a temp directory"
    );
}
