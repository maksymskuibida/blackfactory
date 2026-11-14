# Notes for test authors

## One test program

Every `tests/*.rs` file is built into one test program, `all` (`tests/all.rs`). `build.rs` lists
the files as modules, so:

- A new test file is picked up by adding it to `tests/`. Do not edit `all.rs`, `build.rs` or `Cargo.toml`.
- A file keeps its name as its module name: the tests of `tests/timing.rs` are named `timing::...`.
- Shared helpers stay in `tests/common/` and are used with `mod common;` as before.

Select one file's tests by the old file name:

```bash
cargo test --package=bf timing::
cargo nextest run --package=bf -E 'test(timing::)'
```

## Run the suite

```bash
cargo test --package=bf            # everything, including documentation tests
cargo nextest run --package=bf     # everything except documentation tests, faster
```

Install nextest once with `cargo install cargo-nextest --locked` (or a prebuilt binary from
https://nexte.st). Settings are in `.config/nextest.toml`; CI uses the `ci` profile
(`--profile ci`). Nextest does not run documentation tests, so CI also runs `cargo test --doc`.

## Measured times

Same machine, whole `bf` package, tests already built (336 tests including the library's own):

| Runner | Before (35 test programs) | After (one test program) |
|---|---|---|
| `cargo test --package=bf` | 101.6 s | 52.5 s |
| `cargo nextest run --package=bf` | 51.4 s | 51.8 s |

Nextest was already parallel across files, so it gains nothing; plain `cargo test` now runs all
tests in parallel and links one program instead of 35, which halves its time. The longest single
test (about 8 s) bounds both.
