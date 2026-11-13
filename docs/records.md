# Records

Every fact the engine keeps is a file. Records are TOML files in `factory/records/`, one per file, named by their ID. They are validated by `bf record put` against the schema of their kind: all required fields present, field types right, no unknown fields, IDs of the right shape. This page lists every kind and every field. For the commands see [engine.md](engine.md).

## IDs

An ID is a prefix, a dash and a number: `R-1`, `TH-12`. The prefix says the kind.

| Prefix | Kind | Designed part? |
|---|---|---|
| `R` | requirement | yes |
| `C` | component | yes |
| `S` | screen | yes |
| `J` | journey | yes |
| `T` | target | yes |
| `TH` | threat | yes |
| `D` | decision | yes |
| `U` | unit | no |
| `H` | handoff | no |
| `K` | checkpoint | no |
| `X` | check | no |
| `F` | finding | no |
| `Q` | question | no |
| `I` | idea | no |

A designed part must end up with a unit that delivers it, a check that proves it, a checkpoint that names it, and a passing receipt. `bf ledger check` and `bf plan check` enforce that. Which kinds count as designed parts is set in `factory/policy.toml` under `[ledger] kinds`.

Every record has `id` (string) and `kind` (the kind's name). When `bf record put` is given a file without them it fills them in.

## Design parts

Requirement, component, screen, journey, target, threat and decision share these fields.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | One sentence that states the part, not just its topic |
| `text` | string | no | Detail, shown folded on pages |
| `label` | string | no | One of `critical`, `high`, `medium`, `low`, `core`, `main`, `minor` |
| `links` | list of designed IDs | no | Other parts this one relates to |

Extra fields by kind:

| Kind | Field | Type | Meaning |
|---|---|---|---|
| requirement | `acceptance` | list of strings | What must be true when it is done |
| target | `measure` | string | The number or limit it is held to |
| threat | `mitigation` | string | How the design answers it |
| decision | `chosen` | string | What was chosen |
| decision | `rejected` | string | What was rejected and why |

```toml
id = "R-1"
kind = "requirement"
title = "A visitor can read the greeting."
acceptance = ["src/hello.txt exists"]
```

## Idea (`I`)

The human's starting idea, as a record. The idea gate shows idea records and also the plain file `factory/idea.md` when it exists (the guard lets the conductor write that one file under `factory/`).

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | |
| `text` | string | no | |
| `links` | list of designed IDs | no | |

## Unit (`U`)

A slice of work that one builder does in its own working copy.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | What the unit does |
| `text` | string | no | More detail for the brief |
| `delivers` | list of designed IDs | yes, not empty | The parts this unit builds |
| `files` | list of strings | yes | File scope: paths or folders (`src/a.rs`, `src/ui/`, `src/*.rs`) the unit may change |
| `checkpoint` | `K-` ID | yes | The checkpoint the unit lands on |
| `depends_on` | list of `U-` IDs | no | Units that must land first |
| `estimate` | integer | no | Story points |
| `tier` | `cheap`, `standard` or `strong` | no | Model tier for the builder, overriding the policy |

A unit's state (`planned`, `ready`, `running`, `review`, `fixing`, `reviewed`, `blocked`, `parked`, `landed`) is not in its record. It is replayed from the run log.

## Handoff (`H`)

Something one unit passes to another.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `from` | `U-` ID | yes | The sender |
| `to` | `U-` ID | yes | The receiver; it must depend on the sender |
| `thing` | string | yes | What is handed over |
| `contract` | string | no | The agreed shape of it |

A handoff is satisfied when the sender has landed and, if one of the sender's own unit checks lists the handoff in `proves`, that check has a passing receipt. `bf plan check` demands that some check lists every handoff in `proves`.

## Checkpoint (`K`)

A point where a set of units has landed and the result is proven.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | |
| `text` | string | no | |
| `after` | list of `U-` IDs | yes | The units on it. Each of them names this checkpoint |
| `proves` | list of designed or handoff IDs | yes | What closing it proves. Each ID needs a check among `checks` that proves it |
| `checks` | list of `X-` IDs | no | The checks that run on the checkpoint's working copy |
| `human_step` | string | no | A step only the human can do; it needs an approved `checkpoint-step` gate |
| `on_human_step` | `stop` or `carry-on` | no | Whether the factory waits for the human step or continues with independent work |
| `pull_request` | boolean | no | Whether the checkpoint's branch becomes a pull request |

## Check (`X`)

How one or more parts are proven.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | |
| `text` | string | no | |
| `how` | `command`, `agent` or `human` | yes | `command` checks are run by `bf evidence run`. `agent` and `human` checks are proven by `bf evidence attach` |
| `command` | string or list of strings | for `how = "command"` | The exact command. `bf evidence run` accepts only this command |
| `proves` | list of designed or handoff IDs | yes | What a passing receipt of this check proves |
| `scope` | `unit` or `checkpoint` | yes | Where it runs: in a unit's working copy before landing, or in a checkpoint's working copy before closing |
| `unit` | `U-` ID | for `scope = "unit"` | The unit that must pass it before it lands |

(The record's own kind is `kind = "check"`, so the three ways of proving use the field name `how`.)

## Finding (`F`)

A problem a reviewer found. Findings are created by `bf report ... --result findings`; the ID never changes, and only the outcome changes afterwards.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | |
| `unit` | `U-` ID | yes | |
| `severity` | `critical`, `high`, `medium` or `low` | no | |
| `where` | string | no | File and line |
| `action` | string | no | What must be done |
| `outcome` | `open`, `fixed` or `accepted` | no | Starts as `open`. `bf report --result fixed` sets `fixed` |

## Question (`Q`)

Asked before a stage's gate.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `title` | string | yes | |
| `context` | string | no | |
| `answer_kind` | `single`, `multi` or `text` | yes | (`kind` in the file given to `bf ask`) |
| `options` | list of tables | no | Each with `key`, `text`, `proposed` (boolean) and `why` |
| `stage` | string | no | The gate kind it must be answered before |
| `answer` | any | set by `bf answer` | |
| `answered_at` | string | set by `bf answer` | |
| `answered_turn` | integer | set by `bf answer` | The human turn after the question that the answer rests on |

## Files the engine writes

These are not put with `bf record put`.

### `factory/config.toml`

```toml
project = "myproject"
project_id = "3f9c1a7e5b2d4c60"   # names the state folder outside the project
main_branch = "main"
branch_prefix = "bf/"
```

### `factory/policy.toml`

Only the human edits it. The guard keeps agents away from it.

| Section | Keys | Meaning |
|---|---|---|
| `[limits]` | `max_rounds`, `idle_minutes` | Review rounds before parking (3); idle time before a running unit counts as stuck (60) |
| `[tiers]` | `builder`, `reviewer`, `verifier` | Model tier by role |
| `[budget]` | `cap`, `unit` | Spending cap (0 = none) and its unit |
| `[review]` | `required` | A unit needs a clean review before landing |
| `[ledger]` | `kinds` | Which record kinds the ledger holds to account |
| `[evidence]` | `tail_bytes`, `max_attach_bytes` | Output kept in a receipt; attachment size limit |
| `[human]` | `sessions`, `approval_words`, `refusal_words`, `transcript_root`, `transcript_match` | Session ids that count as the human's own (empty means any); the words an approval must hold and a refusal must not (lists in English, German, French and Spanish by default); where the harness keeps transcripts (`~/.claude/projects`); `strict` or `soft` comparison of the transcript's last message with the prompt |
| `[roles]` | `tokens` | `bf report` needs the one-use token `bf next` handed out (default true) |
| `[exec]` | `path`, `git`, `sandbox` | The `PATH` checks run with and the `git` program, captured at `bf init`; whether checks and git run under the system's sandbox where there is one. Only the human edits them, and a change takes effect when the human accepts it |
| `[guard]` | `allow`, `protected_extra` | `abstain` (default: the guard says nothing about calls it does not deny) or `allow`; more protected paths |

### `factory/receipts/rc-NNNNN.toml`

Written only by `bf evidence`. Sealed with the key in `.bf/key`.

| Field | Meaning |
|---|---|
| `id` | `rc-00001` |
| `type` | `run`, `attach`, `fail` or `unreachable` |
| `check` | The check's ID |
| `unit` | The unit, when given |
| `command`, `exit` | For a run: the command and its exit code |
| `content_hash` | The content hash the command ran on (defined in engine.md) |
| `output_hash`, `bytes`, `output_tail` | SHA-256 of the full output, its length, and the last bytes of it |
| `file_hash`, `file_name` | For an attachment: the stored file |
| `reason` | For `fail` and `unreachable`: what the verifier saw or what blocked it |
| `at`, `seconds`, `working_copy` | When, how long, and where |
| `program` | The program that ran the command (`/bin/sh`) |
| `sandbox` | `sandbox-exec` if the command ran in the sandbox, `none` if it did not |
| `check_hash` | The hash of the check record at the time; if the record changes, the receipt stops counting |
| `seal` | HMAC over the key and the fields above |

A receipt counts only if its seal is valid, its exit code is 0 (for a run), and its `content_hash` equals the content now in the working copy being judged.

### `factory/log/events.jsonl`

One event per line, each with `seq`, `at`, `kind`, `prev` and `h` (an HMAC under the key in the state folder). Event kinds: `project.init`, `head.accept`, `policy.accept`, `tokens.reset`, `programs.accept`, `human.turn`, `human.turn.rejected`, `gate.open`, `gate.approve`, `gate.reject`, `testmode.grant`, `run.start`, `run.stop`, `unit.start`, `unit.report`, `unit.land`, `unit.park`, `evidence.run`, `evidence.attach`, `checkpoint.close`, `question.asked`, `question.answered`, `amend.open`, `amend.narrow`, `amend.approve`.

Gates, approvals, test-mode grants and amendments live here, not in record files, so nothing can change them after the fact: a decision is valid only when the replayed log accepts it. A `gate.approve` counts only if it cites a human turn recorded after the gate opened and not used before, with a quote that is in that turn's text (the turn is not only a question), or is simulated inside a run that holds a test-mode grant (or, for a design gate, a grant waiting for its run).
