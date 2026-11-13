# Black Factory

Black Factory is a lights-out software factory. You give the idea and make the design decisions at five gates (idea, requirements, UI, architecture, delivery plan). Agents then build the product checkpoint by checkpoint, and a small Rust engine called `bf` refuses to move on until every designed part is built and proven by a receipt it wrote itself.

## Status

First working version. It has been tested by automated end-to-end tests and has not been used on a real project yet.

- **Built in this version:** the `bf` engine (records, ledger, plan check, evidence receipts, checkpoints, gates, human-turn proof, test mode, guard hook, page data), the Claude Code conductor and worker roles, and the page template.
- **Not built yet:** plugins, the unattended supervisor, opening pull requests, other harnesses and hosts, the UI/UX mode, the security and legal roles, the operations phase, team work.

## What the factory does not stop yet

The guard and the checks stop mistakes, drift and casual cheating, and make deliberate cheating visible. They do not make a determined agent honest. A worker with only the guarded tools can still hide a path from the guard, which reads command text, and a check runs the unit's own code (in a sandbox on macOS only). An agent with an unrestricted shell under your account can still read the engine's key and policy and forge a log. Real separation needs the harness's sandbox and a host identity the agent does not hold, such as a pull request that only you merge. Details: [docs/using.md](docs/using.md), section 14, and [docs/engine.md](docs/engine.md).

## Quick start

```bash
cargo build --release                               # binary: target/release/bf
harness/claude/install.sh /path/to/your/project     # skill, roles, hooks
cd /path/to/your/project && bf init --name myproject   # creates factory/
claude                                              # then type /factory
```

Put `target/release/bf` on your `PATH` first. Your first run: [docs/first-run.md](docs/first-run.md). The full walkthrough: [docs/using.md](docs/using.md).

## Layout

| Path | What |
|---|---|
| `crates/` | The `bf` engine, in Rust |
| `harness/claude/` | The Claude Code skill, worker roles, hook settings and installer |
| `views/` | The page template and the page data contract |
| `docs/first-run.md` | The steps for a first try on your own project |
| `docs/using.md` | How to run a factory |
| `docs/architecture.md` | How the factory is designed, and why |
| `docs/engine.md`, `docs/records.md` | The engine's commands and the record formats |
| `CLAUDE.md` | Rules for working on this repository |

## Licence

Apache-2.0. See [LICENSE](LICENSE). Third-party crates and their licences are listed in `NOTICE.md`. The design was inspired by AI-DLC.
