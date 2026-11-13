# Claude Code harness

This folder holds everything Black Factory needs inside Claude Code. The engine (`bf`) holds the rules. These files only say how to call it.

| Path | What it is |
|---|---|
| `skills/factory/SKILL.md` | The conductor: the one loop that drives a project through five gates and the build. Used as `/factory`. |
| `agents/builder.md` | Builds one unit from its brief, in its own working copy. |
| `agents/reviewer.md` | Reviews cold: brief, diff and receipts only. |
| `agents/verifier.md` | Checks a checkpoint against the contract. Outcomes: met, failed, unreachable. |
| `settings.fragment.json` | The hooks and permission rules a project needs. |
| `install.sh` | Copies the above into a project's `.claude/` and merges the settings. |

The skill expects the page template at `.claude/skills/factory/page.html`. The installer copies it there from `views/templates/page.html` in this repository.

## Install

```bash
harness/claude/install.sh /path/to/your/project
```

The project must be a git repository. The script changes only files inside that folder, makes a `.bak` copy of anything it would change, merges into `.claude/settings.json` without removing anything, and does nothing the second time you run it. `bf` must be on your `PATH` for the hooks to work.

## The two hooks

**`bf guard` on PreToolUse.** Before every tool call that writes or runs something (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Bash`) and before the three tools that read files (`Read`, `Grep`, `Glob`), Claude Code starts `bf guard --root "$CLAUDE_PROJECT_DIR"`. It reads the tool call, the project's signed guard table `.bf/guard.json` and the key that checks the signature, and answers deny with a reason, or nothing (the harness's own permission prompts stay on). The table is rewritten by the engine whenever state changes, so the costly thinking happens once, not on every call. It is what stops an agent editing a gate, the policy or a receipt, recording an approval, pushing, touching files outside its unit's scope, or opening the state folder. `--root` lets it find the project for a call whose working directory is elsewhere.

**`bf human-turn` on UserPromptSubmit.** When the human sends a message, Claude Code starts `bf human-turn` with the hook payload on stdin and the engine records the message as a human turn. Only a message recorded this way lets a decision command pass (`bf gate approve|reject`, `bf test-mode grant`, `bf amend approve`, `bf head accept`, each with `--quote`). Text from another session, a subagent or the app never goes through this hook as a human turn, so it never counts. The engine also checks the payload: it must name a transcript file under the harness's transcript root (`~/.claude/projects`), the file must parse and its last human message must be this prompt. If a harness writes its transcript only after running the hook, no turn would ever be recorded; the fix is `transcript_match = "soft"` under `[human]` in `factory/policy.toml`, accepted by the human with `bf policy accept`. This is the one assumption not yet tried against a live session.

## Why one guard process per tool call

A guard that starts a second process, calls git or touches the network adds its cost to every tool call, and a run makes thousands. So there is exactly one matcher entry with exactly one command, and it starts no child process and calls no git (about 2 to 3 ms per call). The read tools are in the matcher so the guard can refuse a read of the state folder. Do not add a second guard entry for other tools or a second command to the same entry. Installing twice does not duplicate either hook.

## The permission rules

`settings.fragment.json` also adds two deny rules to `permissions.deny`, so Claude's file tools stay out of the engine's state folder (the key, the log head and the role tokens):

```
Read(~/.local/state/blackfactory/**)
Edit(~/.local/state/blackfactory/**)
```

`install.sh` also adds the same two rules for the folder it finds from `$BF_STATE_HOME` or `$XDG_STATE_HOME` when you set either. Confirmed in the official permissions documentation (`https://code.claude.com/docs/en/permissions`, "Read and Edit"): `~/` means your home folder and `//` an absolute path; an `Edit` rule covers `Write`, `MultiEdit` and `NotebookEdit`; a `Read` deny rule applies, best effort, to `Grep` and `Glob`, and also blocks `Edit` and `Write` on the same path; the deny lists of project and user settings merge.

## What this does not stop

- **A shell.** Deny rules apply to Claude's file tools and to the file commands Claude Code recognises in Bash (`cat`, `head`, `sed`, redirections). The documentation says Bash rules are not a security boundary: a program that reads a file indirectly, or a path built at run time, gets through. The guard reads command text too, and has the same limit.
- **The sandbox, not shipped.** Claude Code's sandbox can enforce a path boundary at the operating-system level for shell commands. The keys exist and are documented (`https://code.claude.com/docs/en/sandboxing`): `sandbox.enabled`, `sandbox.filesystem.denyRead`, `sandbox.filesystem.denyWrite`, `sandbox.filesystem.allowRead`, `sandbox.excludedCommands`. They are not in `settings.fragment.json`, for two reasons. The sandbox is off by default and turning it on changes how every shell command runs. And `bf` itself runs through the shell and needs the state folder, so a `denyRead` on that folder would break the engine unless `bf` were excluded from the sandbox by command text, and the documentation says a call with `cd` or a redirect stays sandboxed. This combination has not been tried here. The sandbox covers shell commands only: Claude's file tools follow permission rules instead.
- **The transcripts.** They are the harness's own files. The guard denies writes under the transcript root from a guarded call, but a program that can write them can make a human turn.
- **The engine's own sandbox.** On macOS the engine runs every check and git call under `sandbox-exec` (no state folder, no network, no writes to `factory/` or other working copies). That is separate from Claude Code's sandbox above, and it covers only what the engine starts, not a worker's own shell calls.
- **One session.** The conductor and its workers run under one user account in one settings scope, so the rules cannot tell them apart. The engine tells them apart by the working directory, one-use role tokens and the guard table.
- **A different state folder.** The deny rules name the default folder and the one the installer saw. If you change `BF_STATE_HOME` later, run the installer again.

## Roles and the engine

The role files match the engine as documented in `docs/engine.md`: `bf next --json` gives directives with `role`, `action`, `unit` or `checkpoint`, `tier`, `working_copy`, `brief`, `limits` and a `token`; `bf brief` writes each worker's context; workers report with `bf report --token`. A token is shown once, a worker may run only `bf evidence run|verify|fail|unreachable|attach`, `bf report`, `bf record put finding`, `bf record get|list` and `bf status` from its working copy, and `bf policy accept`, `bf head accept`, `bf tokens reset` and `bf amend approve` are the human's acts. If the engine changes, change `skills/factory/SKILL.md`, not the engine.
