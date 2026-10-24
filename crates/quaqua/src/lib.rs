//! `quaqua` library surface. The clap binary in `main.rs` is a thin shell over
//! this module set, which lets tests call load/resolve/validate/get in-process.

pub mod config;
pub mod workspace;
pub mod gate;
pub mod init;
pub mod hash;
pub mod git;
pub mod herdr;
pub mod cap;
pub mod harness;
pub mod launch;
pub mod worker;
pub mod grid;
pub mod lead;
pub mod liveness;
pub mod taskforce;
pub mod merge;
pub mod pipeline;
pub mod util;
pub mod state;
pub mod usage;
pub mod auditor;
pub mod engine;
pub mod status;
pub mod doctor;
pub mod playbook;
pub mod identity;
pub mod cos;
pub mod layout;

/// Process-global env is shared by every test in this binary; tests that
/// mutate it take [`test_env::lock_env`] and restore through its guard.
#[cfg(test)]
pub(crate) mod test_env;
