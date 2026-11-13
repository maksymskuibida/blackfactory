# The engine: `bf`

`bf` is the Black Factory engine. It is one Rust binary. Each command is one short process: no daemon, no network, no background work. It keeps the facts of a project in plain files, and it refuses to move a unit, a checkpoint or a gate forward unless the facts support it.

This page lists every command with one example and the refusals you can meet. For the record kinds and their fields see [records.md](records.md).

## Build and try it

```bash
cargo build --release          # binary at target/release/bf
cargo test                     # unit, attack, resume, timing and end-to-end tests
```

```bash
cd your-git-project            # a git repository with at least one commit
bf init --name myproject       # creates factory/ and .bf/, adds .bf/ to .gitignore
bf status
```

## What lives where

| Path | What | In git? |
|---|---|---|
| `factory/config.toml` | project name, main branch, branch prefix | yes |
| `factory/policy.toml` | limits, model tiers, budget cap, review rule, guard settings, the check PATH. Only the human edits it, and a change takes effect only when the human accepts it (see `bf policy`) | yes |
| `factory/records/<ID>.toml` | one record per file | yes |
| `factory/receipts/rc-NNNNN.toml` | receipts written by `bf evidence` | yes |
| `factory/evidence/<sha256>` | files attached to a check, stored by hash | yes |
| `factory/log/events.jsonl` | the append-only run log | yes |
| `.bf/guard.json` | the signed table `bf guard` reads | never |
| `.bf/drafts/`, `.bf/briefs/`, `.bf/work/<ID>/` | drafts, briefs, working copies | never |
| `<state>/<project id>/key` | the key that seals receipts, chains the log and signs the guard table | never, and outside the project tree |
| `<state>/<project id>/head.json` | the log's head: event count, last hash, the main commit the engine last made or the human accepted, the hash and time of the accepted policy | never, outside the project tree |
| `<state>/<project id>/policy.accepted.toml` | the policy the human accepted: **this** is the policy in force, not the file on disk | never, outside the project tree |
| `<state>/<project id>/tokens.json` | hashes of the one-use role tokens | never, outside the project tree |

`<state>` is `$BF_STATE_HOME`, else `$XDG_STATE_HOME/blackfactory`, else `~/.local/state/blackfactory`. `bf init` creates `<state>/<project id>/` with owner-only permissions (0700; the key is 0600); the project's `factory/config.toml` stores only the id. A fresh clone has no key there, so it cannot verify the log or old receipts: run `bf init`, then the human runs `bf head accept` (see below) and the checks are run again. State is never stored in a database. Everything about a run (which unit is where, which gate is open) is replayed from the log each time, so a killed command leaves nothing half-written that a later command cannot read.

## The run log

One JSON object per line. Each event has `seq`, `at`, `kind`, its data, `prev` (the hash of the event before) and `h` (its own hash). `h` is an HMAC-SHA256 under the project's key, which lives outside the project tree: a line written by hand, with a correct `seq` and `prev` and a plain hash, does not verify. Editing or deleting a past line breaks the chain. A line cut short by a killed process is ignored and removed by the next write.

**The head.** The state folder also holds the log's head: how many events there are and the hash of the last, written under the same lock as every append. Every command that reads or changes state compares the log with it. A log that is **shorter** than the head, or whose event at that place **differs**, was rolled back (an old copy put back, `git stash`, a checkout, a restore) and is refused with the reason and a recipe that never says "restore from git". A log that is longer is accepted when the extra lines verify (a command was killed between the append and the head).

**Recovery: `bf head accept --quote "<the human's words>"`.** Only the human decides to keep a log that does not verify or does not match the head. After they say so in chat, this command chains the whole log again under the current key, sets the head to its end and appends a `head.accept` event naming what was wrong. It needs an unused human turn and a quote, like an approval. It also accepts where the main branch stands now (see `bf checkpoint close`), and it is the way to take a cloned project under a new key (old receipts must then be run again). `bf doctor` and `bf log --verify` still work on a damaged log.

## The policy

The policy in force is the copy the human accepted, kept in the state folder with its hash in the log head; the file `factory/policy.toml` is where the human edits. `bf init` accepts the policy that is there the first time. After that, **every command refuses while the file on disk differs from the accepted copy**, with the reason and a recipe: the human reads the change with `bf policy diff` (lines removed and added), and if they agree they say so in chat and `bf policy accept --quote "<their words>"` takes it (a decision like an approval, logged as `policy.accept`). Running `bf init` again does not accept a changed policy. The guard table, the role-token setting, the approval words, the check PATH, the transcript root and every other setting are read from the accepted copy only, so an edit of the file switches nothing on or off by itself. `bf doctor` and `bf log` still run while the policy is unaccepted.

## Common behaviour

