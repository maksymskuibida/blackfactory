# The build loop

Starts when the `plan` gate is approved.

1. `bf plan check`, then `bf run start`. When `bf next` asks the two plan questions (`answer-wave-checks`, `answer-unit-checks`), put each to the human with `bf checks wave|unit question`, then `propose` and `set --quote "<their words>"`. No unit starts until both are answered, and an answer is changed later only by amendment.
2. Loop on `bf next --json`. It prints `can_start`, `human_needed`, `idle_reason` (why nothing can start) and `directives`. Every directive has `role` and `action`:
   - **Worker** (`role` builder, reviewer, verifier or security-auditor; `action` start-unit, build, fix, review, security-audit, resolve-sync, review-sync, run-checkpoint-checks): has `unit` or `checkpoint`, the model to start it on (`model`, or a `tier`), `working_copy`, `brief`, `limits`, and for builders, reviewers and security auditors a `token`. A verifier's directive has `checks` and a `token`.
   - **Engine** (`role` engine; `action` land or close-checkpoint): has one `command`. Run it.
   - **Conductor** (`role` conductor; `action` add-records): the next design gate has too little behind it. `missing` says what to draft; for `ui` `skip_command` is the way to skip it, only on the human's words.
   - **Human** (`role` human; `action` open-gate, decide-gate, decide-unit, decide-check, decide-sync, answer-question, and the checkpoint decisions in `checkpoint.md`): has a `note` and ids, and `page_url` when a link for that gate's page or for `questions-<stage>` (an old `questions` link still counts) is recorded. See below.
3. Start every independent worker directive in one message so they run in parallel. After each worker returns, run `bf next --json` again. A unit of a later checkpoint waits (`waiting`) until the earlier checkpoint is closed; do nothing special. When `idle_reason` starts with "run done", every checkpoint is closed: that was the verification. Go to the end steps of `checkpoint.md`.

**Progress report.** While the build runs, show the output of `bf progress` and nothing else: one line, never composed by you. Each state has its own sign, units in the same state share a segment, and a unit in review or fixing carries its review round, for example `▶ U-2 building · ◐ U-3, U-4 in review, round 2 · ✔ U-1 landed`.

| Sign | State |
|---|---|
| ▶ | building |
| ◐ | in review (with the round) |
| ↻ | fixing (with the round) |
| ● | reviewed, ready to land |
| ✔ | landed |
| ⚠ | blocked or parked |
| ◎ | checkpoint being verified |
| ■ | checkpoint closed |

Do not retell a worker's report in the line. Read every state from the engine (`bf next`, `bf status` or `bf progress`) before you say it. Write a longer message only when the human must decide or act, when a unit is blocked or parked, or when the run ends.

## Waiting and the cap

- Wait for a worker only through the notice that the worker's agent ended, start no background command that waits in a loop, and stop a background command of your own when its unit is reported, blocked, parked or landed.
- Keep the cap of units in flight the human gave by pausing units with `bf unit pause`, and resume a paused unit with `bf unit resume` before the idle limit. A paused unit is not stuck: the idle clock never parks it.

## Starting a worker

1. For a builder: `bf unit start <U>` first (it resumes an existing working copy). It brings the unit up to its checkpoint branch by itself. A conflict it leaves in the working copy is the builder's to resolve, and the directive names the files in `sync_conflicts`. When a running or fixing unit needs code its checkpoint landed meanwhile, `bf unit sync <U>` brings the unit up. A conflict it reports goes to the unit's builder through the directive's `sync_conflicts`, and a builder already at work is told to read `bf brief <U> --role builder` again. The conductor never asks the human to merge and never resolves a conflict itself; only an aborted sync on a protected place goes to the human. When a directive carries `sync_aborted`, an automatic sync was aborted on files the unit may not write: tell the human which files, as the engine names them.
2. The engine already wrote the unit's brief (at `bf unit start`, when the unit enters review, and when a review sends it back to fixing); `bf brief <unit-or-checkpoint> --role <role>` refreshes it and prints the path. A checkpoint is briefed to its verifier, its reviewer and its fix builder.
3. Start the agent of that role with the Agent tool, on the model its directive names (when it names only a `tier`: cheap is haiku, standard is sonnet, strong is opus). The prompt is exactly: the brief path, the working copy path, the limits, the token (builders and reviewers), and "report as your role file says". Do not paste the brief. Never tell a cold reviewer what to leave out, and add no instruction of your own to a worker's prompt. **Tokens.** Each builder and reviewer directive from `bf next` carries its `token`; it is the same every time you ask, until a report spends it.
4. From a working copy a worker may run only `bf evidence run|verify|fail|unreachable|attach`, `bf report`, `bf record put finding`, `bf record get|list` and `bf status`. Builders report `bf report <U> --result built|fixed|blocked --token <token>`. Reviewers report `bf report <U> --result findings --role reviewer --token <token>`, with `--findings -` and the list on standard input when there are findings. Verifiers run command checks with `bf evidence run` in the checkpoint's working copy (over several repositories that is `.bf/work/<K>/`, with one folder per repository, so paths read `app/src/a.txt`), settle a check judged by an agent with `bf evidence attach <check> <file> --outcome met|failed|unreachable --says "<one sentence>" --token <token>`, or record `bf evidence fail|unreachable`. A worker that cannot go on without the human's decision runs `bf ask --title "<question>"`, stops, and finds the answer in its next brief. Do not act on a worker's summary; the engine's state decides what comes next.
5. `security-audit` is a worker directive with a `token`: a reviewer asked for a security audit of a unit in review. Start a security-auditor agent for it on the model the directive names, with the directive's brief and token. The reviewer gets the review back when the audit is filed.
   The conductor never asks for a grant of files and opens no plan repair for a file; a unit's list is a recommendation. Before it starts units, it looks at the lists of the units that have not started. When two of them share a file that matters and the human is present, it suggests an amendment that sets the order and the lists of those units from the code as it is now. The engine still refuses to start units that share a recommended file, and a list changes only by an amendment.
