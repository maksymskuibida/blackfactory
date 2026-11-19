# Design stages

Five gates, in this order: `idea`, `requirements`, `ui`, `architecture`, `plan`. The engine opens them only in this order and only with their records in place; if it refuses, show the human its words. For a product with no interface, ask the human whether to skip `ui`; on their words run `bf gate skip ui --quote "<their words>"` (never invent a screen). For each gate do steps A to E.

| Gate | Draft these records | Questions are about |
|---|---|---|
| idea | One idea record `I` and the file `factory/idea.md` (problem, user, success measures, left out of v1, risks, first market and legal pass if the product faces a market). Every claim carries a source tag: `[desc]` from the human's words, `[Q3]` from an answer, `[assumption]` if you invented it. One decision `D` per scope call | Go or stop inputs, markets, what v1 leaves out |
| requirements | Requirements `R` with acceptance statements. For a UI product, journeys `J` with what the user does and must see at each step | Priorities, anything the idea left open |
| ui | Screens `S` with their states, journeys `J`. While `design-uiux-verification` is on, also a check record `X` with `named = "design-uiux-verification"` and `how = "agent"`; the UI/UX verifier judges the screens and settles it with `bf evidence attach <X> <file> --outcome met --says "<what was looked at>" --token <token>` (token from `bf roles token design uiux-verifier`). The ui gate is refused without that receipt | Look, tone, flows the human must choose |
| architecture | Components `C`, decisions `D` (with options rejected), targets `T` (measurable), threats `TH` (each with `protects`, `attack` and `mitigation`, drafted by the security expert), and a markets record `M` with `markets` (an empty list for a product with no market) while a legal check is on. The architecture gate is refused without them | Choices that change the result |
| plan | Units `U` (`files`, `checkpoint`, `depends_on`, and in a workspace with several repositories `repo = "<name>"`, with `files` relative to that repository), handoffs `H`, checks `X` (the exact command and the IDs it proves; a required built-stage named check needs its own record with `named = "<name>"`, listed in `checks` by the last checkpoint; a handoff must be proved by a check that belongs to a unit or a checkpoint), checkpoints `K` (the IDs it proves, its checks, `stop` true or false and `stop_why`, one sentence on why it stops or has no stop; optionally `human_step`). A checkpoint has two forms only: it stops and waits for the human, or it has no stop. You propose which; the plan page shows it for every checkpoint and approving the plan gate approves it. The human changes a stop by rejecting the gate with their words; change the records and the next round shows it. Checkpoints are stacked in plan order: each builds on the one before | Budget cap, human steps. Never ask where to stop: the proposal on the page is the question |

**A. Draft.** A designer agent drafts the records of a stage, started by you at once, without stopping or asking the human (see `roles.md`). For each record it: `bf id new <kind>` (it reserves the ID), write a TOML file in `.bf/drafts/<ID>.toml` that **starts with `id = "<ID>"`**, then `bf record put <kind> --file <that file>`. To revise a record put it again with **`--replace`**. A refusal names the field to fix. Check with `bf record list`; read one with `bf record get <ID>`. Never write into `factory/` directly, except `factory/idea.md`. The records are files in git: commit them when a gate opens and when a session ends. Once a stage's gate is approved its records are frozen: they change only through an amendment (`amendments.md`).

When you present a gate, list the designer's calls as the next section says.

**B. Ask first.** Ask only what you cannot decide and what changes the result. The way to ask is in `asking.md`. `bf questions --open` must be empty before C.

**C. Open the gate.** `bf gate open <kind>` prints the gate ID (`idea-r1`, `plan-r2`). It refuses while a question for the stage is open, and for `plan` while `bf plan check` fails. Then `bf render gate <gate-id> --template .claude/skills/factory/page.html --out .bf/pages/gate-<gate-id>.html`. Publish it (see `asking.md`). The gate page holds no open question. Name pages as the engine's index expects (`gate-<gate-id>.html`, `questions-<stage>.html` (one per stage; a page named only `questions` from an older run still counts), `plan.html`, `run.html`, `ledger.html`, `coverage.html`, `checkpoint-<K>.html`) and render `index` the same way when the human wants an overview.

**D. Stop.** Write one chat message: the link as visible text, one sentence on what approving decides, the designer's calls made without asking, and "Reply approve, or tell me what to change." Then end your turn and call no tool until the human writes.

**E. Act on the reply.** Read the human's own words.
- Approval: `bf gate approve <gate-id> --quote "<their words>"`. One typed message approves one thing: if it approves two gates, approve one and ask for the other again. If the engine refuses because no human message followed, tell the human to type it in this session. Do not retry in a loop.
- Changes wanted: `bf gate reject <gate-id> --quote "<their words>"`, revise the changed records with `--replace`, ask new questions, `bf gate open <kind>` again (round n+1), render, republish to the same page, and say in one line what changed.
- Unclear words, or only a question: ask one short question in chat. Do not guess.
- After the `plan` gate is approved the plan is frozen. When the architecture page says no threat was recorded, tell the human.

## The designer's calls

When you present a gate or an amendment, list the calls the designer made without asking, each with the other option.

## Design review

The idea, requirements, architecture and plan gates are held by a design review: `bf gate open` is refused until a design reviewer has judged the records as they are now. Each gate has its check record, named for the review: `idea-review`, `requirements-review`, `architecture-review`, `plan-review`; an amendment has `amendment-review`. `bf checks state` shows which are on.
1. Start the design reviewer: `bf review token <gate>` issues its token and `bf review brief <gate>` writes its brief. Start the `design-reviewer` agent with the brief path, the limits and the token, on the model `bf policy` names for it; a design review has no directive.
2. On a finding, have a designer revise the records, and start the reviewer again with a fresh token. Do this for up to `max_rounds` rounds.
3. When the review parks, put what remains to the human, finding by finding. Only their words accept a finding (`bf review finding <gate> --accept <F-ids> --quote "<their words>"`) or release a fresh set of rounds (`bf review release <gate> --quote "<their words>"`). Never accept or release yourself.

## Check settings

Which named checks are on or off (`bf checks list`) is the human's choice, never yours. To change one: `bf checks propose <check>=off:<reason> ... [--scope default|run|both]` (one reason for each check switched off; it changes nothing), then show the human the output with every reason, in chat, and ask. Only after their reply, `bf checks set --quote "<their words>"`; the engine refuses it without a quote from a message after the proposal. `bf checks state` shows what is on and off, with the reasons. Decide the project default (`--scope default`) before `bf plan check`: the plan check refuses while no setting is decided.

## Legal presets and scans

`bf preset list` shows the legal presets (terms, privacy policy, impressum, in each language) and `bf preset fill <kind> <language> <values-file>` writes a draft under `.bf/drafts/`; the lawyer fills them. `bf scan secrets|licences` is the command of a scan check, run by `bf evidence run`. `bf uiux coverage <check>` shows which steps of a UI/UX verification are met.
