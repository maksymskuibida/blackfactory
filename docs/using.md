# Using Black Factory

This page takes you from nothing to a running factory on a project of your own. For a short checklist of the first run, see [first-run.md](first-run.md). You need Rust, Claude Code, `git` and `python3`.

## 1. How it works in one minute

You give an idea and make the design decisions. The factory builds the product on its own and proves, checkpoint by checkpoint, that what you designed is what got built.

- **Design phase.** Five gates, in order: idea and evaluation, requirements, UI design, architecture, delivery plan. At each one you read a page and say in chat whether you approve.
- **Build phase.** Agents build, review and verify unit by unit. The engine (`bf`) decides what happens next and refuses anything unproven. You are called only for the human steps the plan names, or when a unit is stuck.
- **Records.** Everything is stored as small files in `factory/`, in git: inside your project, or in a workspace of its own (section 3). The engine's secrets live outside, in a state folder (section 9).

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

## 3. Choose a layout and install the harness

There are two layouts. Pick one before you start.

| | Project folder | Workspace |
|---|---|---|
| Where the factory's files live | In the code repository: `factory/` (records, receipts, the log) and `.claude/` are committed with your code | In a folder of its own, normally a private repository, that points at one or more code repositories |
| Where Claude Code runs | In the project | In the workspace folder, the root that holds `factory/` and `.claude/` |
| Where the code lives | The project itself | In repositories inside the workspace folder (`./app`, ignored by the workspace's git) or beside it |
| What the code repositories get | The factory's files | Nothing but branches and commits |
| Use it for | A private project with one repository that you do not mind carrying the factory's files | A public project, a project with several repositories, or any code history you want to keep free of factory files |

In a workspace, nothing but branches and commits is written into the code repositories: no `factory/`, no `.claude/`, no `.gitignore` entry. The factory's records, its log and the harness files stay in the workspace. A unit's working copy is a git worktree of its code repository under `.bf/work/` in the workspace.

Install the harness into the folder that holds the factory, never into a code repository of a workspace:

```bash
harness/claude/install.sh /path/to/your/project              # project folder
harness/claude/install.sh --workspace /path/to/workspace     # workspace
```

It copies the conductor skill, the three worker roles and the page template into that folder's `.claude/`, and adds to `.claude/settings.json` two hooks and two permission rules that keep Claude out of the engine's state folder. It keeps a `.bak` copy of anything it changes, touches nothing outside the folder, and is safe to run again. The hooks and rules are explained in `harness/claude/README.md`.

## 4. Start

Run `bf init` from a shell where the tools your project's checks need (`python3`, `cargo`, `node`) are on the `PATH`: checks run with the `PATH` captured then, in `[exec] path` of `factory/policy.toml`. `bf doctor` lists folders of that `PATH` inside the project or your home folder, as information.

**Project folder.** The order matters: commit, install, `bf init`, commit again, start.

1. The project is a git repository with at least one commit. Its `.gitignore` lists your build output (`target/`, `node_modules/`, `__pycache__/`): a receipt is bound to a hash of every file that is not ignored, so a stray build folder voids it.
2. Install the harness (section 3), then:
   ```bash
   cd /path/to/your/project
   bf init --name myproject
   ```
   This creates `factory/`, `.bf/` (derived files, added to `.gitignore`) and the state folder, and accepts the policy as it is.
3. Commit the new files (`.claude/`, `factory/` and `.gitignore`), so that the factory begins on a quiet project:
   ```bash
   git add -A && git commit -m "Add Black Factory"
   ```
   `bf status` then prints one `note:` line saying the main branch moved on by a commit the engine did not make. It is information, not an error.
4. Start Claude Code in the project folder and type `/factory`.

**Workspace.**

1. Every code repository is a git repository with at least one commit on its main branch and its own `.gitignore` for build output. In a workspace nothing is left out of the receipt hash.
2. Make the workspace. `bf init --workspace` makes the folder a git repository too:
   ```bash
   mkdir myapp-factory && cd myapp-factory
   bf init --workspace --name myapp
   ```
3. Put each code repository inside the workspace folder (clone it, or move an existing one) and point the workspace at it. `bf repo add` takes the path, absolute or relative to the workspace, and the branch it is merged into, which defaults to the one checked out; it also writes the folder's name into the workspace's `.git/info/exclude`, so the workspace's git ignores it and nothing of the code repository is ever committed into the workspace:
   ```bash
   git clone <url> app          # or: mv /path/to/app ./app
   bf repo add app ./app
   bf repo list
   ```
   A code repository outside the workspace works too (`bf repo add app ../app`), but then you open Claude Code in the workspace and the code is not in what you see. The recommended layout keeps both under one root.
4. Install the harness into the workspace (section 3), then commit it:
   ```bash
   harness/claude/install.sh --workspace /path/to/myapp-factory
   git add -A && git commit -m "Add Black Factory"
   ```
   You may give the workspace a private remote; the engine never touches the network.
5. Start Claude Code **in the workspace folder** (the root that holds `factory/` and `.claude/settings.json`, not the folder above it) and type `/factory`. If you open it elsewhere the hooks never load and none of your messages are recorded; the conductor notices and tells you.

Then, in either layout, describe your idea in one message. The conductor takes it from there. In a workspace with several repositories the plan says which repository each unit changes.

## 5. The five gates

At every gate the same things happen:

1. The conductor drafts the stage's records.
2. If it needs decisions only you can make, it first publishes a **questions page**. You answer there and write "done" in chat.
3. It then publishes the **gate page**. You read it and reply in chat in your own words: approve, or say what to change.
4. On "change", it revises, and the page for the next round shows what changed.

The questions and gate pages are published as private pages through the Claude app's artifact feature, which needs a Claude app that supports it. Without it the conductor asks in chat instead and you answer there. On a page, each item has a read mark, and filters show what needs you, what is unread and what changed. The page never takes the decision. You give it in chat.

| Gate | What you read | What you decide |
|---|---|---|
| 1. Idea and evaluation | The problem, the user, success measures, what v1 leaves out, risks, first market and legal notes | Go, go with changes, or stop |
| 2. Requirements | Requirements with IDs and acceptance statements; user journeys for a product with an interface | That this is what gets built. From here every ID is tracked to done |
| 3. UI design | Screens, their states, the journeys. For a product with no interface the conductor asks whether to skip it, and you say so in your own words | The look and the flows |
| 4. Architecture | Components, decisions with what was rejected, measurable targets, threats | Structure, targets and security posture |
| 5. Delivery plan | Units as waves, handoffs, checkpoints and the IDs each proves, human steps, estimate, budget | Start building. The plan is frozen from here: later changes need an amendment you approve |

## 6. Answering and approving

- **Questions.** Pick an option or write a note. Proposals are shown but never preselected. Then type `done` in the chat.
- **Approving.** Type it yourself, in the chat of the session that is running the factory, and say it plainly: "Approved." or "Looks good, go ahead." A message from another session, an agent or the page does not count, and neither does a question ("is this ready?" approves nothing). One message decides one thing, so approve each gate in its own message.
- **The quote.** The conductor reads your message and passes the words it took as the decision. The engine stores the quote and your whole message and shows both on the gate page ("Decided on the words", "The human's whole message") and in `bf gate status`. If your message also holds a word that can mean no ("wait", "change"), a caution appears beside it. Read them. If they are not what you meant, tell the conductor in chat.
- **Changing.** Say what to change, in one sentence that says no to the current version: "No, change the second requirement to ...". Nothing is approved until you say so.
- If the engine refuses a decision, the conductor tells you why. The usual cause is that your message did not reach this session, or that it mixed yes and no words.

## 7. What runs unattended

After you approve the delivery plan, the conductor runs a loop: ask `bf next`, start the workers it names (builder, reviewer, verifier), land finished units, close checkpoints. You can leave. It stops only for:

- a human step the plan marks as stopping the run (you test something by hand, you confirm a result),
- a parked or blocked unit, or a failed or unreachable check, that needs a decision (only that unit or checkpoint waits; the rest goes on),
- a question one of the workers put to you: it shows up in chat with its options, only that unit waits, and your answer reaches the worker in its next brief,
- the budget cap,
- the end of the run, after the last checkpoint closes. Each checkpoint's verifier is the verification of what it proves, and `bf ledger check` is the final test.

For a human step that does not stop the run, the factory carries on with independent work and you answer when you can. If the conductor session ends, open Claude Code in the same folder and say "continue the build" (or "continue the <project> build", or `/factory`): a new session knows nothing, reads the state from the engine, tells you in a few lines where things stand and goes on, including a questions page it had already published. Restarting it is your part for now; a supervisor that does it for you is not built yet. If you set a budget cap in `factory/policy.toml` (`[budget] cap`), the conductor reports the cost of every piece of work and the engine refuses new work at the cap.

### Take in work that landed on main during a run

When the main branch gets commits the run lacks, the conductor says so in one line and does nothing until you ask.
Say "take main in" (pull into your local main branch first if the work is on the remote) at a quiet point: the open checkpoint is the latest one and no unit is mid-flight.
The conductor lets running units land, then runs `bf checkpoint sync <K> --quote "<your words>"`. That one sentence is your whole part.
The factory then merges the main branch into the checkpoint, and its own agents resolve any conflict.
It runs the checkpoint's checks on the joined code, repairs what fails, and has a reviewer read the result before the sync counts.
No unit starts until it is settled, and landed units keep their receipts.
You hear about it only if it parks (too many rounds, or the resolver is blocked): then you say whether to release it or abort.
Prefer to merge yourself? Run `git merge <main branch>` in `.bf/work/<K>` (or `.bf/work/<K>/<repo>`), resolve, commit, then tell the conductor, which runs the command with `--resolved`.

## 8. Test runs

A test run completes a whole factory run without you approving each gate. It is for trying the factory on a toy project.

1. Say in chat that you grant a test run, in a sentence that speaks of a test run (the engine refuses a grant that does not).
2. The conductor runs `bf test-mode grant` with your words as the quote.
3. The conductor drafts the records of every stage, then starts the run (`bf run start`), then opens and approves each gate in order with a record marked **simulated**. The order and the not-empty rules hold in a test run too, and skipping the UI gate still needs your own words.

A simulated approval counts for nothing outside a granted run, and a run that contains one can never be merged onto `main`. Never use it on work you intend to ship.

## 9. Where things are

| Where | What |
|---|---|
| `factory/` (in the project folder or the workspace, tracked) | Requirements, design parts, units, checkpoints, receipts, the run log, and in a workspace `workspace.toml` naming the code repositories. One file each |
| `factory/policy.toml` | Model tier by kind of work, round limits, budget cap, the `PATH` checks run with, the words the engine reads. Only you edit it, and a change counts only after you accept it with `bf policy accept` (section 12) |
| `.bf/` (in the project folder or the workspace, ignored by git) | Drafts, briefs, built pages, the signed guard table and the units' working copies (`.bf/work/<unit>`; a checkpoint over several repositories has `.bf/work/<checkpoint>/<repo>`) |
| `.claude/skills/factory/`, `.claude/agents/` | The conductor's instructions, the page template, builder, reviewer, verifier |
| `~/.local/state/blackfactory/<project id>/` | The state folder: the key that seals receipts and the log, the log's head, the accepted copy of the policy, the role tokens. Outside the project. If `BF_STATE_HOME` or `XDG_STATE_HOME` is set, it is under that instead |

Do not edit `factory/` or the state folder by hand. The engine and the guard hook refuse it, and an edited record is not a record of what happened.

## 10. Merging the result

Checkpoints are stacked in plan order: each builds on the one before, so a run flows through all of them and you merge once, at the end. Closing a checkpoint (`bf checkpoint close`) leaves the work on a local branch `bf/K-1`, `bf/K-2` and so on, in each code repository it touched. Nothing reaches the main branch and nothing is pushed until you decide. You have two ways to merge:

- Ask the conductor in chat to merge. It opens a step gate for the last checkpoint, you say merge, and it runs `bf checkpoint close K-<last> --merge`, which brings every earlier checkpoint and merges in every repository involved (each main checkout must be on its main branch).
- Merge it yourself in your own terminal, for example `git merge --no-ff bf/K-<last>` in each code repository.

The engine refuses its own merge after a test run, and complains if a merge brings in commits of a test run or of a unit that never landed. Do not merge the branch of a test run.

## 11. Looking at status

```bash
bf status            # where the run stands
bf gate status       # which gates are open or approved, and the words that decided them
bf log               # the append-only event log
bf next              # what would start next
bf repo list         # in a workspace: the code repositories and where each main branch stands
bf ledger check      # every designed part with no unit, check or evidence
bf ledger impact R-3 # everything linked to R-3
bf doctor            # checks the key, the log, the guard table and the plan
```

Add `--json` for data. To get a page, render one: `bf render run --template .claude/skills/factory/page.html --out run.html`, then open the file.

## 12. When something is refused

Every refusal has four parts: what was refused, why, numbered steps, and how to tell it worked. Do the steps. The common ones:

- **No human message followed the gate, or `bf status` says no message of yours has been recorded.** Claude Code was most likely opened in the wrong folder, so the hooks never loaded. Open it in the folder that holds `factory/` and `.claude/settings.json` (for a workspace, the workspace root), then say "continue the build".
- **The policy file differs from the one you accepted.** Every command stops. Read the change with `bf policy diff`. If you agree, tell the conductor in chat and it runs `bf policy accept` with your words. If not, put the old file back. Only you decide this.
- **A check failed or is unreachable.** The verifier recorded why (`bf next` shows it). Fix the cause or amend the plan, then the verifier runs the check again. On macOS checks run in a sandbox with no network and a short list of environment variables. A check that needs the network or more variables is allowed by setting `[exec] network = true` or `[exec] env = [...]` in `factory/policy.toml`, which you then accept with `bf policy accept`.
- **The log does not match its head, or the main branch moved without the engine.** `bf doctor` and `bf status` say where. Only you decide to keep things as they are: tell the conductor in chat, and it runs `bf head accept` with your words. An ordinary commit of yours on the main branch is only a note, and blocks nothing; the engine objects only when work from a unit that never landed, or from a test run, reached it. Receipts of a cloned project must be run again.
- **A gate will not open.** Gates open in order (idea, requirements, UI, architecture, plan) and only with their records in place. For a product with no interface, tell the conductor in your own words to skip the UI gate.

## 13. When a unit is parked

A unit is parked when it passes its limit of review rounds (`max_rounds`) or of failed attempts in a row (`max_attempts`), or sits idle too long; it is blocked when its builder says it cannot go on. The conductor tells you which unit and why, and the rest of the run carries on.

1. Read the unit's open findings: `bf record list --kind finding`, and `bf record get F-1` for one.
2. Decide, then tell the conductor in chat. To let the unit try again, say so in your own words: the conductor runs `bf unit release U-3` with your words as the quote, and rounds and attempts start again.
3. Or re-plan the unit through an amendment (`bf amend open <ID>`): once you approve it, the engine re-opens the units, checkpoints and checks it touches and nothing else. Or raise `max_rounds` or `max_attempts` in `factory/policy.toml` and accept the change. Or accept a finding by setting its `outcome = "accepted"`.

## 14. What the factory checks, and what it leaves to Claude Code

The engine makes sure every designed part is built and proven and nothing is skipped. It refuses to land a unit, close a checkpoint or finish a run until each part has a receipt it wrote itself, it freezes approved records, and it stores your decisions with your own words. It is the way to make sure nothing is forgotten. It is not a cage.

Claude Code's own permission checks are the layer against deceptive behaviour, and they stay on: the guard hook only refuses what the factory must protect (the log, receipts, gates, policy, other units' files) and otherwise leaves every permission prompt to Claude Code.

