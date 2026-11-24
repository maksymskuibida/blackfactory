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

## Two suites: default and extended

A timing test is a test that holds a time limit. The default suite leaves timing tests out, so a
busy machine never fails a unit's check. The extended suite is the default suite plus the timing
tests, each run alone.

```bash
cargo nextest run --package=bf                     # default suite, no timing tests
cargo nextest run --package=bf --profile extended  # extended suite, run at checkpoints on a quiet machine
```

A test becomes a timing test in one of three ways: it lives in `tests/timing.rs`, its name begins
with `timing_`, or `.config/nextest.toml` names it (both the default filter and the extended
override). That file is the one place that says which tests are timing tests. Never run a timing
test or the extended suite while building a unit.

## The sandbox in fixtures

`Fx::new()`, `Fx::with_checks()` and `Fx::workspace(..)` run the engine with the sandbox off, unless `BF_TEST_SANDBOX=all` is set, which turns it on in every fixture.
A test that needs the sandbox uses `Fx::sandboxed()`, `Fx::with_checks_sandboxed()`, `Fx::workspace_sandboxed(..)` or `Fx::shipped()`, which always have it on.
A test that asserts how a receipt ran fixes the switch with `Fx::new_with(Switch::Off)` (or `Switch::All`) instead of reading the environment.

## Measured times

Same machine, whole `bf` package, tests already built (336 tests including the library's own):

| Runner | Before (35 test programs) | After (one test program) |
|---|---|---|
| `cargo test --package=bf` | 101.6 s | 52.5 s |
| `cargo nextest run --package=bf` | 51.4 s | 51.8 s |

Nextest was already parallel across files, so it gains nothing; plain `cargo test` now runs all
tests in parallel and links one program instead of 35, which halves its time. The longest single
test (about 8 s) bounds both.
