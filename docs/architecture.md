# Black Factory architecture

This is the detailed form of the design, for agents and for anyone who wants to know how Black Factory works. It describes the design the first version is built to. Section 9 says what is not designed or built yet.

## 1. Principles

| # | Principle |
|---|---|
| P1 | **Guard where the action happens, and keep the guard cheap.** The costly thinking is done once, when state changes or a worker starts. The check at each tool call only reads the result. Nothing that runs per tool call starts a second process, calls git as a subprocess, or touches the network. |
| P2 | **Only the engine changes state.** Agents ask and report. They never edit a record of what happened. |
| P3 | **What can be data is data.** Stages, gates, roles, checks, policy and page layouts are files, so a plugin can change them without touching code. |
| P4 | **Every record has two forms from one source.** Structured files for agents, a page for the human, both produced from the same data. |
| P5 | **A refusal teaches.** Every refusal says what was refused and why, the steps to fix it, and how to tell it worked. |
| P6 | **An approval is the human's own words, recorded.** Only a message the human typed counts. Text from another session, a subagent or the app never does. A simulated approval is a different kind of record. |
| P7 | **Speed is tested, not promised.** Each class of command has a time budget checked in CI. |

## 2. Components

### 2.1 Engine core (`bf`)

One Rust binary. One short-lived process per command. No daemon, no network.

| Part | Responsibility |
|---|---|
| Record store | Reads and writes records as files. Validates each against its schema. Keeps an optional derived index that is never the truth. |
| Ledger | Gives every designed part an ID. Holds links from IDs to units, handoffs, checks and evidence. Answers "what is unlinked" and "what is linked to these IDs". |
| Plan and scheduler | Units, handoffs, checkpoints. Checks the graph. Says which units are ready and what each needs. |
| Run | Append-only event log. The state of each unit. Resume after a dead session. Counts attempts and idle time and parks a unit at the limit. |
| Evidence | Runs a check's command itself and writes the receipt, on macOS inside a sandbox with no state folder and no network. Stores attached files by hash. A receipt is bound to the exact content it was run on. |
| Gates and questions | Human gates and their rounds. Approvals with their proof. Questions and answers as data. Test mode. |
| Cost policy | Layered settings. Stamps each piece of work with its model tier and limits. Keeps the budget count and refuses new work past the cap. |
| View data | Builds the data behind every page from the records. |
| Extension points | Loads plugin layers, calls plugin programs at named points, checks the approved plugin set. |

### 2.2 Harness adapter (Claude Code first)

Everything that depends on the AI tool in use sits here, behind one interface.

| Part | Responsibility |
|---|---|
| Conductor | A loop: ask the engine what is next, start the workers it names, report back. It decides nothing. |
| Workers | One agent per piece of work, started with the brief the engine prepared. |
| Page publisher | Turns view data into pages the human opens. |
| Question surface | Shows a question form and returns the answers to the engine. |
| Guard | Three layers, cheapest first. (1) Permissions and a sandbox the engine computes once for each worker when it starts: what it may read, write and run. (2) One fast hook per tool call for what static rules cannot express. It only reads a small guard table the engine rewrites whenever state changes. (3) The engine's own commands. Guarding shell commands by reading their text is a heuristic, so read and write rules rest on the sandbox wherever the harness has one. |

The interface the engine expects from any harness: start a worker with a brief, a model and a working copy; report when it ends and what it used; show a page; ask the human; say how a human act is proven.

### 2.3 Host adapter (GitHub first)

Everything that depends on where code is hosted sits here, behind one interface: open a change, read its checks, merge it, require checks, read who approved. It is used only at the points the delivery plan names. A local-git adapter with no host is the second implementation.

### 2.4 Roles

A role is data: a short prompt, the tools it may use, what it may read, and a model tier from the cost policy.

