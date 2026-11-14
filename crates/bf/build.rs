//! Lists every test file in `tests/` so that they build into one test program (`tests/all.rs`).
//! A new test file is picked up here without editing a shared file.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let tests = manifest.join("tests");
    println!("cargo:rerun-if-changed=tests");
    println!("cargo:rerun-if-changed=build.rs");

    let mut names: Vec<String> = std::fs::read_dir(&tests)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rs"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|n| n != "all")
        .collect();
    names.sort();

    let mut out = String::new();
    for n in &names {
        let path = tests.join(format!("{n}.rs"));
        writeln!(out, "#[path = {:?}]\nmod {n};", path.to_string_lossy()).unwrap();
    }
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("test_modules.rs");
    std::fs::write(dest, out).unwrap();
}
