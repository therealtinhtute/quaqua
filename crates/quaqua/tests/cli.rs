//! One smoke test for the R0 stub: the built binary answers `--version` and
//! `--help`, which is all it is allowed to do before later phases add commands.
//! `CARGO_BIN_EXE_quaqua` is the path cargo builds for this crate's binary, so
//! the test never hard-codes a machine-specific path.

use std::process::Command;

#[test]
fn version_and_help_are_available() {
    let bin = env!("CARGO_BIN_EXE_quaqua");

    let version = Command::new(bin)
        .arg("--version")
        .output()
        .expect("failed to spawn the quaqua binary");
    assert!(version.status.success(), "--version exited non-zero");
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("quaqua {}", env!("CARGO_PKG_VERSION")),
    );

    let help = Command::new(bin)
        .arg("--help")
        .output()
        .expect("failed to spawn the quaqua binary");
    assert!(help.status.success(), "--help exited non-zero");
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_text.contains("Usage: quaqua"),
        "--help does not print a usage line: {help_text}",
    );
}
