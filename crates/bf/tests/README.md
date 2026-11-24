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

## Fixtures are copied from templates

`Fx::new()`, `Fx::with_checks()`, `Fx::workspace(..)` and their `_with` and sandboxed forms copy a project built once by a real `git init` and `bf init`.
The templates live in `target/tmp/bf-fixture-templates`, one per key: the `bf` binary's identity (one `stat`), the text of `common/mod.rs`, the constructor's kind and its policy, the sandbox switch included.
A copy holds the store's lock shared; only putting a finished template in place and removing a template of another binary hold it exclusive, and one key is built by one test under that key's own build lock.
The build folder is removed after the template is taken, so a copy holds no path of it outside the log, its head, the receipts and the guard table; the copy's policy names its own transcript folder and is accepted through the engine with the human's words.
A copy's log therefore holds the events of that acceptance more than a real init's: the human's turn (`human.turn`), the session note it brings (`absent.session`, where none was recorded yet) and `policy.accept`.
The files the acceptance changes that the template had committed (the policy, and for `with_checks` the log) join the commit of `bf init`, so a copy's git tree is as clean or as changed as a real init's.
A test about `bf init`, project ids, the key, the policy's acceptance or the first events of the log, or one that fails on a copy, uses `Fx::new_real()`, `Fx::with_checks_real()` or `Fx::workspace_real(..)` (each with a `_with` form for the switch).
Edit `common/mod.rs` and every template is built again, since its text is part of the key.

`Fx::tiny_project()` and `Fx::tiny_run()` (each with a `_with` form for the switch and a `_sandboxed` form) copy two more templates of the same store: one built by `Fx::new()` and `tiny()`, one by `Fx::new()`, `tiny()` and `start_test_run()`, through those same calls.
A test whose first steps are those calls uses them; a test that does anything between `Fx::new()` and `tiny()` keeps the calls.
The tests of record puts and of the test-mode grant (`e2e`, `flow`, `run_questions`, and the grant and run-start tests of `attacks`, `behaviour` and `hardening`) keep calling `tiny()` and `start_test_run()` on a fresh fixture.
Every fixture repository has git's automatic maintenance switched off (`gc.auto = 0`), so no detached `git gc` writes in a build folder while it is copied.
On a copied run the policy's acceptance lands in the records commit, and the run's own changes stay uncommitted, as after the calls.

## Lifecycle states are copied too

`Fx::state_copy(name, build)` (and `Fx::state_copy_with(name, switch, build)`) gives a state of a run, units started, built or landed, built once per test program and key by a build function of the test file on `Fx::new()`, and copied per test.
The states live in `target/tmp/bf-fixture-states`, keyed like the templates plus the test program's identity and the build function's `name`, under the same locks.
The build first points the policy's transcript root at `<key>.transcripts` in that store, since every working copy checks out the policy; each copy writes its transcript in a folder of its own below it.
The build folder is removed after the state is taken, so every absolute path a copy still holds names a folder that no longer exists, and an engine that reads an old path fails the test.
After each copy the helper runs `git worktree repair` with every working copy's path, writes the signed guard table again with `bf init`, writes again with `bf brief` each unit brief that names the build folder, and runs the canary (`state_canary`), which fails the test on any finding: a worktree path outside the copy, `bf log --verify` failing, or a file holding the build folder's path other than the sealed log, its head and the receipts (in the main checkout and as each working copy checks them out).
A file that builds states keeps at least one test that builds its state through the full `bf` calls, and adds each state that follows the environment's switch to `fixture_state_copy::required_sandbox_is_met_for_state_copy`.
A script a state commits finds its folders from where it runs, never by an absolute path.

## Measured times

Same machine, whole `bf` package, tests already built (336 tests including the library's own):

| Runner | Before (35 test programs) | After (one test program) |
|---|---|---|
| `cargo test --package=bf` | 101.6 s | 52.5 s |
| `cargo nextest run --package=bf` | 51.4 s | 51.8 s |

Nextest was already parallel across files, so it gains nothing; plain `cargo test` now runs all
tests in parallel and links one program instead of 35, which halves its time. The longest single
test (about 8 s) bounds both.
