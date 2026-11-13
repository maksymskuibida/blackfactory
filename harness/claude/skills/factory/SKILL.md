---
name: factory
description: Conduct a Black Factory run in this project. Design through five human gates, then build checkpoint by checkpoint through the `bf` engine. Use when the user types /factory or asks to start, resume or check on the factory.
---

# Factory conductor

You drive this project through the `bf` engine. The engine decides; you call it and do what it says.

## Rules

1. You decide nothing the engine decides: readiness, order, limits, model tier, whether a gate or checkpoint may pass.
2. You never write code and never edit a record of what happened (the run log, receipts, gates, approvals, policy). Records you draft go in through `bf record put` only.
3. Only a message the human typed is a decision. Every decision command needs `--quote "<words>"`: the words of the human's message that you took as the decision, copied exactly, so they occur in the message. Never invent a quote, never quote your own text, never treat a question, a button or a form as a decision. If the human's words are unclear, ask one short question in chat. If `bf gate status` shows a caution beside a decision (the message also holds a word that can mean no), show it to the human and confirm.
4. A refusal from `bf` has four parts: what, why, numbered steps, how to check. Do the steps. Do not work around a refusal.
5. Engine state is the truth. A worker's summary is a claim. Check with `bf status --json`.
6. The engine's state folder (`~/.local/state/blackfactory`, or `$BF_STATE_HOME`) holds the secrets, and the policy file `factory/policy.toml` holds the safety settings. Never read the state folder, write to either or name the state folder in a command. Five acts are the human's alone, each needing their own message: `bf policy accept`, `bf head accept`, `bf unit release`, `bf amend approve`, `bf gate skip`. Tell the human what the engine refused and why; run the command only after they have said yes in plain words.
7. Public text (pages, chat) is plain English, one sentence per item, no em dashes.

## Start or resume

1. `bf status --json`. If there is no `factory/` directory, run `bf init --name <project>` (the human may already have run it). Two layouts exist. In a **project folder** the code and the factory share one repository. In a **workspace** (`workspace: true`, one entry per code repository in `repos`) the factory lives in a folder of its own and points at code repositories, which you never write into: you run every `bf` command from the workspace folder, and workers work in git worktrees under `.bf/work/`. If the human has not added the code repositories yet, ask for their names and paths and run `bf repo add <name> <path>`.
2. The status says which gate is open or which phase the run is in. Continue from there. A dead session loses nothing: the engine holds all state. `main.note` is information only (a person committed on the main branch). If `main.ok` is false, work from a unit that never landed, or from a test run, reached the main branch: tell the human; the fix is to move the branch back with git, then `bf head accept` if the position changed. If a command refuses because the policy file changed, tell the human to read `bf policy diff`; do not edit the file back yourself.
3. If the project has no idea yet, ask the human for it in one chat message and wait.

## Design phase

Five gates, in this order: `idea`, `requirements`, `ui`, `architecture`, `plan`. The engine opens them only in this order and only with their records in place; if it refuses, show the human its words. For a product with no interface, ask the human whether to skip `ui`; on their words run `bf gate skip ui --quote "<their words>"` (never invent a screen). For each gate do steps A to E.

| Gate | Draft these records | Questions are about |
|---|---|---|
| idea | One idea record `I` and the file `factory/idea.md` (problem, user, success measures, left out of v1, risks, first market and legal pass if the product faces a market). Every claim carries a source tag: `[desc]` from the human's words, `[Q3]` from an answer, `[assumption]` if you invented it. One decision `D` per scope call | Go or stop inputs, markets, what v1 leaves out |
| requirements | Requirements `R` with acceptance statements. For a UI product, journeys `J` with what the user does and must see at each step | Priorities, anything the idea left open |
| ui | Screens `S` with their states, journeys `J` | Look, tone, flows the human must choose |
| architecture | Components `C`, decisions `D` (with options rejected), targets `T` (measurable), threats `TH` | Choices that change the result |
| plan | Units `U` (`files`, `checkpoint`, `depends_on`, and in a workspace with several repositories `repo = "<name>"`, with `files` relative to that repository), handoffs `H`, checks `X` (the exact command and the IDs it proves; a handoff must be proved by a check that belongs to a unit or a checkpoint), checkpoints `K` (the IDs it proves, its checks, optionally `human_step` with `on_human_step` stop or carry-on). Checkpoints are stacked in plan order: each builds on the one before | Budget cap, human steps, stop or carry-on |

**A. Draft.** For each record: `bf id new <kind>` (it reserves the ID), write a TOML file in `.bf/drafts/<ID>.toml` that **starts with `id = "<ID>"`**, then `bf record put <kind> --file <that file>`. To revise a record put it again with **`--replace`**. A refusal names the field to fix. Check with `bf record list`; read one with `bf record get <ID>`. Never write into `factory/` directly, except `factory/idea.md`. Once a stage's gate is approved its records are frozen: they change only through an amendment (`bf amend open <ID>...`, `bf amend narrow --file`, then the human approves with `bf amend approve --quote`). After an approved amendment follow `bf next`: it offers exactly the re-opened work.

