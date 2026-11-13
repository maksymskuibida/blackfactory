# Using Black Factory

This page takes you from nothing to a running factory on a project of your own. For a short checklist of the first run, see [first-run.md](first-run.md). You need Rust, Claude Code, `git` and `python3`.

## 1. How it works in one minute

You give an idea and make the design decisions. The factory builds the product on its own and proves, checkpoint by checkpoint, that what you designed is what got built.

- **Design phase.** Five gates, in order: idea and evaluation, requirements, UI design, architecture, delivery plan. At each one you read a page and say in chat whether you approve.
- **Build phase.** Agents build, review and verify unit by unit. The engine (`bf`) decides what happens next and refuses anything unproven. You are called only for the human steps the plan names, or when a unit is stuck.
- **Records.** Everything is stored as small files in `factory/` inside your project, in git. The engine's secrets live outside the project, in a state folder (section 9).

## 2. Build the engine

1. Clone this repository and go into it.
2. Build:
   ```bash
   cargo build --release
   ```
3. The binary is `target/release/bf`. Put it on your `PATH`, for example:
   ```bash
   mkdir -p ~/.local/bin && ln -sf "$PWD/target/release/bf" ~/.local/bin/bf
   ```
4. Check it:
   ```bash
   bf --help
   ```

## 3. Install the harness into your project

Your project must be a git repository with at least one commit.

1. From this repository:
   ```bash
   harness/claude/install.sh /path/to/your/project
   ```
2. It copies the conductor skill, the three worker roles and the page template into the project's `.claude/`, and adds to `.claude/settings.json` two hooks and two permission rules that keep Claude out of the engine's state folder. It keeps a `.bak` copy of anything it changes, touches nothing outside the project, and is safe to run again.
3. Commit the new files in your project if you want them versioned.

The hooks and rules are explained in `harness/claude/README.md`.

## 4. Start

1. In your project:
   ```bash
   cd /path/to/your/project
   bf init --name myproject
   ```
   This creates `factory/` (records, policy, config), `.bf/` (derived files, added to `.gitignore`) and the state folder, and accepts the policy as it is. Run it from a shell where the tools your project's checks need (`python3`, `cargo`, `node`) are on the `PATH`: checks run with the `PATH` captured here, in `[exec] path` of `factory/policy.toml`. A folder inside the project on that `PATH` (a virtual environment's `bin`, `node_modules/.bin`) is refused.
   Make sure `.gitignore` also lists your build output (`target/`, `node_modules/`, `__pycache__/`). A receipt is bound to a hash of every file that is not ignored, so a stray build folder changes the hash and voids the receipt.
2. Start Claude Code in the same folder and type:
   ```
   /factory
   ```
3. Describe your idea in one message. The conductor takes it from there.

## 5. The five gates

At every gate the same things happen:

1. The conductor drafts the stage's records.
2. If it needs decisions only you can make, it first publishes a **questions page**. You answer there and write "done" in chat.
3. It then publishes the **gate page**. You read it and reply in chat in your own words: approve, or say what to change.
4. On "change", it revises, and the page for the next round shows what changed.

Pages are private artifacts. Each item has a read mark, and filters show what needs you, what is unread and what changed. The page never takes the decision. You give it in chat.

| Gate | What you read | What you decide |
|---|---|---|
| 1. Idea and evaluation | The problem, the user, success measures, what v1 leaves out, risks, first market and legal notes | Go, go with changes, or stop |
| 2. Requirements | Requirements with IDs and acceptance statements; user journeys for a product with an interface | That this is what gets built. From here every ID is tracked to done |
| 3. UI design | Screens, their states, the journeys. Skipped if the product has no interface | The look and the flows |
| 4. Architecture | Components, decisions with what was rejected, measurable targets, threats | Structure, targets and security posture |
| 5. Delivery plan | Units as waves, handoffs, checkpoints and the IDs each proves, human steps, estimate, budget | Start building. The plan is frozen from here: later changes need an amendment you approve |

## 6. Answering and approving