- Exit codes: `0` done, `2` refused, `1` error.
- `--json` prints data as JSON. A refusal under `--json` is printed to stdout as `{"ok": false, "refused", "because", "do_this", "it_worked_when"}`.
- Every refusal has four parts, printed in this order on stderr: `refused:` what was refused, `because:` why, `do this:` numbered steps, `it worked when:` how to tell.
- `-C <dir>` runs the command as if started in `<dir>`.
- From inside a unit's working copy, `bf` finds the project's main checkout by itself. State always lives in the main checkout.
- Every command that changes state rewrites `.bf/guard.json`.

## Commands

### `bf init [--name N]`

Creates `factory/`, `.bf/`, a default policy and config, the sealing key, and the guard table, and adds `.bf/` to `.gitignore`. Safe to run again: it only refreshes the key, the guard table and the ignore line.

```bash
bf init --name myproject
```

Refuses when: the folder is not the top of a git repository.

### `bf record put <kind> --file <f>` / `get <id>` / `list [--kind k]`

`put` validates a TOML (or `.json`) file against the schema of its kind and writes `factory/records/<ID>.toml`. A file without `id` gets the next free one. `get` prints a record, `list` prints `ID title` lines.

```bash
bf record put requirement --file r1.toml
bf record get R-1
bf record list --kind unit
```

Refuses when: the record does not match its schema (every problem is listed); the `id` is taken by a different record and `--replace` was not given; the kind is unknown; the file cannot be read; the record's stage gate is approved and the content changes without an approved amendment.

### `bf id new <kind>`

Prints the next free ID of that kind, for example `R-8`, and reserves it: the next `id new`, and a `put` of a file with no `id`, hand out a later number. Numbers may have gaps. To use a reserved ID, write it into the file's `id` field.

**The ID rule for `bf record put`.** It never renumbers silently. (1) A file with no `id` gets the next free ID, and the command says so in plain words (`put R-9 (the file had no id, so the engine assigned R-9 ...)`; `"assigned": true` in JSON). (2) A file whose `id` belongs to a *different* record is refused with the way out: put a fresh ID (`bf id new`), or run the same command with `--replace` to revise that record on purpose. (3) The same content again is accepted and changes nothing. (4) Findings are the exception: a reviewer replaces one by putting it again (to set its `outcome`). (5) **Frozen stages.** Once a stage's gate is approved, a changed record and also a **new** record of that stage's kinds (a unit, handoff, check or checkpoint after the plan gate; a requirement after the requirements gate; and so on) is refused unless an approved amendment names it. `bf unit start`, `bf unit land`, `bf checkpoint close` and `bf doctor` also compare the records with what the gate approved (its snapshot) and refuse on drift, so a record edited or added by other means is found. (6) IDs have no leading zeros (`R-007` is refused). (7) Allocation and writing happen under a lock, so parallel puts never share an ID.

### `bf ledger check [--plan]` / `bf ledger impact <ID>...`

`check` lists every designed part that has no unit, no check or no evidence, and refuses while any exists. `--plan` checks only units and checks (use it before building). The designed kinds are set in `factory/policy.toml` under `[ledger] kinds`. Evidence means a sealed, passing receipt from a check of a **closed** checkpoint that proves the part, at the content that checkpoint closed on (a receipt from a version that no longer exists counts for nothing); for a human check, the checkpoint's approved step gate.

`impact` prints everything linked to the IDs: units that deliver them and units that depend on those, handoffs, checks, checkpoints and receipts.

```bash
bf ledger check
bf ledger impact R-1 C-2 --json
```

Refuses when: any designed part lacks a link.

### `bf plan check`

Checks the delivery plan: every handoff is proved by some check (a check's `proves` may name a handoff `H-n`; so may a checkpoint's); no cycle in the unit graph; every handoff has a sender, a receiver (which depends on the sender) and a named thing; every checkpoint names the IDs it proves and a check for each; every designed ID belongs to a unit and to a checkpoint; every unit has a check; units in one wave share no files. Waves are the levels of the dependency graph.

Refuses when: any of that fails (all problems are listed).

### `bf run start` / `bf run stop` / `bf status` / `bf log`

`run start` begins a run (it needs a plan that passes `plan check`). `run stop` ends it. `status` prints the project, the run, test mode, every unit's state (`planned`, `ready`, `running`, `review`, `fixing`, `reviewed`, `blocked`, `parked`, `landed`) and every gate. `log` prints events: `--tail N` for the last N, `--verify` to check the hash chain.

Refuses when: a run is already active; the plan has problems; the log chain is broken.

### `bf next`