| Role | Works from | May not |
|---|---|---|
| Builder | The unit's brief | Edit records, gates or policy |
| Reviewer (cold) | Brief, diff, receipts. Nothing from the builder's session. | Write code |
| Verifier (UI/UX, code, performance) | The contract and the built result | Read source, for the checks declared blind |
| Security expert and auditor | Threat model, diff, scan output | Write code (auditor) |
| Lawyer and legal auditor | Design records, market list, sources fetched on demand | Write anywhere except findings |
| Planner | Approved design records | Change design records |
| Amendment evaluator | The engine's impact set | Widen the set |

"May not" is enforced by the worker's permissions, sandbox and the guard hook, not by the prompt.

### 2.5 Plugins

Three layers, weakest first:

1. **Data layers.** A plugin adds or overrides files: stage and gate definitions, role definitions, check kinds, policy defaults, page layouts, legal presets. Order: core, then plugins, then the project.
2. **Programs at named points.** The engine starts a plugin's executable at a named point (a new check kind, a new view, before or after a command) and exchanges data with it. Any language.
3. **Patches.** A plugin may carry a source patch to the engine. The engine is rebuilt with it into a binary pinned for that project. This is how a plugin fixes a bug or changes engine behaviour.

The engine holds a record of the approved plugin set: which plugins, at which content, and whether any of them touches a gate. It refuses to run when the set on disk differs from the approved one. Approving a set that loosens a gate needs the human.

## 3. Records

All records are structured files inside the project's repository, one record per file, under one directory (`factory/`). Logs are append-only lines. Nothing is read out of prose.

| Record | Written by | Changes after approval |
|---|---|---|
| Requirement, with its ID and acceptance checks | Agent drafts, human approves at the Requirements gate | Only by amendment |
| Design part: component, screen, journey, target, threat, decision | Agent drafts, human approves at its gate | Only by amendment |
| Unit: what it delivers (IDs), what it depends on, files it may touch, estimate | Planner, approved at the Delivery plan gate | By amendment or re-plan |
| Handoff: from, to, the thing handed over, its contract | Planner | By amendment |
| Checkpoint: after which units, which IDs it proves, its checks, human step and its stop or carry-on | Planner, approved at the Delivery plan gate | By amendment |
| Check: a command, an agent task or a human step, and the IDs it proves | Planner; plugins add kinds | By amendment |
| Receipt: check, content it ran on, exit code, output hash, attached files | Engine only | Never |
| Finding: stable id, severity, where, required action, outcome | Reviewer through the engine | Outcome only |
| Gate and approval: kind, round, proof, real or simulated | Engine, on a human act | Never |
| Question and answer | Engine | Never |
| Amendment: changed IDs, impact set, what re-opens | Engine and evaluator, approved by the human | Never |
| Run event | Engine only | Never |
| Policy | Human | By the human |
| Approved plugin set | Engine, on a human act | By a new approval |

ID families: `R` requirement, `C` component, `S` screen, `J` journey, `T` target, `TH` threat, `D` decision, `U` unit, `H` handoff, `K` checkpoint, `X` check, `F` finding, `Q` question.

## 4. How a unit moves

1. The scheduler marks a unit ready: everything it depends on has landed, and every handoff it receives has a receipt.
2. The conductor asks what is next and gets a directive: role, unit, model tier, working copy, and a brief. The brief is the exact context: the unit record, the parts of the design it delivers, the contracts it receives. No other reading is required.
3. The builder works in its own working copy and branch.
4. Checks run through the engine, which writes receipts bound to the content they ran on.
5. The cold reviewer gets brief, diff and receipts. It verifies receipts instead of re-running them. Findings get ids that never change.
6. Fix rounds, up to the policy's limit. Past it, the unit is parked and the human is told.
7. Land locally. An engine command merges the unit's branch into the local branch of its checkpoint and refuses unless every receipt matches the content. No network. With no host, units land on local main.
8. The ledger's links are updated from the unit record.
9. When a checkpoint's units have landed, its checks run: verifiers, experts, and any human step. `close` refuses unless every ID the checkpoint names has a receipt at that content or an amendment approved by the human. Where the plan says so, the checkpoint's branch becomes a pull request, its required checks re-verify every receipt, and main moves only when it merges.
10. A human step either stops the factory or lets independent work carry on, as the plan said.

At the end the whole result is verified again, and the ledger refuses to close while any ID lacks a link.

