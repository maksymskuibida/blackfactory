# Notices

Black Factory is licensed under the Apache License 2.0 (see `LICENSE`).

The engine (`crates/bf`) is original code. It depends on the Rust crates below. Each is used under its permissive licence; where a crate offers a choice, the choice used is the first permissive one listed. The versions in use are pinned in `Cargo.lock`.

## Direct dependencies

| Crate | Used for | Licence |
|---|---|---|
| serde, serde_derive | Reading policy and config into typed values | MIT OR Apache-2.0 |
| serde_json | Records in memory, the run log, hook input and output, page data | MIT OR Apache-2.0 |
| toml | Records, config and policy files | MIT OR Apache-2.0 |
| sha2 | Content hashes, receipt seals, the log's hash chain | MIT OR Apache-2.0 |
| tempfile (tests only) | Temporary git repositories in tests | MIT OR Apache-2.0 |

## Indirect dependencies

Pulled in by the crates above (from `Cargo.lock`):

| Crate | Licence |
|---|---|
| bitflags, block-buffer, cfg-if, cpufeatures, crypto-common, digest, errno, getrandom, itoa, libc, once_cell, proc-macro2, quote, serde_core, serde_spanned, syn, toml_datetime, toml_parser, toml_writer, typenum, windows-link, windows-sys | MIT OR Apache-2.0 |
| equivalent, fastrand, indexmap | Apache-2.0 OR MIT |
| hashbrown | MIT OR Apache-2.0 |
| generic-array, winnow, zmij | MIT |
| memchr | Unlicense OR MIT |
| linux-raw-sys, rustix | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| r-efi | MIT OR Apache-2.0 OR LGPL-2.1-or-later (used under MIT or Apache-2.0) |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 |
| version_check | MIT/Apache-2.0 |

The engine contains no network code and starts no background process. It calls the `git` command-line tool, which is not bundled.
