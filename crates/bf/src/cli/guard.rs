//! `bf guard`: the hook that checks a tool call before it runs.

use super::{read_stdin, Args};
use crate::guard;
use crate::refusal::R;
use std::path::PathBuf;

pub(crate) fn cmd_guard(args: &Args) -> R<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let root = args
        .flag("root")
        .map(|x| x.to_string())
        .or_else(|| std::env::var("BF_ROOT").ok());
    let o = guard::run_hook(&read_stdin(), &cwd, root.as_deref());
    println!("{o}");
    Ok(())
}
