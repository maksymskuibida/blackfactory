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
| `<state>/<project id>/head.json` | the log's head: event count, last hash, the main commit the engine last made or the human accepted, the main branch's name, the hash and time of the accepted policy | never, outside the project tree |
| `<state>/<project id>/policy.accepted.toml` | the policy the human accepted: **this** is the policy in force, not the file on disk | never, outside the project tree |
| `<state>/<project id>/tokens.json` | the token of each open worker directive | never, outside the project tree |
| `<state>/<project id>/programs.json` | where each command check's program was found and a hash of that file, recorded when the plan gate is approved; a change is noted in the receipt | never, outside the project tree |
| `<state>/<project id>/guard.gen` | the number of the newest guard table; an older signed table is refused | never, outside the project tree |

`<state>` is `$BF_STATE_HOME`, else `$XDG_STATE_HOME/blackfactory`, else `~/.local/state/blackfactory`. `bf init` creates `<state>/<project id>/` with owner-only permissions (0700; the key is 0600); the project's `factory/config.toml` stores only the id. A fresh clone has no key there, so it cannot verify the log or old receipts: run `bf init`, then the human runs `bf head accept` (see below) and the checks are run again. State is never stored in a database. Everything about a run (which unit is where, which gate is open) is replayed from the log each time, so a killed command leaves nothing half-written that a later command cannot read.

## Workspaces: the factory apart from its code

Two ways to set a project up, one code path. **A project folder** (`bf init` in a git repository) holds the factory and the code together: its one code repository is the folder itself. **A workspace** (`bf init --workspace` in a folder of its own) holds only the factory: `factory/` (records, receipts, the log), `.bf/` (derived files and the working copies) and the harness files. It is normally a private git repository of its own (`bf init --workspace` makes it one if the folder is not one yet), and points at one or more code repositories listed in `factory/workspace.toml`:

```toml
[[repo]]
name = "app"
path = "../app"        # absolute, or relative to the workspace
main_branch = "main"
```