The next directives as data: for each, `role` (`builder`, `reviewer`, `verifier`, `engine`, `human`), `action`, the `unit` or `checkpoint`, `tier` (`cheap`, `standard` or `strong`, from the policy), `working_copy`, `brief` (the path `bf brief` will write), `limits` and the `commands` to run. Engine steps have `role: "engine"` (`land`, `close-checkpoint`) and a single `command`; worker directives carry a list of `commands` and the verifier's directive for a checkpoint has `brief` and `limits` like the others; human steps have `role: "human"` (`open-gate`, `decide-gate`, `decide-unit`). Before the plan gate is approved the only directive is for the human. `waiting` says why other units are not ready.

It also says plainly whether anything can start: `can_start` (a worker or engine step exists), `human_needed`, and `idle_reason`, which is null when something can start and otherwise says why not: `run done: ...`, `units parked or blocked: U-3 (parked); the human decides`, `waiting on a human step`, or `nothing is ready: ...`.

```bash
bf next --json
```

Refuses when: no run is active; the budget cap is reached.

### `bf brief <unit-or-checkpoint> --role <builder|reviewer|verifier>`

Given a checkpoint ID (`K-1`), it writes the verifier's brief for the checkpoint: what it proves with acceptance, its checks with the exact commands, no source scope; other roles are refused for a checkpoint. Given a unit, it writes the exact context for that worker to `.bf/briefs/<unit>-<role>.md` and prints the path: the unit, the design parts it delivers with their acceptance, the contracts it receives and hands over, its checks with the exact command, its file scope, open findings, limits and rules. A reviewer's brief also lists the unit's receipts. A verifier's brief leaves out the file scope and tells it not to read the source.

### `bf report <unit> --result <built|findings|fixed|blocked> --token T [--role R] [--note T] [--findings f] [--cost N]`

The only way a worker changes a unit's state.

**Role tokens.** With `[roles] tokens = true` (the default), `bf next` hands the conductor a one-use `token` in each builder and reviewer directive, and `bf report` needs the token for the role it reports as: the builder's token for `built`, `fixed` and `blocked`, the reviewer's for the review. A token is bound to the unit and the role, is spent by one successful report, and is **shown once**: while it is unspent, a further `bf next` prints `token: null` and says it was already handed out. If one is lost, the human runs `bf tokens reset --quote "<their words>"`, which clears all unspent tokens. A builder never holds the reviewer's token: it is created only when the unit is in review, and `bf next` and `bf brief` refuse to run from inside a working copy. **Cost.** With a budget cap, every report must carry `--cost <number>` (the conductor reads the session's real usage); text that is not a number is refused, never counted as zero.

| Unit is | Report | Becomes |
|---|---|---|
| `running` | `built` | `review` (or `reviewed` if the policy does not require review) |
| `review` | `findings --role reviewer [--findings f.json]` | `fixing` while a finding is open (new ones from the file, or filed with `bf record put finding`); `parked` after more than `max_rounds` rounds; `reviewed` when there is no finding at all |
| `review` | `built --role reviewer` | `reviewed` (refused while findings are open) |
| `fixing` | `fixed` (or `built`) | `review` |
| `running`, `fixing` | `blocked` | `blocked` |

`--findings -` reads the list from standard input, so a reviewer needs no file. A findings file is a JSON list such as `[{"title": "...", "severity": "high", "where": "src/a.rs:10", "action": "..."}]`; each entry becomes a finding record with an ID that never changes. `--cost` adds to the budget count.

A finding is open until its `outcome` is `fixed` or `accepted`. A reviewer can file one from the unit's working copy with `bf record put finding --file f.toml`, but only a new, open finding for a unit that is in review (workers may put nothing else, and may not mark a finding fixed or accepted); the human sets `outcome = "accepted"` by putting it from the main checkout.

Refuses when: no run is active; the report does not fit the unit's state; a unit in review is reported on without `--role reviewer`; a clean review is reported while findings are open.

### `bf unit start <U>` / `bf unit land <U>`

`start` creates the unit's working copy (on a unit that is already in flight it only makes sure the working copy exists) at `.bf/work/<U>` on branch `bf/<U>`, cut from the local branch of its checkpoint (`bf/<K>`, itself cut from the main branch the first time). It needs an active run, an approved plan gate, and the unit's dependencies and handoffs satisfied. Running it again resumes the same working copy.

`land` merges the unit's branch into its checkpoint branch (no network). It re-checks first: the unit is `reviewed`; no finding is open; every check of the unit has a sealed passing receipt for the content now in the working copy; every changed path is inside the unit's file scope. Pending changes are committed in the unit's working copy first.

```bash
bf unit start U-1
bf unit land U-1
```

Refuses when: the plan gate is not approved; the unit is not ready or not reviewed; a receipt is missing, unsealed or for other content; the branch changes files outside the scope; the merge conflicts (the merge is aborted).

### `bf evidence run <check> [--unit U] -- <command...>`

Runs the command itself, in the current working copy, with output and errors merged. It stores a bounded tail of the output (4000 bytes by default), the SHA-256 of the full output, the exit code and the content hash (see below), and writes a sealed receipt.