## 5. Human gates

There are five, and a product with no interface has four. Each gate is a page in the same shape: a verdict, the items most likely to be wrong, the rules checked, the stage's own picture, a digest with one sentence per item, and the full records folded shut. On a later round the page shows what changed, in place.

| Gate | What the human decides |
|---|---|
| Idea and evaluation | Go, go with changes, or stop |
| Requirements | That this is what gets built. From here every ID is tracked to done. |
| UI design | The look and the flows |
| Architecture | Structure, targets and security posture |
| Delivery plan | Start construction. After this the factory runs alone until a planned human step. |

A stage asks its questions first, on a form of its own, only for decisions the agent cannot make. The approval page is built after the answers and holds no open question. The decision is said in chat, in the human's own words.

## 6. Decisions

| # | Decision | Chosen | Rejected, and why |
|---|---|---|---|
| D-1 | Engine process model | One short process per command | A background service: faster for hooks, but a second thing that can die, and P1 removes most hooks |
| D-2 | Where state lives | Structured files in git, one record per file, evidence files included; append-only run log. Location set per project: the project's own repository, or a separate branch or repository. Untracked records: later | A database file: fast, but cannot be reviewed or merged in a pull request. Markdown: must be parsed by pattern. A store outside git for large evidence files |
| D-3 | Where rules are enforced | In three layers: per-worker permissions and sandbox computed once at start; one fast guard hook reading a table the engine writes on state change; the engine's own commands | No guard at the tool call: a verifier that has read the source, a deleted file or a pushed branch cannot be undone by a later check. A guard that loads the whole engine on each call: seconds per tool call, against about 2 ms for a guard that reads one small file |
| D-4 | Protecting gates, policy and plugins from the agent | Denied in every worker's permissions and by the guard hook; the engine checks the approved set; the checkpoint's pull request needs the human's approval on any change to them | Relying on the host alone: with pull requests only at checkpoints that is too late |
| D-5 | How the human approves | In the human's own words in chat. The harness adapter reports the human's typed message. The conductor reads it and passes the words it took as the decision; the engine stores that quote with the whole message, checks that the quote is in the message and is not a question, and shows both beside the decision for the human to audit. A message from another session, a subagent or the app is never recorded as a human turn. The guard denies agents the command that records a turn | A separate factory identity on the host; a prompt on the human's machine; the engine judging the sentence |
| D-6 | Who drives a run | First a conductor session the human can watch, starting workers as sessions. Later also a supervisor command that starts unattended workers and restarts dead ones | Supervisor only; conductor only |
| D-7 | Plugins in a compiled engine | Data layers, programs at named points, patches with a rebuild | An embedded scripting or WebAssembly runtime: more moving parts, and a patch covers the same need. Revisit if rebuilds prove slow |
| D-8 | Context for a worker | A brief built by the engine for that unit and role | Shared project documents every role reads: they grow past what an agent can read |
| D-9 | One run of tests | A receipt is reusable by anyone for the same content | Each role re-running the suite |
| D-10 | Isolation of parallel units | One working copy and branch per unit; units in one wave may not share files | A shared checkout |
| D-11 | Test mode | Enabled by a human act, recorded as a grant for one run, given in the human's own words like any approval. Approvals in it are stored as simulated. Inside a granted test run they satisfy every gate so the run completes end to end. Such a run cannot land on the main branch | Trusting a flag in a prompt |
| D-12 | Cost | Layered policy (project, developer, checkout); each directive carries tier and limits; the engine refuses past the cap | Leaving model choice to each agent |
| D-13 | Amendments | Engine computes everything linked to the changed IDs; an agent narrows it with reasons; the human approves one impact page | Re-opening every later stage |
| D-14 | Team work | Not built; nothing in the record model forbids several people | Claims and merges of shared state |
| D-15 | Remote git | Local first. No engine command touches the network; the remote is synced only at points the plan names | Checking the remote on every command: built for teams, wasted on one developer |
| D-16 | Where pull requests open | Set in the delivery plan; per checkpoint by default. Units land locally on the checkpoint's branch | A pull request per unit: a network round trip and a CI run for every unit |
| D-17 | How the engine uses git | The engine calls the `git` program, a few times per command that needs git (the content hash, creating a working copy, landing) and never from the guard, the human-turn hook or record queries. Every call runs with a cleaned environment: no `GIT_*` variables from the caller, a fixed `git` program and no user or system git configuration. An in-process git library was considered and left for later | A git subprocess per question; a library now, which adds many dependencies for the few operations needed |
| D-18 | Skills from other sources | Skills from the harness vendor and other vendors are depended on where installed, behind the role and check-kind interfaces, so another source can stand in later | Copying them |
| D-19 | Questions and approval | Two pages. A stage asks its questions on a form first; the approval page is built after the answers and holds no open question. The decision is said in chat (D-5) | Questions, answers and the gate answer on one page |
| D-20 | Page design | Pages follow one visual language defined in `views/`: one line per row, a read mark per row tied to a hash of its text, the detail folded, changes between rounds shown in place | Ad hoc page formats |