- `bf repo add <name> <path> [--branch B]` adds a repository and notes where its main branch stands; `bf repo list` shows them **The recommended layout is the workspace as the root the human opens, with each code repository a folder inside it** (`./app`, with its own `.git`). The path must be the top of its own git repository with that branch (the branch defaults to the one checked out), not the workspace itself and not inside `factory/`, `.bf/`, `.claude/` or `.git/`; a path beside the workspace (`../app`) works as well. For a repository inside the workspace the engine adds `/<folder>/` to the workspace's **local** ignore file, `.git/info/exclude` (not `.gitignore`), once, and says so; `bf init` and `bf doctor` put the line back when it is missing (a fresh clone of the workspace has no exclude file), and `bf doctor` has a line for it. Nothing of a nested repository is ever staged or committed into the workspace by the engine: it makes no `git add` in the workspace, and content hashes are taken only of working copies. A project folder has exactly one repository, named `code`, at `.`.
- All `bf` commands run from the workspace, from inside a working copy, or from inside a nested code repository: a command finds its project by walking up to the folder that holds `factory/`. **A workspace above wins** over any nearer folder, so a code repository that itself contains a folder named `factory/` (even one with a `config.toml`) is never taken for the project, whether the command runs from its checkout or from a working copy of it. Inside a workspace's working copies the guard treats that `factory/` folder, and `.claude/`, as the repository's own content (a worker may change them within its unit's `files`); only `.git` is off limits there. The workspace's own `factory/` stays protected.
- A **unit** names the repository it changes with `repo = "<name>"` (the only repository is the default; with several, `bf plan check` refuses a unit that names none or an unknown one). Its `files` are relative to that repository, and two units in one wave clash only when they touch the same path of the same repository.
- A unit's working copy is a git worktree of its repository, at `.bf/work/<U>` in the workspace, on branch `bf/<U>`. **Checkpoint branches live in each repository the checkpoint's units change**: a checkpoint over one repository has its working copy at `.bf/work/<K>`; over several it is a folder `.bf/work/<K>/` with one working copy per repository in `.bf/work/<K>/<repo>/`, and its checks run from that folder. Landing merges inside the unit's repository. Closing a checkpoint closes it in each repository, and a merge (with the human's go-ahead, as ever) merges the checkpoint branch onto the main branch of each, whose main checkout must be on it.
- The content a receipt is bound to is the content of the unit's or checkpoint's working copy (for several repositories, a hash of the working copies' hashes by repository name). In a workspace nothing is left out of the hash; in a project folder `factory/` and `.bf/` are.
- The **main-branch note is kept per repository**: the log head records, for each repository, the branch watched and the commit the engine last put there or the human accepted; `bf status` lists the repositories (`repos`) with where each stands, and its `main` is all of them together. `bf head accept` takes every repository as it stands, under the branch names the files give.
- Code repositories get **nothing written into them** except branches, worktrees and the commits workers and landing make: no `factory/`, no harness, no ignore entries. The check sandbox also denies a check writing to their `.git/hooks` and `.git/config`.
- The guard's root is the workspace: `factory/`, the harness files and the guard table are protected there as before. A worker may write only inside its own working copy, within its unit's files; a write by a worker into a code repository's own checkout is denied. Nobody writes git hooks or git config of a code repository through the Write tool; the human edits the checkouts freely.
- Not covered: records that are not in git at all, and one unit changing two repositories.

## The run log

One JSON object per line. Each event has `seq`, `at`, `kind`, its data, `prev` (the hash of the event before) and `h` (its own hash). `h` is an HMAC-SHA256 under the project's key, which lives outside the project tree: a line written by hand, with a correct `seq` and `prev` and a plain hash, does not verify. Editing or deleting a past line breaks the chain. A line cut short by a killed process is ignored and removed by the next write.

**The head.** The state folder also holds the log's head: how many events there are and the hash of the last, written under the same lock as every append. Every command that reads or changes state compares the log with it. A log that is **shorter** than the head, or whose event at that place **differs**, was rolled back (an old copy put back, `git stash`, a checkout, a restore) and is refused with the reason and a recipe that never says "restore from git". A log that is longer is accepted when the extra lines verify (a command was killed between the append and the head).

**Recovery: `bf head accept --quote "<the human's words>"`.** Only the human decides to keep a log that does not verify or does not match the head. After they say so in chat, this command chains the whole log again under the current key, sets the head to its end and appends a `head.accept` event naming what was wrong. It needs an unused human turn and a quote, like an approval. It also accepts where the main branch stands now (see `bf checkpoint close`), and it is the way to take a cloned project under a new key (old receipts must then be run again). `bf doctor` and `bf log --verify` still work on a damaged log.

## The policy

The policy in force is the copy the human accepted, kept in the state folder with its hash in the log head; the file `factory/policy.toml` is where the human edits. `bf init` accepts the policy that is there the first time. After that, **every command refuses while the file on disk says something different from the accepted copy** (a comment, blank line or reordering that leaves the parsed policy equal is not a difference), with the reason and a recipe: the human reads the change with `bf policy diff` (lines removed and added), and if they agree they say so in chat and `bf policy accept --quote "<their words>"` takes it (a decision like an approval, logged as `policy.accept`). Running `bf init` again does not accept a changed policy. The guard table, the role-token setting, the approval words, the check PATH, the transcript root and every other setting are read from the accepted copy only, so an edit of the file switches nothing on or off by itself. `bf doctor` and `bf log` still run while the policy is unaccepted.

## Common behaviour

- Exit codes: `0` done, `2` refused, `1` error.
- `--json` prints data as JSON. A refusal under `--json` is printed to stdout as `{"ok": false, "refused", "because", "do_this", "it_worked_when"}`.
- Every refusal has four parts, printed in this order on stderr: `refused:` what was refused, `because:` why, `do this:` numbered steps, `it worked when:` how to tell.
- `-C <dir>` runs the command as if started in `<dir>`.
- From inside a unit's working copy, `bf` finds the project's main checkout by itself. State always lives in the main checkout.
- Every command that changes state rewrites `.bf/guard.json`.

## Commands

### `bf init [--name N] [--workspace]`

Creates `factory/`, `.bf/`, a default policy and config, the sealing key, and the guard table, and adds `.bf/` to `.gitignore`. `--workspace` makes a workspace (see above): it needs no git repository to start from (it creates one) and adds `factory/workspace.toml`, which `bf repo add` fills. Safe to run again: it only refreshes the key, the guard table and the ignore line.

```bash
bf init --name myproject
```

Refuses when: the folder is not the top of a git repository (not for `--workspace`).

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

A **handoff is a row too**: it needs a proving check, and evidence, which is a passing receipt of that check at the content its unit landed on (a unit check) or its checkpoint closed on (a checkpoint check).

`impact` prints everything linked to the IDs: units that deliver them and units that depend on those, handoffs, checks, checkpoints and receipts.

```bash
bf ledger check
bf ledger impact R-1 C-2 --json
```

Refuses when: any designed part lacks a link.

### `bf plan check`

Checks the delivery plan: every handoff is proved by some check (a check's `proves` may name a handoff `H-n`; so may a checkpoint's), and at least one such check really runs, that is, belongs to a unit (`unit = "U-n"`) or is listed in a checkpoint's `checks`; no cycle in the unit graph; every handoff has a sender, a receiver (which depends on the sender) and a named thing; every checkpoint names the IDs it proves and a check for each; every designed ID belongs to a unit and to a checkpoint; every unit has a check; units in one wave share no files. Waves are the levels of the dependency graph.

Refuses when: any of that fails (all problems are listed).

### `bf run start` / `bf run stop` / `bf status` / `bf log`

`run start` begins a run (it needs a plan that passes `plan check`). `run stop` ends it. `status` prints the project, the run, test mode, every unit's state (`planned`, `ready`, `running`, `review`, `fixing`, `reviewed`, `blocked`, `parked`, `landed`) and every gate. `log` prints events: `--tail N` for the last N, `--verify` to check the hash chain.

`status` also prints `main`: whether the main branch is where the engine left it (see `bf checkpoint close`), with one quiet line when a person committed on it, and `bad_records`: record files that cannot be read, each named with the reason. One unreadable record file never stops `status`; `bf plan check` and `bf doctor` report it as a problem, and a command that asks for that record by id names the file.

Refuses when: a run is already active; the plan has problems; the log chain is broken.

### `bf next`

The next directives as data: for each, `role` (`builder`, `reviewer`, `verifier`, `engine`, `human`), `action`, the `unit` or `checkpoint`, `tier` (`cheap`, `standard` or `strong`, from the policy), `working_copy`, `brief` (the path `bf brief` will write), `limits` and the `commands` to run. Engine steps have `role: "engine"` (`land`, `close-checkpoint`) and a single `command`; worker directives carry a list of `commands` and the verifier's directive for a checkpoint has `brief` and `limits` like the others; human steps have `role: "human"` (`open-gate`, `decide-gate`, `decide-unit`). Before the plan gate is approved the only directive is the design gate to deal with: **the first design gate, in order (idea, requirements, ui, architecture, plan), that is neither approved nor skipped**. If it is open, the directive is for the human to decide it (`decide-gate`, with the gate ID). If it is not open and would open, the directive is `open-gate` with its command. If opening would be refused for want of records, the directive is for the conductor (`role: "conductor"`, `action: "add-records"`, with `missing` saying what: the idea file, a requirement, a screen or journey, a component and a decision, a plan that passes the check); for the ui gate it also carries `skip_command`, because a product with no interface skips the gate instead (`bf gate skip ui`). A directive never carries a command the engine would refuse, and a later gate is never offered while an earlier one waits. (With `[gates] in_order = false` the plan gate is offered as before.) `waiting` says why other units are not ready.

It also says plainly whether anything can start: `can_start` (a worker or engine step exists), `human_needed`, and `idle_reason`, which is null when something can start and otherwise says why not: `run done: ...`, `units parked or blocked: U-3 (parked); the human decides`, `waiting on a human step`, or `nothing is ready: ...`.

```bash
bf next --json
```

`bf next` also runs the idle check (see `bf stuck`) before it answers, so nobody has to call it. Questions that workers put to the human appear as `human` directives (`answer-question`) and hold only their own unit (see `bf ask`).

**When the human's messages are not being recorded**, the factory says so early, as information and never as a refusal: when the log holds events of a conductor at work and not one `human.turn` or `human.turn.rejected` since the project started, `bf status` prints a `note:` line (JSON: `human_turns_note`), `bf doctor` has a `human turns` line, `bf ask` and `bf next` print the same line on stderr (`bf next --json` carries `human_turns_note` too), and a gate decision with no human turn points at the same cause first. The usual cause is that Claude Code was started in a folder other than the one that holds `factory/` and `.claude/settings.json` (for a workspace, the workspace folder itself), so the hook never loaded.

Refuses when: no run is active; the budget cap is reached.

### `bf brief <unit-or-checkpoint> --role <builder|reviewer|verifier>`

Given a checkpoint ID (`K-1`), it writes the verifier's brief for the checkpoint: what it proves with acceptance, its checks with the exact commands, no source scope; other roles are refused for a checkpoint. Given a unit, it writes the exact context for that worker to `.bf/briefs/<unit>-<role>.md` and prints the path: the unit, the design parts it delivers with their acceptance, the contracts it receives and hands over, its checks with the exact command, its file scope, open findings, limits and rules. A reviewer's brief also lists the unit's receipts. A verifier's brief leaves out the file scope and tells it not to read the source.

### `bf report <unit> --result <built|findings|fixed|blocked> --token T [--role R] [--note T] [--findings f] [--cost N]`

The only way a worker changes a unit's state.

**Role tokens.** With `[roles] tokens = true` (the default), `bf report` needs the token of the directive it reports on: the builder's token for `built`, `fixed` and `blocked`, the reviewer's for the review. The token only says which role is reporting. `bf next` shows the current token in every builder and reviewer directive (`token`), **every time it is asked**, until a report spends it: a conductor that resumes, or a new one, always has the token of the work already handed out, and nothing needs the human to reset it. Tokens of directives that no longer exist are dropped. A builder cannot report as the reviewer: the reviewer's directive exists only while the unit is in review, and `bf next` and `bf brief` refuse to run from inside a working copy. **Cost.** With a budget cap, every report must carry `--cost <number>` (the conductor reads the session's real usage); text that is not a number is refused, never counted as zero.