```bash
cd .bf/work/U-1
bf evidence run X-1 --unit U-1 -- cargo test
```

The command runs through `/bin/sh -c` with an environment of its own: nothing is taken from the caller except a short list (`HOME`, `USER`, `LANG`, `TMPDIR`, `CARGO_HOME`, ...), and `PATH` is the one in the policy's `[exec] path` (captured at `bf init`, editable by the human only). A fake program first on the caller's `PATH` cannot stand in for the real one. Every git call the engine makes also runs without the caller's `GIT_*` variables, with the policy's `git` program and no user or system git config, so `GIT_DIR` or `GIT_WORK_TREE` cannot point the content hash at another repository. **On macOS every check, and every git call, runs under `/usr/bin/sandbox-exec`** with a profile that denies the network, reading or writing the project's state folder, writing under `factory/` and under the main checkout's `.bf/` outside the working copy in use. The engine tests at start whether `sandbox-exec` works; where it does not (other systems, or a refusal) the check runs **without** a sandbox and the receipt says `sandbox = "none"`, as does `bf doctor`. `[exec] sandbox = false` in the accepted policy switches it off. On the test machine the sandbox adds about 80 ms to one `bf evidence run`. Before a check starts, the folders on the policy's `[exec] path` are checked: one inside the project, or one that was created or changed after the policy was accepted and is not `/bin`, `/usr/bin`, `/sbin` or `/usr/sbin`, stops the check and is named (a file replaced inside a folder does not change the folder's time, so this is not complete). The receipt records the program (`/bin/sh`), whether it ran in the sandbox, and the hash of the check record it was made for; if the check record changes later, the receipt stops counting.

The command must equal the `command` stored in the check record. Otherwise an agent could run `true` under the name of a real check. The exit code of `bf` is 1 when the command failed (the failing receipt is still stored). Nobody writes a receipt by hand: a receipt without a valid seal is ignored everywhere.

Refuses when: no command is given; the ID is not a check, or the check is not a command check; the command differs from the check's own; the command changed the working copy while it ran (the receipt would describe neither state).

### `bf evidence attach <check> <file>` / `bf evidence verify [--unit U] [--checkpoint K]`

`attach` stores a file by hash under `factory/evidence/` and writes a sealed receipt bound to the content. It is proof only for a check done by an `agent`. A `human` check is settled only by the checkpoint's approved `checkpoint-step` gate, never by a file; a `command` check only by running its command. The size limit is in the policy.

`verify` checks the receipts: the seal and the exit code of each, and, with `--unit` or `--checkpoint`, that every check in scope has a sealed passing receipt at the content now in that working copy. Older receipts for other content are listed as `OTHER` and do not fail the command when a current one exists. It exits 2 when anything fails.

### `bf evidence fail|unreachable <check> --reason "..."`

The verifier's way to say that a check ran and does not match (`fail`), or could not be run (`unreachable`), when there is no passing receipt to show. It writes a sealed record bound to the content, in the working copy where it is run. It is never proof. A checkpoint whose check carries such a mark does not close (the refusal names the check and the reason); `bf next` shows a human directive `decide-check` (so `can_start` is false and `idle_reason` says a check failed or is unreachable); the checkpoint view marks the row and sets `needs_you`. A later passing receipt of the same check at the same content replaces the mark, so once the cause is fixed the verifier just runs the check again.

### `bf checkpoint close <K> [--merge]`

Closes a checkpoint (the records must match what the gates approved). All its units must have landed, its working copy must be clean, and every ID in its `proves` list needs a sealed passing receipt, from one of its listed checks, at the content now in the checkpoint's working copy, or an ID waived by an approved amendment. A checkpoint with a `human_step` also needs an approved `checkpoint-step` gate for it.

`--merge` also merges the checkpoint branch onto the main branch (the main checkout must be on it). It needs a human act **after the closure**: the checkpoint must be closed, and an approved `checkpoint-step` gate for it (real, not simulated) must have been opened after the close event (`bf gate open checkpoint-step --ref K`, the human says merge in chat, `bf gate approve <gate> --quote ...`). Merging twice does nothing the second time. A test run refuses it before anything else. After a merge the engine remembers the main commit it made. **The main branch is compared with the commit recorded in the log head** (set at `bf init`, after an engine merge, and when the human accepts). If it moved and the engine did not move it, `bf status` says so (`main.ok` false, with the commits' files) without refusing; `bf unit land`, `bf checkpoint close` and `bf ledger check` refuse until the human accepts the new position with `bf head accept --quote "<their words>"`. A human's commit that touches only `factory/` and `.gitignore` is not a move. Work of a unit that never landed, or landed in a test run, found on the main branch is always a problem, and accepting the position does not clear it: the human moves the branch back. A run that contains a simulated approval, or a unit landed in a test run, can never be merged.

```bash
cd .bf/work/K-1
bf evidence run X-3 -- cargo test --all
cd ../..
bf checkpoint close K-1
```

Refuses when: units have not landed; the working copy has uncommitted changes; an ID has no proof; a human step is not approved; with `--merge`, the run is tainted by simulated approvals or the main checkout is on another branch.

### `bf human-turn`

For the harness's `UserPromptSubmit` hook. Reads the hook payload on stdin and, when it is a message the human typed, appends a `human.turn` event with the text. It prints nothing (a hook's output goes to the model). It always exits 0 on a recorded or rejected message, and 1 on an error; it never exits 2, because that would erase the human's prompt.

