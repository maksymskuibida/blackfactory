# Working on Black Factory

Black Factory is a lights-out software factory: a Rust engine (`bf`) plus instructions for Claude Code. Read `README.md` first, then `docs/architecture.md`.

## This repository is public

Anything committed here is read by strangers. Never commit:

- names of people, or of private projects and products
- absolute paths into anyone's home directory, e-mail addresses, session ids
- links to private pages or artifacts, tokens, keys, `.env` files
- text copied from private requests, notes or planning files

Keep private notes out of the repository. Write as if the reader knows nothing about who built this.

## How changes land

- Work on a branch. Never commit to `main`.
- Every change goes through a pull request. The CI checks must pass before merge.
- One concern per pull request. Say what changed and how you checked it.
- A refusal from `bf` is a design feature (what, why, steps, how to check). Do not weaken a check to make a test pass.

## Build and test

```bash
CARGO_BUILD_JOBS=4 cargo build --release   # binary: target/release/bf
CARGO_BUILD_JOBS=4 cargo test              # all tests, including attack and timing tests
cargo fmt --check && cargo clippy          # style
```

Do not install anything globally. The engine makes no network call except `cargo` fetching crates. Rust crates must be permissively licensed (MIT, Apache-2.0, BSD or similar) and listed in `NOTICE.md`.

## Where things live

| Path | What |
|---|---|
| `crates/` | The `bf` engine |
| `harness/claude/` | Conductor skill, builder, reviewer, verifier, hook settings, `install.sh` |
| `views/` | Page template and the page data contract |
| `docs/` | `first-run.md`, `using.md`, `architecture.md`, `engine.md`, `records.md` |

Rules belong in the engine, which refuses. The harness instructions only say how to call it. Target T-7 keeps `harness/claude/skills/**` and `harness/claude/agents/**` under 500 lines in total: count them with `wc -l` before you add to them.

## Wording

Everything a human reads (docs, pages, messages from the engine, refusals) is plain, direct English. No em dashes. One sentence per item, detail behind a fold. Do not put user-facing copy in code as a fallback: it belongs with the page or record data.

## Changing the design

`docs/architecture.md` is the design of record. A change to a decision there needs a pull request that says what changed and why, and updates the targets and tests that depend on it.