| Unit is | Report | Becomes |
|---|---|---|
| `running` | `built` | `review` (or `reviewed` if the policy does not require review) |
| `review` | `findings --role reviewer [--findings f.json]` | `fixing` while a finding is open (new ones from the file, or filed with `bf record put finding`); `parked` after more than `max_rounds` rounds; `reviewed` when there is no finding at all |
| `review` | `built --role reviewer` | `reviewed` (refused while findings are open) |
| `fixing` | `fixed` (or `built`) | `review` |
| `running`, `fixing` | `blocked` | `blocked` (counts as an attempt) |

`--findings -` reads the list from standard input, so a reviewer needs no file. A findings file is a JSON list such as `[{"title": "...", "severity": "high", "where": "src/a.rs:10", "action": "..."}]`; each entry becomes a finding record with an ID that never changes. `--cost` adds to the budget count.

A finding is open until its `outcome` is `fixed` or `accepted`. A reviewer can file one from the unit's working copy with `bf record put finding --file f.toml`, but only a new, open finding for a unit that is in review (workers may put nothing else, and may not mark a finding fixed or accepted); the human sets `outcome = "accepted"` by putting it from the main checkout.

Refuses when: no run is active; the report does not fit the unit's state; a unit in review is reported on without `--role reviewer`; a clean review is reported while findings are open.

### `bf unit start <U>` / `bf unit land <U>` / `bf unit release <U> --quote "..."`

`start` creates the unit's working copy (on a unit that is already in flight it only makes sure the working copy exists) at `.bf/work/<U>` on branch `bf/<U>`, cut from the local branch of its checkpoint (`bf/<K>`). **A checkpoint's branch is cut from the branch of the checkpoint just before it in plan order** (the first from the main branch), so each checkpoint holds everything before it and the last one holds the whole result; the first unit of a checkpoint therefore waits until the earlier checkpoint is closed (`bf next` says `builds on K-1, which is not closed yet`). If work lands in an earlier checkpoint after a later branch was cut (an amendment re-opened it), it is merged into the later branch before more lands there. It needs an active run, an approved plan gate, and the unit's dependencies and handoffs satisfied. Running it again resumes the same working copy.

`land` merges the unit's branch into its checkpoint branch (no network). It re-checks first: the unit is `reviewed`; no finding is open; every check of the unit has a sealed passing receipt for the content now in the working copy; every changed path is inside the unit's file scope. Pending changes are committed in the unit's working copy first.

`release` is the human's way back for a unit that is `parked` (after `max_rounds` review rounds, `max_attempts` failing check runs or blocked reports in a row, or an idle limit) or `blocked`. It needs the human's words as a quote, like an approval. The unit goes back to work (`fixing` if it was parked after a review round, otherwise where it was), its round and attempt counts start again, and the log keeps the history. `bf next` then offers it again.

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

The command runs through `/bin/sh -c` with an environment of its own: nothing is taken from the caller except a short list (`HOME`, `USER`, `LANG`, `TMPDIR`, `CARGO_HOME`, ...), and `PATH` is the one in the policy's `[exec] path` (captured at `bf init`, editable by the human only). A fake program first on the caller's `PATH` cannot stand in for the real one. Every git call the engine makes also runs without the caller's `GIT_*` variables, with the policy's `git` program and no user or system git config, so `GIT_DIR` or `GIT_WORK_TREE` cannot point the content hash at another repository. **On macOS every check, and every git call, runs under `/usr/bin/sandbox-exec`** with a profile that denies the network, reading or writing the project's state folder, writing under `factory/` and under the main checkout's `.bf/` outside the working copy in use. The engine tests at start whether `sandbox-exec` works; where it does not (other systems, or a refusal) the check runs **without** a sandbox and the receipt says `sandbox = "none"`, as does `bf doctor`. `[exec] sandbox = false` in the accepted policy switches it off. On the test machine the sandbox adds about 80 ms to one `bf evidence run`. `[exec] network = true` in the accepted policy lets checks reach the network under the sandbox, and `[exec] env = ["JAVA_HOME"]` names more environment variables a check may see. The profile also denies a check writing to `.git/hooks`, `.git/config`, `.claude/` and the transcripts, because git or the harness would run what is planted there later, outside the sandbox. **The folders on the check PATH are not checked for change**: a version manager rehashing its shims or an install changing a folder stops nothing. `bf doctor` lists, for information, the PATH folders that lie inside the project or in the user's home folder. **The program a check starts is recorded when the plan gate is approved**: for each command check, where its first word is found on the check PATH (the resolved path) and a hash of that file, kept in the state folder. Before the check runs the program is looked up again; if it is found somewhere else, or the file's contents differ, the check still runs, `bf` prints a note, and the receipt carries a `program_note`; the new state becomes the recorded one, so a change is noted once. A check added after the plan gate is recorded the first time it runs. Only the first word of the command is covered, not a program that program starts. The receipt records the program (`/bin/sh`), whether it ran in the sandbox, and the hash of the check record it was made for; if the check record changes later, the receipt stops counting.