## 7. Targets

Each becomes a checkpoint criterion with its own check.

| ID | Target | Check |
|---|---|---|
| T-1 | Queries and state changes on a project of 1,000 records: median 50 ms and 150 ms, 99th percentile 200 ms and 500 ms | Timing test in CI |
| T-2 | A run killed at any point resumes without redoing a landed unit | Kill test at each step |
| T-3 | The ledger cannot close with an unlinked ID | Attack test |
| T-4 | A checkpoint cannot close without a receipt or an approved amendment for each ID | Attack test |
| T-5 | A receipt for other content is rejected | Attack test |
| T-6 | Every refusal has the four parts of P5 | Test over all refusals |
| T-7 | Harness instructions total under 500 lines | Count in CI |
| T-8 | Core commands make no network call | Test with the network denied |
| T-9 | Outside a granted test run a simulated approval satisfies nothing; inside one the whole run completes with simulated approvals and cannot land on the main branch | Attack test, and a full run in test mode |
| T-10 | The engine refuses to run with an unapproved plugin set | Attack test |
| T-11 | Every gate page is built from records in under one second | Timing test |
| T-12 | The guard hook: median 30 ms at normal load, 99th percentile 100 ms, one process per tool call, one small file read | Timing test in CI, and on the developer's machine |
| T-13 | Only a message the human typed counts as an approval; text from another session, a subagent or the app never does | Attack test with each of the three |

The 30 ms target for the guard rests on the hook reading one small file, not on the language. Measured on the engine: the guard takes about 2 ms per call as a separate process, and queries on a project of 1,000 records take about 20 to 40 ms.

## 8. Threat sketch

What is protected: the gates, the approvals, the evidence, the project's secrets, the budget.

| ID | Threat | Mitigation |
|---|---|---|
| TH-1 | An agent claims a check passed | The engine runs checks and writes receipts; landing re-verifies them locally and the checkpoint's pull request re-verifies them all (T-5) |
| TH-2 | An agent approves its own gate | Only a message the human typed is recorded as an approval (D-5, T-13); the guard denies agents the command that records one |
| TH-3 | An agent edits a gate, the policy or a plugin | D-4 |
| TH-4 | Text fetched from the internet or found in the repository instructs an agent | Research roles have no write tools; what they fetch is stored as data with its source |
| TH-5 | Cost runs away | Hard cap and stop switch in the engine (D-12) |
| TH-6 | A plugin runs harmful code | The plugin set is approved by the human, shown as a diff, before the engine will run it (T-10) |
| TH-7 | A secret reaches a public repository | Secret scan as a required check |
| TH-8 | A blind verifier reads the source | The worker's tool permissions, not its prompt |

## 9. Not designed or not built yet

The UI/UX mode, the security and legal roles, and the operations phase have a place in this structure (roles, check kinds, plugin layers) and no design of their own yet. Each needs its own pass.

The first version builds the engine core, the conductor instructions for Claude Code, and the pages. Plugins, the supervisor command, other harnesses, other hosts and team work are designed for but not built. `docs/using.md` lists what works today.