A payload is **not** a human turn when: it does not say `hook_event_name: UserPromptSubmit`; it has no `session_id`; it names no `transcript_path`, or that file is not **under the harness's transcript root** (`[human] transcript_root`, default `~/.claude/projects`), or it does not parse, or **its last human message is not this prompt**; it carries `agent_id`, `agent_type`, `parent_tool_use_id`, `subagent` or `agent_name`; its `source` is set and is not the user; the prompt is empty; the prompt starts with a wrapper tag that programs use (`<system-reminder>`, `<task-notification>`, `<side-sessions-event>`, `<ci-monitor-event>` and similar); its `cwd` is outside the project or inside a unit's working copy; or the policy lists the human's session ids (`[human] sessions`) and this one is not among them. Rejected payloads are logged as `human.turn.rejected` with the reason and never satisfy anything. A recorded turn also stores the result of one more check on the transcript file (`transcript`): `match` when the last human message in it is this prompt, `differs` when it is another one (the file may not yet hold the prompt when the hook runs), `unparsed` when no human message could be read. The comparison is strict: a turn whose transcript does not end with it is rejected and logged as such. A harness that writes the transcript only after the hook runs can set `transcript_match = "soft"` in the accepted policy, and a mismatch is then only noted.

The engine cannot tell a person from a program that writes to the hook's input. The hook is the source of human turns, the checks above remove the obvious fakes, and the quote on every decision (below) makes it visible which words were taken. See "What the engine does not stop".

### `bf gate open <kind> [--ref ID]` / `approve` / `reject` / `status`

Kinds: `idea`, `requirements`, `ui`, `architecture`, `plan`, `checkpoint-step` (needs `--ref K-<n>`). A gate has rounds; its ID is `<kind>-r<round>` (or `<kind>-<ref>-r<round>`). Opening records a snapshot of the records the page shows, so the next round can mark what is new, changed or removed.

`approve <gate> --quote "<words>"` and `reject <gate> --quote "<words>"` need a human turn recorded **after** the gate opened and not yet used for anything else. One typed message decides one thing. **The quote** is the words of the human's message that decide this gate. The engine cannot understand a sentence, so it applies plain rules and refuses what is not plain:

- the quote must be the **whole message or one whole sentence** of it, not a few words cut out;
- for an approval (`approve`, `test-mode grant`, `amend approve`, `policy accept`, `head accept`, `tokens reset`): the quote holds at least one word of the policy's `[human] approval_words`, and the message holds **no** word of `refusal_words` anywhere (not, no, don't, never, reject, stop, wait, change, and the same in German, French and Spanish), and the quote is not a question;
- for a rejection: the quote holds a refusal word and the message holds no approval word;
- a message with words of both kinds is refused: the conductor is told to ask the human to say it again plainly, in one sentence;
- a test-mode grant must also speak of a test run.

The quote is stored with the decision, and so is the human's **whole** message. `bf gate status`, `bf gate approve --json` and the gate page ("Decided on the words", "The human's whole message") show both, so a reader sees exactly what was taken as the decision and what it was cut from. The quote is there for the human to audit. `approve --simulated` is described next. A bare kind (`plan`) means its latest round.

`bf gate open` prints the gate ID. `bf gate reject` closes the round; the next `bf gate open <kind>` starts round n+1.

```bash
bf gate open architecture
# the human says "I approve the architecture" in chat; the hook records it
bf gate approve architecture --quote "I approve the architecture"
```

Refuses when: no quote is given, it is not in the message, or it is not a whole sentence; the message mixes approval and refusal words, or is a question; the kind is unknown; a gate of that kind is still open; unanswered questions are tagged for this stage (see `bf questions`); for `plan`, the plan has problems; no human turn follows the open; the gate is already decided.

### `bf test-mode grant`

A grant for one run, made from the human's latest message (it must be unused). If a run is active it applies to that run; otherwise to the next `bf run start`. A grant that is waiting for its run also lets the five design gates (`idea`, `requirements`, `ui`, `architecture`, `plan`) be approved with `--simulated`, so a test run can go through them in order before `bf run start`; the checkpoint-step gate needs the run. Inside a granted run, `bf gate approve <gate> --simulated` satisfies a gate and is recorded as simulated. Outside one it satisfies nothing: the command refuses, and an approval event written into the log by other means is ignored when the state is replayed.

