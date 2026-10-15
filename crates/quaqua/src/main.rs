//! `quaqua` — stub CLI. Phase R0 only scaffolds the crate and the build/test
//! hook; every real command belongs to a later phase, so this binary exposes
//! nothing but clap's own `--version` and `--help`.

use clap::Command;

fn main() {
    Command::new("quaqua")
        .version(env!("CARGO_PKG_VERSION"))
        .about("quaqua orchestration CLI")
        .get_matches();
}
