# Your first run

The exact steps for trying Black Factory on a project of your own for the first time. For the full picture, see [using.md](using.md). You need Rust, Claude Code, `git` and `python3`.

## Before you start

1. **Pick a small project.** A few hundred lines is plenty. A throwaway project is better for the first try.
2. **Make sure it is a git repository with one commit.** Check with `git log --oneline`.
3. **Add build output to `.gitignore`** (`target/`, `node_modules/`, `__pycache__/`, `dist/`). A receipt is bound to a hash of every file that is not ignored, and a stray build folder voids it.
4. **Expect to be asked for your words.** Every decision (each gate, a test run, a merge) is your own typed message in the Claude Code session that runs the factory. Nothing else counts. Keep each one a plain sentence: "Approved." An approval with words like "no", "not" or "change" in it ("approved, no changes needed") is refused, and the conductor will ask you to say it again.

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
3. Install the harness into your project:
   ```bash
   harness/claude/install.sh /path/to/your/project
   ```
   It lists what it created. It adds two hooks and two permission rules to `.claude/settings.json`, keeps a `.bak` copy of anything it changed, and can be run again safely.
4. Create the factory in your project, from a shell where the tools your project's checks need (`python3`, `cargo`, `node`) are on the `PATH`. Checks run with the `PATH` captured now:
   ```bash
   cd /path/to/your/project
   bf init --name myproject
   ```
5. Restart Claude Code if it was already open in that folder, so it reads the new settings.

## Run it

1. In your project folder, start Claude Code and type `/factory`.
2. Describe your idea in one message: the problem, who it is for, how you will know it works.
3. The conductor works through five gates. At each one:
   - It may first publish a **questions page**. Answer it, then write `done` in chat.
   - It then publishes a **gate page** and shows the link. Read it. Use "Needs you" and "Unread" to find what matters.
   - Reply in chat in your own words. "Approved" or "looks good, go ahead" approves. "Change the second requirement to ..." asks for a new round. A question approves nothing.
   - Approve each gate in its own message.

   | Gate | It asks you |
   |---|---|
   | Idea and evaluation | Is this the right thing to build, and what does v1 leave out? |
   | Requirements | Is this what you mean? From here every item is tracked to done. |
   | UI design | Do the screens and flows look right? (Skipped if there is no interface.) |
   | Architecture | Are the parts, targets and threats right? |
   | Delivery plan | Start building? The plan is frozen once you approve it. |

4. After the plan gate the conductor builds on its own. It stops for a human step the plan names, a stuck unit, a failed check or the budget cap, and tells you why in chat.
5. At the end it runs the ledger check and tells you what was built and proven. The result is on a branch named `bf/K-1` (one per checkpoint). Merge it yourself, `git merge --no-ff bf/K-1`, or ask the conductor in chat to merge it.

## A test run, with no gate approvals

Use this on a throwaway project to see the whole loop.

1. Install and `bf init` as above.
2. Type `/factory` and describe a tiny idea.
3. When the conductor is about to ask for the first approval, write in your own words that you grant a test run, for example: "I grant a test run, simulate the approvals."
4. The conductor drafts every stage, starts the run and approves the gates with records marked **simulated**. It says so in each message.
5. A test run can never be merged onto `main`. Delete the branches and try a real run.

## When something is refused

Every refusal prints four parts: what was refused, why, numbered steps, and how to tell it worked. Do the steps.

| You see | Do this |
|---|---|
| `no human turn was recorded after the gate opened` | Type your decision in the Claude Code session that is running the factory. If you did and it still happens, see [using.md](using.md), section 12, about `transcript_match`. |
| `the message mixes approval and refusal words`, `not a whole sentence` | Say the decision again in one plain sentence. |
| `factory/policy.toml differs from the policy the human accepted` | Run `bf policy diff`. If you agree with the change, tell the conductor in chat and it accepts it with your words. Otherwise restore the file. |
| `the main branch moved` | Run `bf status`. If you made that commit, tell the conductor and it accepts the position with your words. |
| `a role token was already handed out` | Tell the conductor to reset the tokens with your words (`bf tokens reset`). |
| a folder on the check `PATH` is refused | A folder inside your project (a virtual environment, `node_modules/.bin`) is on the `PATH` captured at `bf init`. Run `bf init` from a shell without it, edit `[exec] path` in `factory/policy.toml` and accept the change. |
| `these questions for the stage are unanswered` | Answer the questions page and write `done`. |
| `the plan has N problem(s)` | The conductor fixes them and reopens the gate. |
| `a check failed` or `is unreachable` | Read the reason with `bf next`. Fix the cause or ask for an amendment. |
| `a unit is parked` | Run `bf stuck`, read the findings, and tell the conductor what to do (see [using.md](using.md), section 13). |
| `the log does not match its head` | Run `bf doctor`. If you accept the log as it is, tell the conductor in chat, and it runs `bf head accept` with your words. |
| `bf: command not found` | `bf` is not on your `PATH`. Repeat step 2 of "Install it". |

## What to know before you rely on it

- It is a first version. Read [using.md](using.md), section 14, "What the factory does not stop yet": an agent with an unrestricted shell under your account can still reach the engine's secrets.
- On macOS checks run in a sandbox with no network; on other systems they run unsandboxed and the receipt says so.
- Look at the words the engine took as your decision: `bf gate status` prints them, and so does each gate page.
- To stop for the day, just stop. State is kept in `factory/` and the state folder, and `/factory` resumes where it was. Do not delete `~/.local/state/blackfactory`: it holds the key that seals the log and receipts.