- **Questions.** Pick an option or write a note. Proposals are shown but never preselected. Then type `done` in the chat.
- **Approving.** Type it yourself, in the chat of the session that is running the factory, in one plain sentence: "Approved." or "Looks good, go ahead." A message from another session, an agent or the page does not count, and neither does a question ("is this ready?" approves nothing). One message approves one thing, so approve each gate in its own message.
- **Keep it plain.** The engine reads your message with simple word lists, in English, German, French and Spanish. An approval needs a word like "approved", "yes" or "go ahead" and no word like "no", "not", "change", "wait" anywhere in the message. "Approved, no changes needed" is refused because of "no" and "changes". If the engine cannot tell what you meant, the conductor tells you and asks you to say it again plainly. It never rephrases your words for you.
- **The quote.** The conductor passes the whole message, or one whole sentence of it, as the quote. The engine stores the quote and your whole message and shows both on the gate page ("Decided on the words", "The human's whole message") and in `bf gate status`. Read them. If they are not what you meant, tell the conductor in chat.
- **Changing.** Say what to change, in one sentence that says no to the current version: "No, change the second requirement to ...". Nothing is approved until you say so.
- If the engine refuses a decision, the conductor tells you why. The usual cause is that your message did not reach this session, or that it mixed yes and no words.

## 7. What runs unattended

After you approve the delivery plan, the conductor runs a loop: ask `bf next`, start the workers it names (builder, reviewer, verifier), land finished units, close checkpoints. You can leave. It stops only for:

- a human step the plan marks as stopping the run (you test something by hand, you confirm a result),
- a parked unit or a failed or unreachable check that needs a decision,
- the budget cap,
- the end of the run, after the last checkpoint closes. Each checkpoint's verifier is the verification of what it proves, and `bf ledger check` is the final test.

For a human step that does not stop the run, the factory carries on with independent work and you answer when you can. If you set a budget cap in `factory/policy.toml` (`[budget] cap`), the conductor reports the cost of every piece of work and the engine refuses new work at the cap.

## 8. Test runs

A test run completes a whole factory run without you approving each gate. It is for trying the factory on a toy project.

1. Say in chat that you grant a test run, in one plain sentence that speaks of a test run (the engine refuses a grant that does not).
2. The conductor runs `bf test-mode grant` with your words as the quote.
3. The conductor drafts the records of every stage, then starts the run (`bf run start`), then opens and approves each gate in order with a record marked **simulated**.

A simulated approval counts for nothing outside a granted run, and a run that contains one can never be merged onto `main`. Never use it on work you intend to ship.

## 9. Where things are

| Where | What |
|---|---|
| `factory/` (in your project, tracked) | Requirements, design parts, units, checkpoints, receipts, the run log. One file each |
| `factory/policy.toml` | Model tier by kind of work, round limits, budget cap, the `PATH` checks run with, the words the engine reads. Only you edit it, and a change counts only after you accept it with `bf policy accept` (section 12) |
| `.bf/` (in your project, ignored by git) | Drafts, briefs, built pages, the signed guard table and the units' working copies |
| `.claude/skills/factory/`, `.claude/agents/` | The conductor's instructions, the page template, builder, reviewer, verifier |
| `~/.local/state/blackfactory/<project id>/` | The state folder: the key that seals receipts and the log, the log's head, the accepted copy of the policy, the role tokens. Outside the project. If `BF_STATE_HOME` or `XDG_STATE_HOME` is set, it is under that instead |

Do not edit `factory/` or the state folder by hand. The engine and the guard hook refuse it, and an edited record is not a record of what happened.

## 10. Merging the result

Closing a checkpoint (`bf checkpoint close`) leaves the work on a local branch `bf/K-1`. Nothing reaches `main` and nothing is pushed until you decide. You have two ways to merge:

- Ask the conductor in chat to merge. It opens a step gate for the checkpoint, you say merge, and it runs `bf checkpoint close K-1 --merge`.
- Merge it yourself in your own terminal, for example `git merge --no-ff bf/K-1`.

The engine refuses its own merge after a test run, and complains if a merge brings in commits of a test run or of a unit that never landed. Do not merge the branch of a test run.

## 11. Looking at status

```bash
bf status            # where the run stands
bf gate status       # which gates are open or approved, and the words that decided them
bf log               # the append-only event log
bf next              # what would start next
bf ledger check      # every designed part with no unit, check or evidence
bf ledger impact R-3 # everything linked to R-3
bf doctor            # checks the key, the log, the guard table and the plan
```

Add `--json` for data. To get a page, render one: `bf render run --template .claude/skills/factory/page.html --out run.html`, then open the file.

## 12. When something is refused

Every refusal has four parts: what was refused, why, numbered steps, and how to tell it worked. Do the steps. The common ones:

