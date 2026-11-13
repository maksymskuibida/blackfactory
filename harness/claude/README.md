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
harness/claude/install.sh --workspace /path/to/your/workspace
```

The first form installs into a project, a git repository that holds the code. The second installs into a workspace, a folder made with `bf init --workspace` that holds only the factory and points at code repositories. The folder is detected by `factory/workspace.toml`; `--workspace` only makes the script refuse a folder that is not one, so a code repository is never picked by mistake. In both forms everything goes into that folder's `.claude/`: nothing is written into a code repository. The script touches nothing outside the folder, makes a `.bak` copy of anything it would change, merges into `.claude/settings.json` without removing anything, and does nothing the second time you run it. `bf` must be on your `PATH` for the hooks to work. In a workspace, start Claude Code in the workspace folder so that `CLAUDE_PROJECT_DIR` is the workspace.

## The two hooks

**`bf guard` on PreToolUse.** Before every tool call that writes or runs something (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Bash`) and before the three tools that read files (`Read`, `Grep`, `Glob`), Claude Code starts `bf guard --root "$CLAUDE_PROJECT_DIR"`. It reads the tool call, the project's signed guard table `.bf/guard.json` and the key that checks the signature, and answers deny with a reason, or nothing (the harness's own permission prompts stay on). The table is rewritten by the engine whenever state changes, so the costly thinking happens once, not on every call. It is what stops an agent editing a gate, the policy or a receipt, recording an approval, pushing, touching files outside its unit's scope, or opening the state folder. `--root` lets it find the project for a call whose working directory is elsewhere.

**`bf human-turn` on UserPromptSubmit.** When the human sends a message, Claude Code starts `bf human-turn` with the hook payload on stdin and the engine records the message as a human turn, as given. Only a message recorded this way lets a decision command pass (`bf gate approve|reject`, `bf test-mode grant`, `bf amend approve`, `bf policy accept`, `bf head accept`, `bf unit release`, `bf gate skip`), each with a `--quote` of the words the conductor took as the decision. The engine stores the quote and the whole message and shows both beside the decision. Text from another session, a subagent or the app is not recorded as a human turn, so it never counts. The engine also looks at the transcript file the payload names and notes what it finds; a harness that writes its transcript before running the hook may ask for it to be enforced with `transcript_match = "strict"` in the accepted policy.

## Why one guard process per tool call

A guard that starts a second process, calls git or touches the network adds its cost to every tool call, and a run makes thousands. So there is exactly one matcher entry with exactly one command, and it starts no child process and calls no git (about 2 to 3 ms per call). The read tools are in the matcher so the guard can refuse a read of the state folder. Do not add a second guard entry for other tools or a second command to the same entry. Installing twice does not duplicate either hook.

## The permission rules

`settings.fragment.json` also adds two deny rules to `permissions.deny`, so Claude's file tools stay out of the engine's state folder (the key, the log head and the role tokens):

```
Read(~/.local/state/blackfactory/**)
Edit(~/.local/state/blackfactory/**)
```

`install.sh` also adds the same two rules for the folder it finds from `$BF_STATE_HOME` or `$XDG_STATE_HOME` when you set either. Confirmed in the official permissions documentation (`https://code.claude.com/docs/en/permissions`, "Read and Edit"): `~/` means your home folder and `//` an absolute path; an `Edit` rule covers `Write`, `MultiEdit` and `NotebookEdit`; a `Read` deny rule applies, best effort, to `Grep` and `Glob`, and also blocks `Edit` and `Write` on the same path; the deny lists of project and user settings merge.

## What this does and does not do

The hooks and rules keep the factory's own records safe: the log, receipts, gates, policy, the state folder and other units' files. They are not meant to keep an agent in a box. Claude Code's own permission checks stay on (the guard abstains on what it does not deny) and are the layer against deceptive behaviour.

- **Deny rules and the guard read what a call names.** The documentation says Bash rules are not a security boundary, and the guard reads command text too. A path built at run time gets through. An agent with full shell access under your account can reach the state folder.
- **Claude Code's sandbox is not shipped.** It can enforce a path boundary for shell commands, with the keys documented at `https://code.claude.com/docs/en/sandboxing` (`sandbox.enabled`, `sandbox.filesystem.denyRead`, `sandbox.filesystem.denyWrite`, `sandbox.excludedCommands`). It is off by default and changes how every shell command runs, and `bf` itself runs through the shell and needs the state folder, so a `denyRead` on that folder would break it unless `bf` were excluded. If you want that separation, set it up deliberately.
- **The engine runs checks in its own sandbox on macOS**, separate from the one above: no state folder, no network, no writes to `factory/` or other working copies. It covers what the engine starts, not a worker's own shell calls.
- **The transcripts** are the harness's own files. The guard denies writes under the transcript root from a guarded call.
- **A different state folder.** The deny rules name the default folder and the one the installer saw. If you change `BF_STATE_HOME` later, run the installer again.

## Roles and the engine

The role files match the engine as documented in `docs/engine.md`: `bf next --json` gives directives with `role`, `action`, `unit` or `checkpoint`, `tier`, `working_copy`, `brief`, `limits` and, for builders and reviewers and the verifier, a `token` (shown every time until a report spends it); `bf brief` writes each worker's context; workers report with `bf report --token`. A worker may run only `bf evidence run|verify|fail|unreachable|attach`, `bf report`, `bf record put finding`, `bf record get|list` and `bf status` from its working copy, and `bf policy accept`, `bf head accept`, `bf unit release`, `bf amend approve` and `bf gate skip` are the human's acts. If the engine changes, change `skills/factory/SKILL.md`, not the engine.