How it works, in plain terms:

- **Approval is the conductor's reading of your words.** That is how it works. The engine checks that the quote is in your message and is not a question, and shows the quote and your whole message on the gate page, so you can see what was taken.
- **Checks run the unit's own code.** On macOS the engine runs them in a sandbox (no network, no access to the state folder, no writes to `factory/` or other working copies). Elsewhere they run unsandboxed and the receipt says so. What a check proves rests on the check being a good one.
- **The cost of a piece of work is the number the conductor reports.**
- **The engine's state folder is protected from the guarded tools and by permission rules, not from an unrestricted shell.** An agent that already has full shell access under your account can reach it. If you want separation beyond Claude Code's checks, use Claude Code's sandbox and a host where only you can merge.
- **Moving the main branch** by git is noticed and the engine waits for you; only a host's branch protection prevents it.

The full table is in [engine.md](engine.md), section "What the engine does not stop".

## 15. What is not built yet

- Plugins, including plugin patches, and the approved plugin set check.
- The supervisor command that runs workers unattended without a watching session. Today the conductor session is the driver.
- Opening pull requests. The plan can mark a checkpoint for one, and nothing acts on it yet.
- Harnesses other than Claude Code, and hosts other than local git.
- The UI/UX mode, the security and legal roles, and the operations phase (deploy, observe).
- Team work: one person per project. Records that are not in git.
- One unit that changes two repositories: a unit names one repository.