- **No human message followed the gate.** Type your decision in the session that is running the factory. If you did type it and the engine still never records a message, the harness may write its transcript after the hook runs. Then open `factory/policy.toml`, set `transcript_match = "soft"` under `[human]`, and accept the change as described below. This is the one point that has not been tried against a live session.
- **The message mixes yes and no words, or is not a whole sentence.** Say it again plainly, in one sentence.
- **The policy file differs from the one you accepted.** Every command stops. Read the change with `bf policy diff`. If you agree, tell the conductor in chat and it runs `bf policy accept` with your words. If not, put the old file back. Only you decide this.
- **A check failed or is unreachable.** The verifier recorded why (`bf next` shows it). Fix the cause or amend the plan, then the verifier runs the check again. On macOS checks run in a sandbox with no network.
- **The log does not match its head, or the main branch moved without the engine.** `bf doctor` and `bf status` say where. Only you decide to keep things as they are: tell the conductor in chat, and it runs `bf head accept` with your words. A commit that touches only `factory/` and `.gitignore` is not a move. Receipts of a cloned project must be run again.
- **A role token is lost.** Tokens are shown once, to the conductor. Only you can reset them: tell the conductor in chat, and it runs `bf tokens reset` with your words.

## 13. When a unit is parked

A unit is parked when it passes its limit of review rounds or sits idle too long. The conductor tells you which unit and why.

1. Run `bf stuck` to list parked units and the reason.
2. Read the unit's open findings: `bf record list --kind finding`, and `bf record get F-1` for one.
3. Decide, then tell the conductor in chat: re-plan the unit through an amendment (`bf amend open <ID>`), raise the limit in `factory/policy.toml` (only you edit that file), or accept a finding by setting its `outcome = "accepted"`. The engine has no command that releases a parked unit by itself.
4. The rest of the run carries on meanwhile.

## 14. What the factory does not stop yet

The guard and the checks stop mistakes, drift and casual cheating, and make deliberate cheating visible. They do not make a determined agent honest. There are two cases.

**A worker with only the guarded tools** (Write, Edit, Read, Grep, Glob and Bash through the hook) can no longer, in a few calls, record a human turn, approve a gate with your refusal, edit the policy or the check records unnoticed, add a unit to an approved plan, read the key by name or move the main branch without the engine saying so. It can still:

- **Hide a path.** The guard reads command text. A path built at run time, a variable, a glob or a script file that does the work is not seen. The sandbox around checks and git helps only where the engine starts the process.
- **Run its own code as a check.** A check runs the unit's code. On macOS the engine runs it in a sandbox (no state folder, no network, no writes to `factory/` or other working copies). Elsewhere it runs unsandboxed and the receipt says so. Either way, what a check proves rests on someone reading that code.
- **Plant a program where checks look.** A program replaced inside an unchanged folder on the check `PATH` is not seen.
- **Take the reviewer's token** by running `bf next` from a script before the conductor does.
- **Use an older, still valid guard table**, and **race a symlink**.
- **Move the main branch, then be noticed.** The engine refuses to continue until you accept the position. Only a host's branch protection stops the move.

**An agent with an unrestricted shell under your account** can read the state folder: with the key it can seal receipts, chain log lines, sign a guard table, rewrite the log and its head together, and hand itself role tokens. It can write anywhere you can, including the policy and the transcripts. Nothing in this engine stops that. Real separation needs the harness's sandbox, so that worker sessions cannot touch the state folder, `factory/`, `.bf/`, the transcripts and other working copies at all, and a host identity the agent does not hold: a pull request that only you merge, with required checks that re-verify every receipt.

Two limits of reading your words: the engine cannot understand a sentence, so it cannot tell "Yes, but only if..." from "Yes" (the quote and your whole message are shown for you to audit), and the cost of a piece of work is the conductor's word. The full table is in [engine.md](engine.md), section "What the engine does not stop".

## 15. What is not built yet

- Plugins, including plugin patches, and the approved plugin set check.
- The supervisor command that runs workers unattended without a watching session. Today the conductor session is the driver.
- Opening pull requests. The plan can mark a checkpoint for one, and nothing acts on it yet.
- Harnesses other than Claude Code, and hosts other than local git.
- The UI/UX mode, the security and legal roles, and the operations phase (deploy, observe).
- Team work: one person per project. Records outside git.
