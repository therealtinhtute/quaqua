//! Project Lead operations (`quaqua lead {spawn, cleanup}`).

pub mod cleanup;
pub mod spawn;

use crate::git::Git;
use crate::herdr::Herdr;

pub fn handle_lead(args: &[String], git: &Git, herdr: &Herdr) -> Result<(), (i32, String)> {
    if args.is_empty() {
        return Err((1, format!("{}\n", spawn::USAGE)));
    }
    match args[0].as_str() {
        "spawn" => spawn::spawn_lead(&args[1..], git, herdr),
        "cleanup" => cleanup::cleanup_lead(&args[1..], herdr),
        _ => Err((1, format!("{}\n", spawn::USAGE))),
    }
}