Needs `--quote` like an approval. Refuses when: the human's latest message is missing, already used, only a question, or does not contain the quote.

### `bf guard`

The per-tool-call hook. See the section on the guard below.

### `bf view <kind> [id] [--json]` / `bf render <kind> [id] --template <file> --out <file>`

`view` prints the page data in the view contract (version 1, `contract: 1`). Kinds: `index`, `gate` (id is a gate ID or kind; a kind never opened gives a draft page), `questions` (id is a stage), `ledger`, `plan`, `run`, `checkpoint` (id is a checkpoint), `coverage`. The output is always JSON.

The `index` page is a list of pages: sections of rows, each with `href` and `meta`. The links follow one naming convention, so a conductor that renders each page under that name gets working links: `gate-<gate id>.html`, `questions.html`, `plan.html`, `run.html`, `ledger.html`, `coverage.html`, `checkpoint-<K>.html`.

`render` also sets the page title: if the template's title is `<title>Black Factory</title>` it becomes `<page title> · Black Factory`, and a `/*BF_TITLE*/` marker, if the template has one, becomes the page title.

`render` puts the JSON into an HTML template at the marker `/*BF_DATA*/` and writes one file. If a literal placeholder (`null`, `undefined`, `{}`, `[]`) follows the marker it is replaced too, so `const DATA = /*BF_DATA*/ null;` works. The JSON is made safe for a script element (`</` and line separators are escaped).

```bash
bf render gate architecture --template views/templates/page.html --out out/architecture.html
```

Refuses when: the template cannot be read or has no marker; the page kind is unknown; no gate matches.

### `bf ask --file q.json [--stage S]` / `bf answer <Q> --file a.json` / `bf questions [--open]`

