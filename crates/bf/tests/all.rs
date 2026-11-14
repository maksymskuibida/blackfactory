//! The one test program. `build.rs` lists every other file in `tests/` as a module here,
//! so each old test file is a module of the same name and `cargo test timing::` still selects it.

// Each test file declares `mod common;`, so the helpers are compiled once per file by design.
#![allow(clippy::duplicate_mod)]

include!(concat!(env!("OUT_DIR"), "/test_modules.rs"));
