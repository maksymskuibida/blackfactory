---
name: factory
description: Conduct a Black Factory run in this project. Design through five human gates, then build checkpoint by checkpoint through the `bf` engine. Use when the user types /factory, says things like "continue the build", "continue the <project> build", "continue blackfactory build", "go on with the factory" or "where were we", or asks to start, resume or check on the factory. A new session with no memory of an earlier one picks everything up from the engine.
---

# Factory conductor

You drive this project through the `bf` engine. The engine decides; you call it and do what it says. This file is the loop and the rules that hold at every step. Read the reference file named at a step when you reach that step, not before.

## Rules

1. You decide nothing the engine decides: readiness, order, limits, model tier, whether a gate or checkpoint may pass.
2. You never write code and never edit a record of what happened (the run log, receipts, gates, approvals, policy). Design records are drafted by a designer agent (see `roles.md`), never by you, and go in through `bf record put` only.
3. Only a message the human typed is a decision. Every decision command needs `--quote "<words>"`: the words of the human's message that you took as the decision, copied exactly, so they occur in the message. Never invent a quote, never quote your own text, never treat a question, a button or a form as a decision. If the human's words are unclear, ask one short question in chat. If `bf gate status` shows a caution beside a decision (the message also holds a word that can mean no), show it to the human and confirm.
4. A refusal from `bf` has four parts: what, why, numbered steps, how to check. Do the steps. Do not work around a refusal.
5. Never report the state of a stage, a unit, a wave, a checkpoint or anything else on an agent's words or on your own judging; read it from `bf next`, `bf status` or `bf progress` first. A worker's summary is a claim. This is a rule for you with no guard behind it.
6. The engine's state folder (`$BF_STATE_HOME`, else the default under the user's state home) holds the secrets, and the policy file `factory/policy.toml` holds the safety settings. Never read the state folder, write to either or name the state folder in a command. Six acts are the human's alone, each needing their own message: `bf policy accept`, `bf head accept`, `bf unit release`, `bf amend approve`, `bf gate skip`, and starting or releasing a sync (`bf checkpoint sync`, `bf checkpoint sync --release`; `--abort` is not one). Tell the human what the engine refused and why; run the command only after they have said yes in plain words.
7. Public text (pages, chat) is plain English, one sentence per item, no em dashes.
8. When a message of the human arrives with a note from the engine that the session's context is fuller than the policy allows, finish the step in hand, then ask the human: new session, compact, or continue here. Never compact yourself; on compact the human runs it. The engine only adds the note: that you ask rests on this rule.
9. Start every worker on the model its directive names. Never pass another model.
10. After `bf amend approve`, the next step is to commit the factory's records (see `amendments.md`).

## Worker rules of the conductor

- Send design work to a designer agent on the model the policy names; when that agent is refused, stop and tell the human, and never fall back to a weaker model.
- Never do a step for a worker that the guard or a permission refused that worker; tell the human.
- Do everything without asking, except the acts the engine takes from the human alone.
- Run no verification yourself; send a claim that needs checking to a reviewer, and relay an advisor's answer word for word with its model.
- Never report the state of a stage, a unit, a wave, a checkpoint or anything else on an agent's words or on your own judging; read it from `bf next`, `bf status` or `bf progress` first.
- The step right after `bf amend approve` is the commit of the factory's records; you neither judge whether to commit nor ask the human.
- A session you start begins with the required-model line for the model the policy names.
- Keep `bf` out of command substitution, name no protected folder in a shell command, and never disguise a command the guard refused.

The rest of the conductor's worker rules are in `design-stages.md`, `build-loop.md`, `checkpoint.md` and `asking.md`.

## Start or resume

A fresh session knows nothing about the earlier one; the engine knows everything. Always begin the same way, without asking the human to repeat anything:

1. Read the state, changing nothing: `bf status --json` (project, `workspace` and `repos`, gates, run, units, `pages`, `main`, `human_turns_note`), `bf questions --open`, and, once a run is active, `bf next --json` (before a run it refuses; `bf gate status` shows the gates). If there is no `factory/` directory, run `bf init --name <project>` (or `bf init --workspace --name <project>` for a workspace) and ask the human for the idea.
2. If `human_turns_note` is set (or `bf status` prints a `note:` that no human message has been recorded), stop before any other work and tell the human in two lines: open Claude Code in the folder that holds `factory/` and `.claude/settings.json` (for a workspace, the workspace root), then say "continue the build". The hooks are not loaded in this session, so nothing they type can count.
3. Say in two or three lines where things stand (the project, which gates are approved or open, whether a run is going, anything parked or failed) and what you need from the human, if anything. Then continue from the state: an open gate waits for their words, open questions need their answers, `bf next` names the work.
4. **Open questions already asked.** If `pages` has a `questions-<stage>` link (an old `questions` link still counts), read that page again and check that it still matches `bf questions --open` (see `asking.md`).
5. Record files left uncommitted by a dead session are normal: commit them (`git add -A && git commit`, in the project folder or the workspace, never in a code repository of a workspace) and go on. `main.note` is information only (a person committed on the main branch). If `main.ok` is false, work from a unit that never landed, or from a test run, reached the main branch: tell the human; the fix is to move the branch back with git, then `bf head accept` if the position changed. If a command refuses because the policy file changed, tell the human to read `bf policy diff`; do not edit the file back yourself.
6. **Layouts.** In a **project folder** the code and the factory share one repository. In a **workspace** (`workspace: true`) the factory lives in a folder of its own, normally with each code repository inside it (`./app`, ignored locally by the workspace's git) or beside it; you run every `bf` command from the workspace folder, never write into a code repository, and workers work in git worktrees under `.bf/work/`. If the code repositories are not added yet, ask for their names and paths and run `bf repo add <name> <path>`.
7. **Spaces and intents.** Read `spaces-intents.md` when the engine names a space or an intent.

## The loop

1. **Design.** Five gates in order: `idea`, `requirements`, `ui`, `architecture`, `plan`. Read `design-stages.md`. Changes to approved records go through `amendments.md`.
2. **Build.** Starts when the `plan` gate is approved. Read `build-loop.md`: `bf plan check`, `bf run start`, then loop on `bf next --json` and start each worker directive on the model it names.
3. **Checkpoints.** Reviews, fixes, syncs and the close are in `checkpoint.md`.
4. **End.** When `bf next` says the run is done, follow the end steps of `checkpoint.md`.

## Reference files

| File | Holds |
|---|---|
| `design-stages.md` | The five gates, the design review, the check settings |
| `amendments.md` | An amendment from open to approval, and the commit after it |
| `build-loop.md` | The build loop, directives, starting a worker, human directives |
| `checkpoint.md` | Checkpoint actions, fixes, syncs, the end of a run, test mode |
| `spaces-intents.md` | Spaces, intents, rules and notes of code knowledge |
| `asking.md` | Asking the human, pages, how a report is written |
| `roles.md` | The roles, their tokens, the commands the engine adds for them |

## Commands to know of

`bf scan secrets|licences` runs a scan as a check's command. `bf preset list|fill` lists the legal presets and fills one with values. `bf uiux coverage <check>` lists the state of every step of a UI/UX verification. `bf --help` lists the commands, `bf status --json` says where the run stands, `bf gate status` the gates.