A stage's questions come before its gate. `ask` takes a JSON list of questions in the view contract's shape without IDs: `[{"title", "kind": "single|multi|text", "context", "stage", "options": [{"key", "text", "proposed", "why"}]}]` (a table with a `questions` list, or one question, also works). The engine assigns `Q-` IDs and prints them; a bad entry writes nothing. `--stage` fills the stage of entries that have none. `answer` needs a human turn recorded after the question was asked (the turn is stored as `answered_turn`), and takes one file per question, for example `{"choice": "A", "note": "..."}`, and stores it as the answer (or the file's `answer` value if it has one). `questions` lists them.

### `bf amend open <ID>... ` / `narrow --file f` / `approve [A-n]`

`open` computes the impact set from the ledger. `narrow` takes `{"keep": [...], "waive": [...], "reasons": {"<ID>": "why"}}`: it may only remove IDs from the set, every removed ID needs a reason, and every waived ID needs a reason. `approve` needs a human turn after the narrowing. An approved amendment lets its IDs be replaced with `bf record put` after their gate was approved, and its `waive` list stands in for a receipt at `checkpoint close`.

### `bf cost status` and the policy

`bf cost status` prints spent, cap, tiers and per-unit cost. `--cost` on `bf report` adds spend. When `[budget] cap` in `factory/policy.toml` is above 0 and spend reaches it, `bf next` and `bf unit start` refuse.

### `bf stuck [--dry-run]`

Parks every running unit with no event for `idle_minutes`, and lists all parked units. Units parked by review rounds are listed too.

### `bf doctor [--perf]`

`doctor` checks the key, the log chain, the guard table, the ignore line, ignored events and the plan. `--perf` times each class of command in the current project and prints medians and 99th percentiles, including `bf guard` as a real child process.

## The content hash

A receipt is bound to the content it ran on. The content hash is the **git tree-object hash of every file under the working copy's top folder that is tracked, staged, or untracked and not ignored, as the files are on disk when the command starts, leaving out the folders `factory/` and `.bf/`**. It is computed in a temporary copy of the index (`git add -A`, `git rm --cached` for the two folders, `git write-tree`), so the real index and the history are untouched. Files that `.gitignore` covers do not count; add build output to `.gitignore`. The hash is taken before and after a command runs; if they differ, no receipt is written.

For a unit the working copy is `.bf/work/<U>`; for a checkpoint it is `.bf/work/<K>`; in the main checkout it is the checkout itself.

## The guard

`bf guard` is the hook that runs before every tool call. It reads the tool call on stdin and the project's guard table, `.bf/guard.json`, and checks the table's signature with the key in the state folder (the one extra small file it reads). It starts no process and calls no git; it resolves paths with `canonicalize`, so a symlink cannot hide a target.

**Which project, which table.** The guard looks for the project from the payload's `cwd`, from the file a write tool names and from the absolute paths a shell command names. When the hook command carries `--root <dir>` (or `$BF_ROOT` is set) that folder is the project and the only one looked at, so a `factory/` folder further up cannot take its place; the harness should always pass `--root "$CLAUDE_PROJECT_DIR"`. The **outermost** folder above each of them that has a `.bf/guard.json` or a `factory/` folder is the project, so a table planted deeper (inside a working copy, for example) is ignored. A project with a `factory/` folder whose table is missing, empty, not signed, signed wrongly or not its own **denies** every call, and says why; only a folder with neither is "not a factory" and left alone.

Configure it in the project's `.claude/settings.json` (the matcher must include the read tools):

```json
{
  "hooks": {
    "PreToolUse": [{ "matcher": "Write|Edit|MultiEdit|NotebookEdit|Bash|Read|Grep|Glob", "hooks": [{ "type": "command", "command": "bf guard" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "bf human-turn" }] }]
  }
}
```

**Input** (Claude Code's `PreToolUse` payload; only these fields are read):

```json
{ "tool_name": "Write", "tool_input": { "file_path": "/abs/path/factory/policy.toml" }, "cwd": "/abs/path" }
```

**Output** (always exit code 0). A call the table forbids:

```json
{ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny",
  "permissionDecisionReason": "refused: write factory/policy.toml\nbecause: ...\ndo this:\n  1. ...\nit worked when: ...\n" } }
```

A call it does not forbid prints `{}`: the guard **abstains**, so the harness's own permission prompts stay on. An explicit `allow` is printed only when the policy asks for it (`[guard] allow = "allow"`), because Claude Code runs an allowed call without asking.

**What the table holds** (signed, rewritten by every state-changing command): the project root; the protected paths (`factory/`, `.bf/`, `.claude/settings.json`, `.claude/settings.local.json` and the policy's `protected_extra`); the state folder; the commands a worker and a verifier may run; and one entry per active unit and per checkpoint with its working copy path and file scope.

**What it decides:**

- `Write`, `Edit`, `MultiEdit`, `NotebookEdit`: denied on protected paths (after resolving `.`, `..`, absolute paths and symlinks) and in the state folder; inside a unit's working copy, denied outside the unit's `files`; denied in a checkpoint's working copy; denied outside its own working copy for a session that works in a unit's. Open to the conductor: `.bf/drafts/` and `factory/idea.md`.
- `Read`, `Grep`, `Glob`: the state folder and its parent folder are denied.
- `Bash`: quotes and backslashes are stripped (`g"it" push`, `b\f human-turn`), the text is split into commands, and then:
  - any text containing `human-turn` is denied;
  - **`bf` must be called plainly.** A `bf` whose subcommand is not a literal word from the list of subcommands (`bf $(echo human)-turn`, `bf ${x}`, `bf "$(printf ...)"`), or that comes after `eval`, `sh -c`, `bash -c`, `env`, `xargs`, `command`, a backtick or `$(`, is denied;
  - any command that names the state folder, the variables that locate it (`BF_STATE_HOME`, `XDG_STATE_HOME`) or its parent folder is denied;
  - any command that names `factory/`, `.bf/` or the hook settings, in any spelling that resolves there, is denied **unless it is a `bf` command or a plain read** (`cat`, `ls`, `head`, `jq`, `sed -n`, `awk`, `sort`, `grep`, `git status|diff|log|show|add|stash list`, ...). Redirects to `/dev/null` and `2>&1` do not count; any other redirect does. So `python -c`, `perl -e`, `node -e`, `sh -c`, `find -delete` and here-documents that touch those paths are denied, and `cat factory/records/R-1.toml 2>/dev/null` is not;
  - writing under the harness's transcript root is denied (reading is not);
  - the git commands that can move the main branch or a checkpoint branch or lose records (`push`, `pull`, `update-ref`, `fetch` with a refspec, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `stash`, `clean`, `reset --hard`, switching to or committing on an engine branch, forced `branch`, `worktree`, `--git-dir`, `git config` of an alias, filter, `core.*` or include) are denied unless the **target** (`-C dir` or the working directory) is a unit's own working copy; a git subcommand the guard does not know (it may be an alias) is denied, and so is plumbing such as `commit-tree`; ordinary `git commit`, `git add` and `git checkout <ref> -- <path>` are left to the user, and the main-branch watch (above) looks at what lands;
  - from a unit's working copy (a landed unit's old copy counts too) the only `bf` commands are `evidence run|verify|fail|unreachable|attach`, `report`, `record put finding`, `record get|list` and `status`; from a checkpoint's working copy the same without `report` and `record put`. `bf -C`, a `cd` out of the working copy, and a write whose target is elsewhere in the project are denied there.
- Every other tool is allowed (abstained).

Measured in this repository's tests, the guard takes about 2 ms per call as a separate process (design target: 30 ms median).

The engine also enforces the worker limits itself, without the hook: a command run from inside a working copy (by the process's real current directory, not by `-C`) is limited to the same list, and `bf next`, `bf brief`, gates, landing and planning are refused there.

## What the engine does not stop

The guard and the checks stop mistakes, drift and casual cheating, and make deliberate cheating visible. They do not make a determined agent honest. There are two cases.

**A worker with only the guarded tools (Write, Edit, Read, Grep, Glob, and Bash through the hook).** After the checks above it can no longer, in a few calls, record a human turn, approve a gate with the human's refusal, edit the policy or the check records unnoticed, add a unit to an approved plan, read the key by name, plant a program in a folder it was told about, or move the main branch without the engine saying so. What it can still do:

- **Hide a path.** The guard reads command text. A path built at run time (`'fac' + 'tory'` inside a script, a variable, a glob such as `fact*/`) or a script file that does the work (`sh src/x.sh`) is not seen. The sandbox around checks and git helps only where the engine starts the process; a worker's own Bash call is not sandboxed unless the harness does it.
- **Run its own code as a check.** A check runs the unit's code; that is the point of it. On macOS the engine runs it in a sandbox (no state folder, no network, no writes to `factory/` or other working copies); elsewhere it runs unsandboxed and the receipt says so. Either way the code can do what it likes inside its working copy and its temporary folder, and what a check proves rests on someone reading that code.
- **Plant a program where checks look.** The folders on the check PATH are refused if they changed after the policy was accepted, but a program replaced inside an unchanged folder is not seen, and a standard system folder is trusted.
- **Take the reviewer's token.** Tokens are shown once, to the conductor. A script run by the worker that changes folder to the project and runs `bf next` before the conductor does would be handed the reviewer's token (the guard cannot see inside a script, and the engine only sees that the current folder is the project). Order matters: the conductor should ask for the next directives as soon as a unit reports.
- **Use an older, still valid guard table.** The table is signed but carries no counter, so a copy from before a state change is accepted until the next state change.
- **Race a symlink.** Paths are resolved when the guard looks; a link swapped afterwards is not seen.
- **Move main, then be noticed.** The engine sees a move of the main branch the next time it reads, and refuses to land, close or call the ledger closed until the human accepts the position. It does not stop the move: only a host's branch protection does.

**An agent with an unrestricted shell under the same user account.** It can read the state folder (`~/.local/state/blackfactory/<id>/`): with the key it can seal receipts, chain log lines, sign a guard table, rewrite the log and its head together, and hand itself role tokens. It can write anywhere the user can, including the policy and the transcripts. Nothing in this engine stops that. Real separation needs:

- **the harness's sandbox**, so that worker sessions cannot read or write the state folder, `factory/`, `.bf/`, the transcripts and other working copies at all (layer 1 of D-3; not built here); and
- **a host identity the agent does not hold**: a pull request on the host that only the human merges, with required checks that re-verify every receipt.

What each protection gives, and what stays open:

| Protection | What remains |
|---|---|
| Key and head outside the project tree | Same-account shell can read them; forging a line needs the key, undoing a rollback needs the head |
| Signed guard table, outermost project, `canonicalize` | A same-account agent can still write files by means the command text hides |
| Accepted policy in the state folder | A same-account agent can replace the copy and the head together |
| Role tokens, shown once | A script that asks for a token before the conductor does; a leaked token is a role |
| Whole-sentence decisions with refusal words | The engine cannot understand a sentence; it cannot tell "Yes, but only if" from "Yes". The quote and the whole message are shown for the human to audit |
| Hook payload checks and the transcript root | The transcripts are the harness's own files; a program that can write them can make a human turn |
| Clean environment, sandbox and PATH check | Not complete: see above |
| Main-branch watch | It notices after the fact |
| Cost on every report | The number is still the conductor's word |
| Frozen records and receipt-to-check binding | A same-account agent could edit records and the log together |

## Git: the `git` program, not a library


The engine runs the `git` program for every git read and write. It does not use a Rust git library such as `gix`, and its dependencies contain none. It makes one to three git calls per command that needs git (the content hash is three calls; creating or merging a working copy is one to three), and none at all from `bf guard`, `bf human-turn`, `bf next` (before a checkpoint is ready), queries and state changes of records. The reasons: a library would add many crates and a long build for the few operations needed here, and the operations that matter (a tree hash of the files on disk including untracked ones, worktrees, merges) are the ones where the `git` program is the reference behaviour. The measured times of every class of command are in the test output (`cargo test --test timing -- --nocapture`).

## Testing aids

`BF_CRASH_AT=<point>` makes `bf` exit with code 137 at a named point, to test resuming a killed run. The points are `unit-start-after-git` and `land-after-merge`. It does nothing else.
