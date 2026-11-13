# Your first run

The exact steps for trying Black Factory on a project of your own for the first time. For the full picture, see [using.md](using.md). You need Rust, Claude Code, `git` and `python3`.

## Choose a layout

- **Project folder.** The factory's files (`factory/`, `.claude/`) live in your code repository and are committed with your code. Simplest for a private project with one repository.
- **Workspace.** The factory lives in a folder of its own, normally a private repository, and points at one or more code repositories. Nothing but branches and commits is written into the code repositories. Use it for a public project, for several repositories, or whenever you want the code history free of factory files.

## Before you start

1. **Pick a small project.** A few hundred lines is plenty. A throwaway project is better for the first try.
2. **Make sure the code repository has one commit on its main branch and is clean.** Check with `git log --oneline` and `git status`.
3. **Add build output to the code repository's `.gitignore`** (`target/`, `node_modules/`, `__pycache__/`, `dist/`). A receipt is bound to a hash of every file that is not ignored, and a stray build folder voids it.
4. **Expect to be asked for your words.** Every decision (each gate, a test run, releasing a stuck unit, a merge) is your own typed message in the Claude Code session that runs the factory. Nothing else counts. Say it plainly ("Approved."). The conductor quotes the words it took as your decision, and the gate page shows them.

## Install it

1. Build the engine, from this repository:
   ```bash
   cargo build --release
   ```
2. Put `bf` on your `PATH`. This is the one thing the installer cannot do for you:
   ```bash
   mkdir -p ~/.local/bin && ln -sf "$PWD/target/release/bf" ~/.local/bin/bf
   ```
   If `~/.local/bin` is not on your `PATH`, add it to your shell's startup file. Open a new terminal and check:
   ```bash
   bf --help
   ```
3. Set up the factory, from a shell where the tools your project's checks need (`python3`, `cargo`, `node`) are on the `PATH`. Checks run with the `PATH` captured now. The order matters.

   **Project folder:** install the harness, create the factory, commit.
   ```bash
   harness/claude/install.sh /path/to/your/project
   cd /path/to/your/project
   bf init --name myproject
   git add -A && git commit -m "Add Black Factory"
   ```

   **Workspace:** create the workspace, point it at each code repository, install the harness into the workspace, commit.
   ```bash
   mkdir myapp-factory && cd myapp-factory
   bf init --workspace --name myapp
   bf repo add app /path/to/app
   /path/to/blackfactory/harness/claude/install.sh --workspace "$PWD"
   git add -A && git commit -m "Add Black Factory"
   ```

   The installer lists what it created. It adds two hooks and two permission rules to `.claude/settings.json`, keeps a `.bak` copy of anything it changed, touches nothing outside the folder it is given and can be run again safely. `bf status` after the commit may print one `note:` line saying the main branch moved on by a commit the engine did not make. It is information, not an error: an ordinary commit of yours blocks nothing.
4. Restart Claude Code if it was already open in that folder, so it reads the new settings.

## Run it

1. Start Claude Code in your project folder, or in the workspace folder for a workspace (not in a code repository), and type `/factory`.
2. Describe your idea in one message: the problem, who it is for, how you will know it works.
3. The conductor works through five gates. At each one:
   - It may first publish a **questions page**. Answer it, then write `done` in chat.
   - It then publishes a **gate page** and shows the link. Read it. Use "Needs you" and "Unread" to find what matters.
   - Pages are published as private pages through the Claude app's artifact feature, which needs a Claude app that supports it. Without it the conductor asks the questions in chat and describes each gate there instead.
   - Reply in chat in your own words. "Approved" or "looks good, go ahead" approves. "Change the second requirement to ..." asks for a new round. A question approves nothing.
   - Approve each gate in its own message.

   | Gate | It asks you |
   |---|---|
   | Idea and evaluation | Is this the right thing to build, and what does v1 leave out? |
   | Requirements | Is this what you mean? From here every item is tracked to done. |
   | UI design | Do the screens and flows look right? (For a product with no interface it asks whether to skip this gate, and you say so in your own words.) |
   | Architecture | Are the parts, targets and threats right? |
   | Delivery plan | Start building? The plan is frozen once you approve it. |

4. After the plan gate the conductor builds on its own. It stops for a human step the plan names, a stuck unit, a failed check or the budget cap, and tells you why in chat.
5. At the end it runs the ledger check and tells you what was built and proven. Checkpoints are stacked, so the last one (a branch named `bf/K-<last>` in each code repository it touched) holds the whole result. Merge it yourself, `git merge --no-ff bf/K-<last>`, or ask the conductor in chat to merge it.

## A test run, with no gate approvals

Use this on a throwaway project to see the whole loop.

1. Install and `bf init` as above.
2. Type `/factory` and describe a tiny idea.
3. When the conductor is about to ask for the first approval, write in your own words that you grant a test run, for example: "I grant a test run, simulate the approvals."
4. The conductor drafts every stage, starts the run and approves the gates with records marked **simulated**. It says so in each message.
5. A test run can never be merged onto the main branch. Delete the branches and try a real run.

## When something is refused

Every refusal prints four parts: what was refused, why, numbered steps, and how to tell it worked. Do the steps.

| You see | Do this |
|---|---|
| `no human turn was recorded after the gate opened` | Type your decision in the Claude Code session that is running the factory. If you did and it still happens, check that the hooks are installed (`.claude/settings.json`) and restart Claude Code. |
| `factory/policy.toml differs from the policy the human accepted` | Run `bf policy diff`. If you agree with the change, tell the conductor in chat and it accepts it with your words. Otherwise restore the file. |
| `the main branch moved` | Run `bf status`. If you made that commit, tell the conductor and it accepts the position with your words. |
| a gate will not open | Gates open in order and only with their records in place. Read what the engine says, or tell the conductor to skip the UI gate if there is no interface. |
| a check cannot find a program, or needs the network | Checks run with the `PATH` captured at `bf init` and, on macOS, in a sandbox with no network. Edit `[exec] path`, `[exec] network = true` or `[exec] env` in `factory/policy.toml` and accept the change with `bf policy accept`. |
| `these questions for the stage are unanswered` | Answer the questions page and write `done`. |
| `the plan has N problem(s)` | The conductor fixes them and reopens the gate. |
| `a check failed` or `is unreachable` | Read the reason with `bf next`. Fix the cause or ask for an amendment. |
| `a unit is parked` or blocked | Read its findings, and tell the conductor in your own words to release it, or to re-plan it (see [using.md](using.md), section 13). |
| a worker's question appears in chat | Answer it in chat. Only that unit waits. |
| `the log does not match its head` | Run `bf doctor`. If you accept the log as it is, tell the conductor in chat, and it runs `bf head accept` with your words. |
| `bf: command not found` | `bf` is not on your `PATH`. Repeat step 2 of "Install it". |

## What to know before you rely on it

- It is a first version. Read [using.md](using.md), section 14, "What the factory checks, and what it leaves to Claude Code": the engine makes sure nothing is skipped, and Claude Code's own permission checks are the layer against deceptive behaviour.
- On macOS checks run in a sandbox with no network; on other systems they run unsandboxed and the receipt says so.
- Look at the words the engine took as your decision: `bf gate status` prints them, and so does each gate page.
- To stop for the day, just stop. State is kept in `factory/` and the state folder, and `/factory` resumes where it was. If the conductor session ends in the middle of a run, start it again the same way: that restart is your part for now. Do not delete `~/.local/state/blackfactory`: it holds the key that seals the log and receipts.
