use std::path::PathBuf;
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

    let home = std::env::var("HOME").map(PathBuf::from).expect("HOME must be set");
    let live_runs = home.join("quaqua/runs");

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

    let home = std::env::var("HOME").map(PathBuf::from).expect("HOME must be set");
    let live_db = home.join("quaqua/runs/quaqua.db");

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

    let home = std::env::var("HOME").map(PathBuf::from).expect("HOME must be set");
    let live_db = home.join("quaqua/runs/quaqua.db");

    let resolved_db = quaqua::engine::journal_sqlite::SqliteJournal::default_path();

    assert_ne!(
        resolved_db, live_db,
        "SqliteJournal::default_path() under test must resolve to a temp workspace, never live $HOME/quaqua/runs/quaqua.db"
    );
}

#[test]
fn test_live_db_open_refused_under_test() {
    let home = std::env::var("HOME").map(PathBuf::from).expect("HOME must be set");
    let live_db = home.join("quaqua/runs/quaqua.db");

    let open_res = quaqua::bus::FarmBus::open(&live_db);
    assert!(
        open_res.is_err(),
        "opening live workspace database at {live_db:?} must be refused under test"
    );
}