The command must equal the `command` stored in the check record. Otherwise an agent could run `true` under the name of a real check. The exit code of `bf` is 1 when the command failed (the failing receipt is still stored). Nobody writes a receipt by hand: a receipt without a valid seal is ignored everywhere.

Refuses when: no command is given; the ID is not a check, or the check is not a command check; the command differs from the check's own; the command changed the working copy while it ran (the receipt would describe neither state).

### `bf evidence attach <check> <file> [--outcome met|failed|unreachable --says "..." --token T]` / `bf evidence verify [--unit U] [--checkpoint K]`

`attach` stores a file by hash under `factory/evidence/` and writes a sealed receipt bound to the content. **A check judged by an agent is settled by a stated outcome, not by a file.** `--outcome met|failed|unreachable` with `--says "<one sentence: what was looked at>"` records the judgment; it is accepted only with the verifier's role token (`--token`, the `token` on the verifier's directive from `bf next`; for a check that belongs to one unit, the reviewer's token for that unit). A builder holds neither, so it cannot settle a check for its own unit; with `[roles] tokens = false` no token is asked. `met` is a passing receipt; `failed` and `unreachable` are the marks `bf evidence fail|unreachable` make (the checkpoint waits for the human, and a later `met` at the same content replaces them; while a mark stands, the `decide-check` directive carries `verifier_token` so the verifier can look again). A file attached **without** an outcome is kept as supporting material and settles nothing. The outcome, the sentence and the file are shown on the checkpoint page and the coverage page. A `human` check is settled only by the checkpoint's approved `checkpoint-step` gate, never by a file; a `command` check only by running its command. The size limit is in the policy. The path is resolved when the file is read, links included, and a path that leads into the project's state folder is refused. The same holds for every file the engine reads for a caller (`--file`, `--findings`, `--template`).

`verify` checks the receipts: the seal and the exit code of each, and, with `--unit` or `--checkpoint`, that every check in scope has a sealed passing receipt at the content now in that working copy. Older receipts for other content are listed as `OTHER` and do not fail the command when a current one exists. It exits 2 when anything fails.

### `bf evidence fail|unreachable <check> --reason "..."`

The verifier's way to say that a check ran and does not match (`fail`), or could not be run (`unreachable`), when there is no passing receipt to show. It writes a sealed record bound to the content, in the working copy where it is run. It is never proof. A checkpoint whose check carries such a mark does not close (the refusal names the check and the reason); `bf next` shows a human directive `decide-check` (so `can_start` is false and `idle_reason` says a check failed or is unreachable); the checkpoint view marks the row and sets `needs_you`. A later passing receipt of the same check at the same content replaces the mark, so once the cause is fixed the verifier just runs the check again.

### `bf checkpoint close <K> [--merge]`

Closes a checkpoint (the records must match what the gates approved). All its units must have landed, its working copy must be clean, and every ID in its `proves` list needs a sealed passing receipt, from one of its listed checks, at the content now in the checkpoint's working copy, or an ID waived by an approved amendment. A checkpoint with a `human_step` also needs an approved `checkpoint-step` gate for it.

