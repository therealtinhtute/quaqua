//! `quaqua` library surface. The clap binary in `main.rs` is a thin shell over
//! this module set, which lets tests call load/resolve/validate/get in-process.

pub mod config;
pub mod models;