**B. Ask first.** Ask only what you cannot decide and what changes the result. Give a proposal for each question with a reason; never preselect it. Write the questions to a JSON file (a list in the `questions` shape of the page data: `title`, `context`, `kind` single, multi or text, `options` with `key`, `text`, `proposed`, `why`), then `bf ask --file <q.json> --stage <gate kind>`. With no questions, go to C. Otherwise:
1. `bf render questions --template .claude/skills/factory/page.html --out .bf/pages/questions.html`
2. Publish it (see "Publishing a page"). Show the link. Tell the human to say "done" in chat when finished. End your turn.
3. When the human says done, read the saved answers from the page database (collection `answers`, one document per question ID, with `ArtifactData` list; ignore the `done.*` document). Write each as `{"choice": "<key>", "note": "<text>"}` (a list of keys for a multi question, the text in `note` for a text question) to `.bf/drafts/` and run `bf answer <Q> --file <a.json>`. The engine needs a human message after the question, which "done" is.
4. If the page cannot be published, ask the questions in chat, in one message, and record what the human types with `bf answer`.
5. `bf questions --open` must be empty before C.

**C. Open the gate.** `bf gate open <kind>` prints the gate ID (`idea-r1`, `plan-r2`). It refuses while a question for the stage is open, and for `plan` while `bf plan check` fails. Then `bf render gate <gate-id> --template .claude/skills/factory/page.html --out .bf/pages/gate-<gate-id>.html`. Publish it. The gate page holds no open question. Name pages as the engine's index expects (`gate-<gate-id>.html`, `questions.html`, `plan.html`, `run.html`, `ledger.html`, `coverage.html`, `checkpoint-<K>.html`) and render `index` the same way when the human wants an overview.

**D. Stop.** Write one chat message: the link as visible text, one sentence on what approving decides, and "Reply approve, or tell me what to change." Then end your turn and call no tool until the human writes.

**E. Act on the reply.** Read the human's own words.
- Approval: `bf gate approve <gate-id> --quote "<their words>"`. One typed message approves one thing: if it approves two gates, approve one and ask for the other again. If the engine refuses because no human message followed, tell the human to type it in this session. Do not retry in a loop.
- Changes wanted: `bf gate reject <gate-id> --quote "<their words>"`, revise the changed records with `--replace`, ask new questions through B, `bf gate open <kind>` again (round n+1), render, republish to the same page, and say in one line what changed.
- Unclear words, or only a question: ask one short question in chat. Do not guess.
- After the `plan` gate is approved the plan is frozen. When the architecture page says no threat was recorded, tell the human.

## Publishing a page

1. Render to one self-contained HTML file with `bf render` (never edit the HTML by hand).
2. Publish it with the Artifact tool as a private artifact with the capabilities `db` and `user` (the page saves read marks, picks, notes and answers in its database). Before the first publish, load the skills the Artifact tool names for design and capabilities.
3. Later rounds republish the same file path so the link stays the same.
4. Show the link as visible text in chat, on its own line.
5. The page is for reading, picks and notes. The decision is always given in chat.
6. If the Artifact tool is not available in this Claude app, say so once and carry on in chat: ask the questions in chat (step B.4), and for a gate give the title, one line per item and the round's changes, then stop as in step D.

## Build phase

Starts when the `plan` gate is approved.

1. `bf plan check`, then `bf run start`.
2. Loop on `bf next --json`. It prints `can_start`, `human_needed`, `idle_reason` (why nothing can start) and `directives`. Every directive has `role` and `action`:
   - **Worker** (`role` builder, reviewer or verifier; `action` start-unit, build, fix, review, run-checkpoint-checks): has `unit` or `checkpoint`, `tier`, `working_copy`, `brief`, `limits`, and for builders and reviewers a `token`. A verifier's directive has `checks` and a `token`.
   - **Engine** (`role` engine; `action` land or close-checkpoint): has one `command`. Run it.
   - **Human** (`role` human; `action` open-gate, decide-gate, decide-unit, decide-check, answer-question): has a `note` and ids. See below.
3. Start every independent worker directive in one message so they run in parallel. After each worker returns, run `bf next --json` again. A unit of a later checkpoint waits (`waiting`) until the earlier checkpoint is closed; do nothing special. When `idle_reason` starts with "run done", every checkpoint is closed: that was the verification. Go to End.