`--merge` also merges the checkpoint branch onto the main branch (the main checkout must be on it). It needs a human act **after the closure**: the checkpoint must be closed, and an approved `checkpoint-step` gate for it (real, not simulated) must have been opened after the close event (`bf gate open checkpoint-step --ref K`, the human says merge in chat, `bf gate approve <gate> --quote ...`). Merging twice does nothing the second time. A test run refuses it before anything else. After a merge the engine remembers the main commit it made. **The main branch is compared with the commit recorded in the log head**, and the branch watched is the name recorded there too (so an edit of `factory/config.toml` cannot move the watch; `bf doctor` shows the difference, and `bf head accept` takes the config's name). A watched branch that no longer exists is a problem, not "ok".

The recorded commit is set at `bf init`, after an engine merge, and when the human accepts. Ordinary commits by a person, including the first commits of a new project, are not a problem: `bf status` shows one quiet line (`the main branch has moved on ... by commits the engine did not make (3 new); they hold no unlanded or test-run work`) and nothing is refused. The watch complains only about **work that should not be on main**: the commits of a unit that never landed, or that landed in a test run, found on the main branch. Then `bf status` says so (`main.ok` false, with the commits' files), and `bf unit land`, `bf checkpoint close` and `bf ledger check` refuse until the human moves the branch back and, if the position changed, accepts it with `bf head accept --quote "<their words>"`. A run that contains a simulated approval, or a unit landed in a test run, can never be merged.

```bash
cd .bf/work/K-1
bf evidence run X-3 -- cargo test --all
cd ../..
bf checkpoint close K-1
```

Refuses when: units have not landed; the working copy has uncommitted changes; an ID has no proof; a human step is not approved; with `--merge`, the run is tainted by simulated approvals or the main checkout is on another branch.

### `bf checkpoint sync <K> --quote "<the human's words>" [--resolved]`

The run builds on a stack of checkpoint branches cut from the main branch as it stood. When the human merges other work into the repository's **local** main branch meanwhile, the stack and the main branch meet only at the final merge, where they can conflict and where the receipts prove content from before the join. `bf status` therefore says when the main branch holds commits the stack lacks (`main_ahead` in `--json`: repository, checkpoint, commits, the command; plain status prints a `note:` line) and `bf next --json` carries the same as information, never as a directive. `sync` is the supported step: a human act (the same rule as an approval: an unused human turn after the request, the quote in it) that merges the repository's local main branch into checkpoint branch `K`, in each repository of a workspace that `K` touches and whose main branch has commits `K` lacks. The engine makes no network call: pulling from a remote into the local main branch is an ordinary git command done before.

It is allowed only at a quiet point: `K` is open and at the tip of the stack (no later checkpoint has started), and no unit is between start and landing (running, in review, fixing, reviewed, blocked or parked); otherwise it refuses and names the units to finish or release first. Nothing to take in is not an error: it says so and changes nothing. A merge conflict is aborted (earlier repositories of the same call are put back): the refusal lists the conflicting files and nothing is changed. The simplest honest way through is the human's: in the checkpoint's working copy (`.bf/work/<K>`, or `.bf/work/<K>/<repo>`) run `git merge <main branch>`, resolve, `git add`, `git commit`, then `bf checkpoint sync <K> --resolved --quote "..."`, which checks that the branch now holds the main branch (and that its tip is a merge) and records it; `--resolved` also records a merge the human made by hand in the first place. A `checkpoint.sync` event (checkpoint, repository, the main branch's commit, the merge commit) is appended; units that start afterwards start from the new tip; the receipts of `K`'s own checkpoint checks that exist then stop counting (the content changed) and the checks are open again; the receipts of landed units still count for their units; the recorded main commit of that repository moves to the main branch's tip, so the quiet main-branch note goes away, and the final `--merge` onto the main branch has nothing foreign between.

**A hand merge without the command.** Merging the main branch into the checkpoint branch by hand in `.bf/work/<K>` is not refused by the engine (an agent's `git merge` there is refused by the guard, so it is the human's step). Nothing is logged; the checkpoint's own receipts are for the old content and stop counting; the unit receipts stay; the main-branch note stays until something records the position. `bf checkpoint sync <K> --resolved --quote ...` after the hand merge records it.

### `bf human-turn`

For the harness's `UserPromptSubmit` hook. Reads the hook payload on stdin and, when it is a message the human typed, appends a `human.turn` event with the text. It prints nothing (a hook's output goes to the model). It always exits 0 on a recorded or rejected message, and 1 on an error; it never exits 2, because that would erase the human's prompt.

A payload is **not** a human turn when: it does not say `hook_event_name: UserPromptSubmit`; it has no `session_id`; it carries `agent_id`, `agent_type`, `parent_tool_use_id`, `subagent` or `agent_name`; its `source` is set and is not the user; the prompt is empty; the prompt is the host's own message and not the human's (below); its `cwd` is outside the project or inside a unit's working copy; or the policy lists the human's session ids (`[human] sessions`) and this one is not among them. Rejected payloads are logged as `human.turn.rejected` with the reason (it names the form that matched) and never satisfy anything.

**What is the host's own message.** After removing any leading `<system-reminder>...</system-reminder>` blocks and surrounding whitespace, a prompt is the host's when the WHOLE of it is one of: a `<task-notification>` element (a background task finished); an `<agent-message from="...">` element (a subagent's hand-back or another agent's message); a `<cross-session-message ...>` element (another session's message); a `<side-sessions-event>` element; a `<ci-monitor-event>` element; the sentence `The app was quit while you were working. Please continue from where you left off.`; or text that starts with `Background (from the session that started this one):` or `Task (from the session that started this one):` and a newline (the brief a session is started with). An element must start the prompt and its first closing tag must end it (`</name>`, with `<~/name>` or `<\~/name>` also read as closing the two message forms, and a `~` or `\~` after the `<` of the opening tag ignored), so a message of the human that contains or quotes such a block inside other words is still theirs. A prompt made only of reminders is the host's too. Prompts that start with a `<local-command-stdout>`, `<local-command-caveat>`, `<user-prompt-submit-hook>`, `<teammate-message>` or `<channel>` wrapper are not the human's either. The human's pasted text (`<pasted_content`) and a message sent while a turn is running arrive on their own and count. **The transcript row, when it can be found** (the payload's prompt id, and exactly this text), gives a second signal by its own marks: host when `promptSource` is `system`, `turnOrigin` is `task_notification` or `peer`, or `origin.kind` is `task-notification`, `coordinator`, `peer` or `auto-continuation`; human when `origin.kind` or `turnOrigin` is `human` or `promptSource` is `typed`. A host mark rejects whatever the words, a human mark accepts whatever the words; a row with neither mark (the desktop app marks its own notes and the human's alike as `sdk`) decides nothing and the words decide. The CLI writes the row after the hook returns, so often there is none yet: its absence decides nothing.

**The transcript is information, not a gate.** The turn is recorded from the payload as given. The engine also looks at the transcript file the payload names and stores what it saw with the turn (`transcript`): `match` when the last human message in it is this prompt; `differs` when it is another one; `unparsed` when no human message could be read; `absent` when the payload names no transcript, the file does not exist, or it is not under `[human] transcript_root` (default `~/.claude/projects`). Claude Code writes the transcript after the hook runs, so for the first prompt of a session the file is absent and for a later one it ends with the previous prompt: `differs` and `absent` are the normal values. Lines of type `api-request-blob`, meta lines and tool results are not counted as the human's message. `bf gate status` and the gate page show the value beside each decision. A policy that sets `[human] transcript_match = "strict"` refuses a turn unless the transcript already ends with the prompt (for a harness that writes it first).

The engine cannot tell a person from a program that writes to the hook's input. The hook is the source of human turns, the checks above remove the obvious fakes, and the quote on every decision (below) makes it visible which words were taken. See "What the engine does not stop".

### `bf gate open <kind> [--ref ID]` / `approve` / `reject` / `skip ui` / `status`

Kinds: `idea`, `requirements`, `ui`, `architecture`, `plan`, `checkpoint-step` (needs `--ref K-<n>`). A gate has rounds; its ID is `<kind>-r<round>` (or `<kind>-<ref>-r<round>`). Opening records a snapshot of the records the page shows, so the next round can mark what is new, changed or removed.

**Design gates open in order and never empty** (`[gates] in_order`, on by default; the same in a granted test run, with simulated approvals). The order is idea, requirements, ui, architecture, plan: `open` refuses, with the usual four parts, while an earlier gate is not approved (a rejected gate's next round is not held by this), and when the stage has nothing to approve: idea needs an idea record or `factory/idea.md`; requirements at least one requirement; ui at least one screen or journey; architecture at least one component and one decision; plan a plan that passes `bf plan check` with at least one unit and one checkpoint. A product with no interface has no screen to invent: `bf gate skip ui --quote "<the human's words>"` (a decision like an approval, once) records the skip in the log; a skipped ui gate counts as passed for the order, and `bf status` (`skipped_gates`), the index page and the architecture page say so. An architecture gate with no threat record opens, and its page carries a visible caution that no threat was recorded.

`approve <gate> --quote "<words>"` and `reject <gate> --quote "<words>"` need a human turn recorded **after** the gate opened and not yet used for anything else. One typed message decides one thing. **The quote** is the words of the human's message that the conductor read as the decision. The conductor's reading of the human's words is intended; the engine does not second-guess it. It checks only what it can:

- the quote is in the human's message (and is not empty);
- an approval (`approve`, `test-mode grant`, `amend approve`, `policy accept`, `head accept`, `unit release`) is not a question: the quote does not end with `?`, and the message is not only questions;
- a test-mode grant also speaks of a test run.

The quote is stored with the decision, and so is the human's **whole** message. `bf gate status`, `bf gate approve --json` and the gate page ("Decided on the words", "The human's whole message") show both. When the message also holds a word that can point the other way (the policy's `[human] refusal_words` for an approval, `approval_words` for a rejection: *not*, *wait*, *change*, *later*, *cannot* and so on, in English, German, French and Spanish), the gate status and the page show a **caution** ("the message also contains a word that can mean no: \"wait\""). The lists never refuse anything. `approve --simulated` is described next. A bare kind (`plan`) means its latest round.

`bf gate open` prints the gate ID. `bf gate reject` closes the round; the next `bf gate open <kind>` starts round n+1.

```bash
bf gate open architecture
# the human says "I approve the architecture" in chat; the hook records it
bf gate approve architecture --quote "I approve the architecture"
```

Refuses when: no quote is given, or it is not in the message; an approval is a question; the kind is unknown; a gate of that kind is still open; unanswered questions are tagged for this stage (see `bf questions`); for `plan`, the plan has problems; no human turn follows the open; the gate is already decided.

### `bf test-mode grant`

A grant for one run, made from the human's latest message (it must be unused). If a run is active it applies to that run; otherwise to the next `bf run start`. A grant that is waiting for its run also lets the five design gates (`idea`, `requirements`, `ui`, `architecture`, `plan`) be approved with `--simulated`, so a test run can go through them in order before `bf run start`; the checkpoint-step gate needs the run. Inside a granted run, `bf gate approve <gate> --simulated` satisfies a gate and is recorded as simulated. Outside one it satisfies nothing: the command refuses, and an approval event written into the log by other means is ignored when the state is replayed.

Needs `--quote` like an approval. Refuses when: the human's latest message is missing, already used, only a question, or does not contain the quote.

### `bf guard`

The per-tool-call hook. See the section on the guard below.

### `bf view <kind> [id] [--json]` / `bf render <kind> [id] --template <file> --out <file>`

`view` prints the page data in the view contract (version 1, `contract: 1`). Kinds: `index`, `gate` (id is a gate ID or kind; a kind never opened gives a draft page), `questions` (id is a stage), `ledger`, `plan`, `run`, `checkpoint` (id is a checkpoint), `coverage`. The output is always JSON.

The `index` page is a list of pages: sections of rows, each with `href` and `meta`. The links follow one naming convention, so a conductor that renders each page under that name gets working links: `gate-<gate id>.html`, `questions.html`, `plan.html`, `run.html`, `ledger.html`, `coverage.html`, `checkpoint-<K>.html`.

`render` also sets the page title: if the template's title is `<title>Black Factory</title>` it becomes `<page title> · Black Factory`, and a `/*BF_TITLE*/` marker, if the template has one, becomes the page title.

`--out` is resolved when the file is written, links included: a path that leads into the state folder, `factory/`, `.git/`, the transcripts or the private part of `.bf/` (anything but `pages/`, `drafts/` and `work/`) is refused. `render` puts the JSON into an HTML template at the marker `/*BF_DATA*/` and writes one file. If a literal placeholder (`null`, `undefined`, `{}`, `[]`) follows the marker it is replaced too, so `const DATA = /*BF_DATA*/ null;` works. The JSON is made safe for a script element (`</` and line separators are escaped).

```bash
bf render gate architecture --template views/templates/page.html --out out/architecture.html
```

Refuses when: the template cannot be read or has no marker; the page kind is unknown; no gate matches.

### `bf ask --file q.json [--stage S]` / `bf ask --title T [--unit U]` / `bf answer <Q> --file a.json` / `bf questions [--open]`

A stage's questions come before its gate. `ask` takes a JSON list of questions in the view contract's shape without IDs: `[{"title", "kind": "single|multi|text", "context", "stage", "options": [{"key", "text", "proposed", "why"}]}]` (a table with a `questions` list, or one question, also works). The engine assigns `Q-` IDs and prints them; a bad entry writes nothing. `--stage` fills the stage of entries that have none. `answer` needs a human turn recorded after the question was asked (the turn is stored as `answered_turn`), and takes one file per question, for example `{"choice": "A", "note": "..."}`, and stores it as the answer (or the file's `answer` value if it has one). `questions` lists them.

A question is **tagged**: a stage question (`--stage`, or `stage` in the file) holds that stage's gate; a **unit question** holds only its unit. A builder, reviewer or verifier asks from its working copy with `bf ask --title "..." [--context "..."] [--options "A=first;B=second"]` (no file needed; `--file -` or a file also work) and is tagged with its own unit or checkpoint; the conductor can tag one with `--unit U-<n>`. `bf next` shows each open unit question as a human directive (`answer-question`) and offers no directive for that unit until the human answers; other units go on. The answer reaches the worker in its next brief (`## Questions you asked the human`). A question with neither a stage nor a unit is refused, with the recipe, because nothing would wait for it.

**Every choice question takes an Other answer.** `other` (lowercase) is a reserved choice key: `bf ask` refuses a question whose own option uses it, and a question may carry `"other": false` to switch Other off (stored on the record; text questions have none). `bf answer <Q> --file a.json` accepts `{"choice": "other", "other": "<the human's own answer>", "note": "..."}` for a `single` question and `{"choices": ["A", "other"], "other": "<text>"}` for a `multi` one; choosing `other` with empty or missing text is refused, choosing it on a question with Other switched off is refused, and `other` text without the key chosen is kept as given and does not count as the choice. The text is stored in the answer's `other` field and is what every listing shows: `bf questions`, the briefs that quote answers and the questions view (whose `answer` object carries `other`, and whose question carries `other: true|false` for choice questions) show the human's text, never the bare word "other". Earlier records, with no `other`, read as they did.

### `bf amend open <ID>... ` / `narrow --file f` / `approve [A-n]`

`open` computes the impact set from the ledger. `narrow` takes `{"keep": [...], "waive": [...], "reasons": {"<ID>": "why"}}`: it may only remove IDs from the set, every removed ID needs a reason, and every waived ID needs a reason. `approve` needs a human turn after the narrowing. An approved amendment lets its IDs be replaced with `bf record put` after their gate was approved, and its `waive` list stands in for a receipt at `checkpoint close`.

**An approved amendment re-opens what it touches.** Every unit in the kept impact set whose work is not all waived goes back to work (`running`; what it landed stays in the log; its rounds and attempts start again; its brief and directive name the amendment), every checkpoint that proves a changed ID or comes after a re-opened unit opens again, and the receipts of the changed checks that exist at the approval stop counting (newer ones count). The ledger therefore reads open until the new evidence exists. Units and checkpoints outside the set keep their state and approvals, and `bf next` offers exactly the re-opened work.

### `bf cost status` and the policy

`bf cost status` prints spent, cap, tiers and per-unit cost. `--cost` on `bf report` adds spend. When `[budget] cap` in `factory/policy.toml` is above 0 and spend reaches it, `bf next` and `bf unit start` refuse.

### `bf page set <name> <url>` / `bf page list [--json]`

A page the conductor publishes (the questions form, a gate page, the plan) is found again by a new session through the log. `page set` records where `<name>` was published; the name is the page's name without `.html` (`questions`, `gate-<gate id>`, `plan`, `run`, `index`, ...). It appends a `page.published` event with `name` and `url`; setting a name again replaces the earlier link. Only `https://` links are accepted. `page list` prints the latest link per name; `bf status --json` carries the same as `pages: [{name, url, at}]` and plain `bf status` prints one `page <name>: <url>` line each. A `bf next` directive that waits on the human for a gate (`decide-gate`) or for a worker's question (`answer-question`) includes `page_url` when a link for `gate-<gate id>` or `questions` is recorded. Links are private: they live in the workspace's log and nowhere else, and the engine writes them into no code repository, no commit and no rendered page.

### `bf stuck [--dry-run]`

Parks every running unit with no event for `idle_minutes`, and lists all parked units. Units parked by review rounds are listed too. `bf next` runs the same check by itself.

### `bf doctor [--perf]`

`doctor` checks the key, the log chain, the guard table, the ignore line, ignored events and the plan. `--perf` times each class of command in the current project and prints medians and 99th percentiles, including `bf guard` as a real child process.

## The content hash

A receipt is bound to the content it ran on. The content hash is the **git tree-object hash of every file under the working copy's top folder that is tracked, staged, or untracked and not ignored, as the files are on disk when the command starts, leaving out the folders `factory/` and `.bf/`**. It is computed in a temporary copy of the index (`git add -A`, `git rm --cached` for the two folders, `git write-tree`), so the real index and the history are untouched. Files that `.gitignore` covers do not count; add build output to `.gitignore`. The hash is taken before and after a command runs; if they differ, no receipt is written.

For a unit the working copy is `.bf/work/<U>`; for a checkpoint it is `.bf/work/<K>`; in the main checkout it is the checkout itself.

## The guard

`bf guard` is the hook that runs before every tool call. It reads the tool call on stdin and the project's guard table, `.bf/guard.json`, and checks the table's signature with the key in the state folder (the one extra small file it reads). It starts no process and calls no git; it resolves paths with `canonicalize`, so a symlink cannot hide a target. The table is numbered: every time the engine writes one it raises a counter kept next to the key, and the guard refuses a signed table whose number is below the counter, so an old copy put back loosens nothing.

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
  - **The engine is found only where a shell would run it.** The guard reads the command line the way a shell does (quotes, `$( )`, backticks, heredocs, `&&`, `||`, `;`, `|`, a newline) and treats `bf` as a call only in command position: the first word of a simple command, after leading `VAR=value` assignments and the wrappers `env`, `command`, `exec`, `time`, `nohup`, `xargs`, `sudo`, and the string argument of `sh -c`, `bash -c`, `zsh -c` and `eval`, read the same way. The word `bf` as an argument of another program (`cargo test -p bf`, `ls crates/bf`), in quotes that are data for another program (a `grep` or `sed` pattern, `echo` text), in a comment, or in the body of a heredoc that feeds a program that is not a shell is not a call, and the same goes for `human-turn` appearing as data. In `bf evidence run <check> ... -- <command>` the part after `--` is a command line of its own, read by the same rule. A real call is judged as before: **`bf` must be called plainly** (a subcommand that is not a literal word from the list, `bf $(echo human)-turn`, `bf ${x}`, `bf "$(printf ...)"`, is denied; so is a `bf` reached through `eval`, `sh -c`, `bash -c`, `env`, `xargs`, `command`, `exec`, a backtick or `$(`), `bf human-turn` is denied, `$B human-turn` with a program built at run time is denied, and so is an interpreter (`python3 -c`, `perl -e`, `node -e`, `sh -c`) or `cargo` given an argument that spells `bf human-turn`. Where the reading is unsure about a word that is not in command position, the word passes
  - any command that names the state folder, the variables that locate it (`BF_STATE_HOME`, `XDG_STATE_HOME`) or its parent folder is denied;
  - any command that names `factory/`, `.bf/`, `.git/hooks`, `.git/config` or the hook settings, in any spelling that resolves there, is denied **unless it is a `bf` command or a read** (`cat`, `ls`, `head`, `jq`, `sed -n`, `awk`, `sort`, `grep`, `find` without `-delete` or `-exec`, an interpreter such as `python3 -c` or `node -e`, `git status|diff|log|show|add|stash list`, ...). Redirects to `/dev/null` and `2>&1` do not count; any other redirect does. So `sh -c "echo x > factory/policy.toml"`, `rm`, `cp`, `find -delete` and here-documents that write to those paths are denied, and `cat factory/records/R-1.toml 2>/dev/null` and `python3 -c "import tomllib; ..."` are not. An interpreter that writes without a redirect is left to the harness's own permission checks (see below);
  - writing under the harness's transcript root is denied (reading is not);
  - the git commands that can move the main branch or a checkpoint branch or lose records (`push`, `pull`, `update-ref`, `fetch` with a refspec, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `stash`, `clean`, `reset --hard`, switching to or committing on an engine branch, forced `branch`, `worktree`, `--git-dir`, `git config` of an alias, filter, `core.*` or include) are denied unless the **target** (`-C dir` or the working directory) is a unit's own working copy; a git subcommand the guard does not know is a user's own alias and is left alone (setting an alias up is denied above), and so is plumbing that only makes objects; ordinary `git commit`, `git add` and `git checkout <ref> -- <path>` are left to the user, and the main-branch watch (above) looks at what lands;
  - from a unit's working copy (a landed unit's old copy counts too) the only `bf` commands are `evidence run|verify|fail|unreachable|attach`, `report`, `record put finding`, `record get|list` and `status`; from a checkpoint's working copy the same without `report` and `record put`. `bf -C`, a `cd` out of the working copy, and a write whose target is elsewhere in the project are denied there.
- Every other tool is allowed (abstained).

Measured in this repository's tests, the guard takes about 2 ms per call as a separate process (design target: 30 ms median).

The engine also enforces the worker limits itself, without the hook: a command run from inside a working copy (by the process's real current directory, not by `-C`) is limited to the same list, and `bf next`, `bf brief`, gates, landing and planning are refused there.

## What the engine does not stop

The guard and the checks stop mistakes, drift and casual cheating, and make deliberate cheating visible. They do not make a determined agent honest. There are two cases.

**A worker with only the guarded tools (Write, Edit, Read, Grep, Glob, and Bash through the hook).** After the checks above it can no longer, in a few calls, record a human turn, approve a gate with the human's refusal, edit the policy or the check records unnoticed, add a unit to an approved plan, read the key by name, plant a program in a folder it was told about, or move the main branch without the engine saying so. What it can still do:

- **Hide a path.** The guard reads command text. A path built at run time (`'fac' + 'tory'` inside a script, a variable, a glob such as `fact*/`) or a script file that does the work (`sh src/x.sh`) is not seen. The sandbox around checks and git helps only where the engine starts the process; a worker's own Bash call is not sandboxed unless the harness does it.
- **Run its own code as a check.** A check runs the unit's code; that is the point of it. On macOS the engine runs it in a sandbox (no state folder, no network, no writes to `factory/` or other working copies); elsewhere it runs unsandboxed and the receipt says so. Either way the code can do what it likes inside its working copy and its temporary folder, and what a check proves rests on someone reading that code.
- **Plant a program where checks look.** The folders on the check PATH are not watched for change. A program replaced in place is noted in the receipt of the next run (see `bf evidence run`), only for the first word of a check's command.
- **Take the reviewer's token.** The token only says which role is reporting. A worker that runs `bf next` from a script before the conductor does can read the reviewer's token; the engine refuses `bf next` from a working copy, and the rest is left to Claude Code's own permission checks (below).
- **Move main, then be noticed.** The engine sees a move of the main branch the next time it reads, and refuses to land, close or call the ledger closed until the human accepts the position. It does not stop the move: only a host's branch protection does.

### What Claude Code's own permission checks are relied on for

The engine is there so that nothing is forgotten: every ID linked, every unit reviewed, every gate decided in order, every decision shown with the words behind it. It is not a box for an agent that means to hide something. Hiding takes a script or an interpreter, and that is the layer Claude Code's own permission prompts (and, under auto mode, its own checks of deceptive actions) see, because they see the call as it runs. These routes are therefore left to that layer, and the engine adds no text matching for them, because every such rule can refuse an ordinary command:

- a script the agent writes in a file it may write and then runs, reaching the state folder, the key or a transcript by a path built at run time;
- a Bash call that builds a protected path, a `bf` subcommand or an environment variable name from pieces (`'fac' + 'tory'`, `B=bf; $B ...`, `base64 -d | sh`);
- an interpreter (`python3 -c`, `node -e`, `perl -e`) that writes to `factory/` or the policy without a shell redirect;
- a worker that reaches `bf next` from a script before the conductor does and so is offered the reviewer's token; a builder that reviews itself this way;
- a worker that starts `sandbox-exec` with a permissive profile of its own, so that a check runs unsandboxed (the receipt says `sandbox = "none"`);
- a builder that reports `fixed` without changing anything (every open finding is then marked fixed), or attaches any file to an agent-run check;
- one human message used for a gate other than the one it was meant for, and the same payload piped to `bf human-turn` twice;
- the Write tool writing outside the project from a unit's working copy;
- a flaky check re-run until it passes; a check that depends on a file the content hash does not see;
- `bf init` run with another state folder, which re-signs the guard table with a new key.

Where a person wants more than that, the harness's own sandbox for worker sessions, with the state folder, `factory/`, `.bf/`, `.git/` and the transcripts denied, is the layer that does it (below).

**An agent with an unrestricted shell under the same user account.** It can read the state folder (`~/.local/state/blackfactory/<id>/`): with the key it can seal receipts, chain log lines, sign a guard table, rewrite the log and its head together, and hand itself role tokens. It can write anywhere the user can, including the policy and the transcripts. Nothing in this engine stops that. Real separation needs:

- **the harness's sandbox**, so that worker sessions cannot read or write the state folder, `factory/`, `.bf/`, the transcripts and other working copies at all (layer 1 of D-3; not built here); and
- **a host identity the agent does not hold**: a pull request on the host that only the human merges, with required checks that re-verify every receipt.

What each protection gives, and what stays open:

| Protection | What remains |
|---|---|
| Key and head outside the project tree | Same-account shell can read them; forging a line needs the key, undoing a rollback needs the head |
| Signed guard table with a counter, outermost project, `canonicalize` | A same-account agent can still write files by means the command text hides |
| Accepted policy in the state folder | A same-account agent can replace the copy and the head together |
| Role tokens | A script that asks for a token before the conductor does gets one; the conductor's second request voids it with a note; a leaked token is a role |
| The quote and the whole message stored with every decision, with a caution when a word points the other way | The engine does not understand the sentence; the conductor's reading is the decision, and the human can see it |
| Hook payload checks; the transcript noted beside each turn | The transcripts are the harness's own files and are not a gate; a program that can feed the hook can make a human turn |
| Clean environment, sandbox and PATH check | Not complete: see above |
| Main-branch watch | It notices after the fact, and only work that should not be there |
| Program hashes at the plan gate | A note in the receipt, not a refusal; only the first word of a check's command |
| Cost on every report | The number is still the conductor's word |
| Frozen records and receipt-to-check binding | A same-account agent could edit records and the log together |

## Git: the `git` program, not a library


The engine runs the `git` program for every git read and write. It does not use a Rust git library such as `gix`, and its dependencies contain none. It makes one to three git calls per command that needs git (the content hash is three calls; creating or merging a working copy is one to three), and none at all from `bf guard`, `bf human-turn`, `bf next` (before a checkpoint is ready), queries and state changes of records. The reasons: a library would add many crates and a long build for the few operations needed here, and the operations that matter (a tree hash of the files on disk including untracked ones, worktrees, merges) are the ones where the `git` program is the reference behaviour. The measured times of every class of command are in the test output (`cargo test --test timing -- --nocapture`).

## Testing aids

`BF_CRASH_AT=<point>` makes `bf` exit with code 137 at a named point, to test resuming a killed run. The points are `unit-start-after-git` and `land-after-merge`. It does nothing else.