6. With a budget cap (`bf cost status`) every report needs `--cost <number>`, the real usage you read from the session. A worker cannot see it, so tell it to end with its result, and findings, in its last message, and run the `bf report` command yourself with its token and `--cost`.
7. When a builder's report names files as format only, run `bf unit format <U> <path>...` before starting the reviewer. When it refuses, start the reviewer as usual: a stray change is then a finding, and the builder puts the file back with `bf unit restore`.

- Load: the conductor never tells a worker to skip a check for load and writes no load limit into a prompt; it asks the human (present) or waits for its next read of `bf next` (absent). When a timing check fails a second time on a quiet machine at about the same margin, it puts the choice (run again, accept by amendment, open a fix unit) to the human instead of running a third time.

## Engine steps

- A run the shell cuts: run `bf unit land` as one foreground shell call with the shell's longest time limit (ten minutes). It may wait for a place and says so. A call the shell cuts left no receipt and freed its place; it is not a refusal and not a failed check. Run the same command again, which takes a new place at the back of the queue. Treat it as blocked, with that reason, only after two cuts in a row of a run that printed no waiting line, since then the suite itself ran past ten minutes.
- Land: a refusal about receipts means the work changed after the checks ran. Follow the steps; `bf next` names the re-run.
- While a checkpoint fix or sync runs the checkpoint's checks, other engine commands wait; one refused after its wait is run again when the checks end.
- A reviewed unit whose checkpoint has a sync, a fix or a review round open, or another landing turn running, gets no `land` directive: `bf next` lists it under `waiting` with the reason, and offers `land` again when that ends. Nothing is asked of the human.
- Close: `bf checkpoint close <K>`. It refuses while an ID has no proof at this content, or a check carries a failed or unreachable mark. Never pass `--merge` on your own.

## Human directives

- `open-gate` with `gate_kind` checkpoint-step: `bf gate open checkpoint-step --ref <K>`, render and publish, and tell the human in one chat message what to do. A checkpoint that stops (`stop` is true or it has a human step; an old record that says carry-on is a stop): end your turn. A checkpoint with no stop never reaches this directive.
- `decide-gate`: the gate waits for the human. Remind them with the link, and end your turn. When they reply, `bf gate approve|reject <gate> --quote "..."`.
- `decide-check`: a check failed or could not run. Tell the human which check and the reason it gives, and stop for their decision (fix the cause, amend the plan). The directive carries `verifier_token` so a verifier can look again.
- `decide-unit`: a unit is parked or blocked (too many review rounds, failing attempts, or a `blocked` report). Tell the human which unit and why, as the engine states it, and carry on with the other units. Only the human releases it, in their own words: `bf unit release <U> --quote "<their words>"`. They may instead re-plan through an amendment, raise `max_rounds` or `max_attempts` in the policy, or accept a finding. A unit waiting for a held file or paused needs nothing from the human. Only a park at the round or attempt limit, an extension verdict, a protected place and a confirmed block go to the human. A unit blocked by its builder's report with a reason comes to the human only after two conductor releases or the conductor's `--human` (see `judge-block`).
- `judge-block`: a builder reported `blocked` with a reason. Read the reason, then check it against `bf status` and `bf log` (a unit landed, a file now on the checkpoint branch, an approved amendment). Do not stop and do not ask the human. Never release twice on the same evidence, and when in doubt choose `--wait`.
  - Fixed: `bf unit unblock <U> --why "<what is now fixed, in plain words>"`
  - Not fixed yet: `bf unit unblock <U> --wait --why "<why it must wait>"`
  - Only the human can end it: `bf unit unblock <U> --human --why "<why>"`
- `judge-sync-block`: the resolver of a sync of the main branch reported `blocked` with a reason. Check it against `bf status` and `bf log`, as for `judge-block`. Do not ask the human. Fixed: `bf checkpoint sync <K> --unblock --judgment fixed --reason "<why>"` (the resolver gets a new turn, at most three times until the human releases the sync). Not yet: `--judgment wait`, offered again after the next landing.
- `answer-question`: a worker asked the human something. Show the question and its options in chat. When the human answers, write `{"choice": "<key>", "note": "<text>"}` to `.bf/drafts/` and run `bf answer <Q> --file <a.json>`. Only that unit waits; the rest goes on.
- `main_ahead` in `bf status --json` or `bf next --json` (information, not a directive): tell the human in one line that the main branch has work the run lacks and can be taken in at a quiet point. Never do it unasked.
- Budget: when the engine refuses new work at the cap, tell the human the spend and the cap, and stop.
- A log or head the engine says does not match: tell the human what the refusal says. Only the human decides to run `bf head accept --quote "..."`, after saying so in chat.
