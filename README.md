# Black Factory

Black Factory is a lights-out software factory. You give the idea and make the design decisions at five gates (idea, requirements, UI, architecture, delivery plan). Agents then build the product checkpoint by checkpoint, and a small Rust engine called `bf` refuses to move on until every designed part is built and proven by a receipt it wrote itself.

## Status

First working version. It has been tested by automated end-to-end tests and has not been used on a real project yet.

- **Built in this version:** the `bf` engine (records, ledger, plan check, evidence receipts, checkpoints, gates, human-turn proof, test mode, guard hook, page data), the Claude Code conductor and worker roles, and the page template.

## What the factory checks

The engine makes sure every designed part is built and proven and nothing is skipped: it refuses to land a unit, close a checkpoint or finish a run until each part has a receipt it wrote itself, and it records your decisions in your own words. It is the way to make sure nothing is forgotten. Claude Code's own permission checks stay on and are the layer against deceptive behaviour; the factory does not claim to stop a determined agent.

Not built yet: plugins, the unattended supervisor, opening pull requests, other harnesses and hosts, the UI/UX mode, the security and legal roles, the operations phase, team work. Details: [docs/using.md](docs/using.md), sections 14 and 15.

## Quick start

```bash
cargo build --release                                  # binary: target/release/bf
harness/claude/install.sh /path/to/your/project        # skill, roles, hooks
cd /path/to/your/project && bf init --name myproject   # creates factory/
git add -A && git commit -m "Add Black Factory"        # then start Claude Code here and type /factory
```

Put `target/release/bf` on your `PATH` first, and give the project one commit before the installer. The questions and gate pages are published as private pages through the Claude app's artifact feature, which needs a Claude app that supports it. Without it the conductor asks in chat instead. Your first run: [docs/first-run.md](docs/first-run.md). The full walkthrough: [docs/using.md](docs/using.md).

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

Apache-2.0. See [LICENSE](LICENSE). Third-party crates and their licences are listed in `NOTICE.md`. The design was inspired by AI-DLC (https://github.com/awslabs/aidlc-workflows).