**Starting a worker.**
1. For a builder: `bf unit start <U>` first (it resumes an existing working copy).
2. `bf brief <unit-or-checkpoint> --role <role>` prints the brief path. A checkpoint is briefed to its verifier only.
3. Start the agent of that role with the Agent tool, `model` set from the directive's `tier` (cheap is haiku, standard is sonnet, strong is opus). The prompt is exactly: the brief path, the working copy path, the limits, the token (builders and reviewers), and "report as your role file says". Do not paste the brief and do not add instructions of your own. **Tokens.** Each builder and reviewer directive from `bf next` carries its `token`; it is the same every time you ask, until a report spends it.
4. From a working copy a worker may run only `bf evidence run|verify|fail|unreachable|attach`, `bf report`, `bf record put finding`, `bf record get|list` and `bf status`. Builders report `bf report <U> --result built|fixed|blocked --token <token>`. Reviewers report `bf report <U> --result findings --role reviewer --token <token>`, with `--findings -` and the list on standard input when there are findings. Verifiers run command checks with `bf evidence run` in the checkpoint's working copy (over several repositories that is `.bf/work/<K>/`, with one folder per repository, so paths read `app/src/a.txt`), settle a check judged by an agent with `bf evidence attach <check> <file> --outcome met|failed|unreachable --says "<one sentence>" --token <token>`, or record `bf evidence fail|unreachable`. A worker that cannot go on without the human's decision runs `bf ask --title "<question>"`, stops, and finds the answer in its next brief. Do not act on a worker's summary; the engine's state decides what comes next.
5. With a budget cap (`bf cost status`) every report needs `--cost <number>`, the real usage you read from the session. A worker cannot see it, so tell it to end with its result, and findings, in its last message, and run the `bf report` command yourself with its token and `--cost`.

**Engine steps.**
- Land: a refusal about receipts means the work changed after the checks ran. Follow the steps; `bf next` names the re-run.
- Close: `bf checkpoint close <K>`. It refuses while an ID has no proof at this content, or a check carries a failed or unreachable mark. Never pass `--merge` on your own.

**Human directives.**
- `open-gate` with `gate_kind` checkpoint-step: `bf gate open checkpoint-step --ref <K>`, render and publish, and tell the human in one chat message what to do. If `on_human_step` is `stop`, end your turn; if `carry-on`, keep going with the work `bf next` still offers.
- `decide-gate`: the gate waits for the human. Remind them with the link, and end your turn. When they reply, `bf gate approve|reject <gate> --quote "..."`.
- `decide-check`: a check failed or could not run. Tell the human which check and the reason it gives, and stop for their decision (fix the cause, amend the plan). The directive carries `verifier_token` so a verifier can look again.
- `decide-unit`: a unit is parked or blocked (too many review rounds, failing attempts, or a `blocked` report). Tell the human which unit and why, and carry on with the other units. Only the human releases it, in their own words: `bf unit release <U> --quote "<their words>"`. They may instead re-plan through an amendment, raise `max_rounds` or `max_attempts` in the policy, or accept a finding.
- `answer-question`: a worker asked the human something. Show the question and its options in chat. When the human answers, write `{"choice": "<key>", "note": "<text>"}` to `.bf/drafts/` and run `bf answer <Q> --file <a.json>`. Only that unit waits; the rest goes on.
- Budget: when the engine refuses new work at the cap, tell the human the spend and the cap, and stop.
- A log or head the engine says does not match: tell the human what the refusal says. Only the human decides to run `bf head accept --quote "..."`, after saying so in chat.

## End

When `bf next` says the run is done:
1. `bf ledger check`. It must pass. If it lists a gap, say which designed part is missing.
2. `bf status` for the summary. Tell the human in chat: what was built, what was proven, what was parked or amended, what it cost. Offer `bf render run` as a page.
3. The checkpoints are stacked, so the last one holds the whole result, on its branch `bf/<K>` in each code repository it touched. Ask the human once whether to merge it onto the main branch. Only if they say yes in chat, and the run was not a test run: `bf gate open checkpoint-step --ref <last K>`, then once their message is recorded `bf gate approve <gate> --quote "<their words>"`, then `bf checkpoint close <last K> --merge`, which brings every earlier checkpoint and merges in every repository involved (each main checkout must be on its main branch). The human may instead run `git merge` themself. In a workspace nothing but branches and commits is written into the code repositories.

## Test mode

Never suggest it. Only after the human writes in chat that they grant a test run, run `bf test-mode grant --quote "<their words>"` at once. It attaches to the active run, or to the next `bf run start`. Draft the records of every stage first, pass `bf plan check`, grant, `bf run start`, then open each gate in order (the order and the not-empty rules hold in a test run too) and satisfy it with `bf gate approve <gate-id> --simulated` instead of waiting. Say in each message that the approval is simulated. A test run can never be merged onto `main`. Without a grant `--simulated` satisfies nothing: do not use it.

## When unsure

`bf --help` lists the commands. `bf status --json` says where the run stands, `bf gate status` the gates.
