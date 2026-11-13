# The view contract

`bf view <kind> [id] --json` prints one JSON object. The page template (`views/templates/page.html`) renders any kind of it. The machine-readable form is [`view.schema.json`](view.schema.json); this file says what each field means and what the page does with it.

Version 1 is fixed: fields may be added (marked *optional addition* below), never renamed or removed. Every field except `contract`, `kind`, `id`, `page.title` and `sections` may be absent, and the page must still render.

## Top level

| Field | Meaning |
|---|---|
| `contract` | Always `1`. |
| `kind` | `index`, `gate`, `questions`, `ledger`, `plan`, `run`, `checkpoint` or `coverage`; `report` is an *optional addition* for a written report that is not produced by the engine (it renders like any other kind). All kinds use the same layout; the kind decides which blocks the engine puts in. `index` is a list of pages with no filter bar. |
| `id` | Stable id of the page, for example `architecture-r2`. It is the key for everything the page stores, so keep it the same across rebuilds of the same page. |
| `project`, `built_at` | Shown in the footer. |
| `page` | The header card: see below. |
| `sections` | Cards in reading order. Each holds optional `blocks` above its `rows`. |
| `records` | Raw records, drawn once in one collapsed block at the end. Nothing else repeats them. |
| `questions` | Question cards, drawn after the sections. |
| `answer` | `prompt` is the sentence in the strip at the end. `how` is where the decision is given; today always `chat`. |

## `page`

`crumbs` (the last one is the current page), `crumb_hrefs` (*optional addition*, a link per crumb), `status` (`text` and a `tone` of `wait` amber, `ok` green or `info` blue), `title`, `lede` (one sentence), `facts` (tiles with `label`, `value`, `sub`), `round`.

`round` is `{n, prev, changed, new, removed}`. The page draws "Round 2 compared with round 1: 2 changed, 3 new, 1 removed" and each count is a button that shows only the changed rows. It counts the rows itself; the numbers in `round` are used only when no row carries a round mark.

## `rows`

A row is one line that opens in place.

| Field | Meaning |
|---|---|
| `id` | Mono id. Any id in this page's rows that appears in text becomes a link to that row. |
| `title` | One sentence that states the point, not only the topic. `**bold**`, `` `code` `` and a plain `https://` address (made a link) work. Never cut in the data: the page wraps it. |
| `label` | How costly the row is to get wrong (`critical`, `high`, `medium`, `low`) or how central it is (`core`, `main`, `minor`). Section headers count them. |
| `look_first` | An outlined "Look first" tag. |
| `needs_you` | The row is part of the "Needs you" filter. |
| `round` | `new`, `changed` or `removed`. A changed or new row has a coloured left edge and tint; a removed row is struck through. |
| `round_note` | Why it changed. Shown as "Changed in v2: ..." above the body. |
| `handled` | `{tone: ok|accepted|open, text}`, a tag such as "Fixed". |
| `body` | Blocks (below). |
| `refs` | Linked ids as chips with a tooltip. A chip whose label is an id on this page links to that row even with no `target`. |
| `refs_label` | *Optional addition.* The word before the chips; default "Linked". Use "Proves" on a check or checkpoint that lists the ids it proves. |
| `picks` | Buttons for the viewer's lean on this row. Nothing is ever preselected. |
| `note` | *Optional addition.* `false` hides the "Add a note" control. |
| `href`, `meta` | *Optional additions, kind `index`.* The page a row opens and a muted text on the right. |

Evidence outcome: `passed`, `failed` or `unreachable`. Absent means passed when `exit` is 0 and failed when `exit` is another number. A failed check is drawn red; an unreachable one (the check could not run, which says nothing about the thing checked) is drawn dashed in amber with "Could not run:" and its `reason`. Never send `unreachable` for a check that ran and failed.

Body blocks: `text`, `list`, `was` (the previous wording, label optional), `table`, `code`, `evidence` (check, command, exit code, commit, time, output tail, and *optional additions* `outcome` and `reason`), `image` (a path relative to the page).

## Section blocks

Drawn above the rows of a section, each `{"type": ...}`:

- `criteria`: groups of items with a verdict `holds`, `fails`, `partly` or `na`. A tally bar and a coloured dot per item.
- `layers`: boxed layers, one may carry `emphasis`. `flows` holds one `[down, up]` pair of texts between each two layers; an empty text leaves that arrow out.
- `steps`: a numbered strip. `human: true` marks the step that is yours.
- `waves`: columns of units with a state (`planned`, `ready`, `running`, `landed`, `parked`) and points. `edges` become "after U-1" lines, so nothing needs fixed positions.
- `coverage`: requirement, unit, checkpoint, check count and a state (`proven`, `planned`, `gap`). The page sorts gaps first, then planned, then proven, and with more than 12 rows shows every gap plus the first rows up to 12 and folds the rest behind "Show N more". The tally above always counts all rows.
- `meter`: a labelled bar.

All of these reflow to one column at phone width. Wide tables and code scroll inside their own box.

## Questions

`{id, title, context, kind, options, answer}` with kind `single`, `multi` or `text`. Options are `{key, text, proposed, why}`. A proposed option is tagged and its `why` shows in a box with a Choose button; it is never chosen for the viewer. Every single and multi question also shows "Other" after its own options (the template draws it; set `other: false` on the question to turn it off). Choosing Other opens a text field for the person's own answer, and the question counts as answered with Other only when that text is not empty. In a single question Other replaces the listed options; in a multi question it can be combined with them. The saved answer lists the reserved key `other` in `choices` and holds the text in a field `other`; `answer.other` in the data shows it filled in. Every question has a note box. Single and multi questions are required (set `optional: true` to relax), text questions are optional (set `required: true` to require). When any question is required the bar shows a Done button that unlocks once all required ones are answered. The page only records Done; the viewer tells the agent in chat.

## Page rules the template follows

1. The decision is given in chat, never on the page. The page is for reading, read marks, picks and notes.
2. Every row has its own read mark, tied to a hash of the row's `title` plus the text of its `body` and nothing else. Change a word and that row is unread again; change a label, a note or a round mark and it is not. The hash is `cyrb53(title + "\n" + body leaf strings joined by "\n")` in base 36, where leaf strings are every string or number inside `body`, in order, leaving out `type` and `path` keys.
3. A change between rounds is shown in place, in every view. Nothing is drawn twice.
4. "Needs you" is a filter over all sections, not a block. Filters: All, Needs you, Unread, Changed.
5. A mark is set only with the row's read dot, the "Mark read" button, "Mark this section read" or "Mark all read". Opening a row does not mark it.
6. The sticky bar is one row of about 48 px on a desktop and under 100 px on a phone. Nothing is wider than a 390 px screen except inside its own scroll box.

## What the page stores, and where

Published as an artifact with the `db` and `user` capabilities, the page keeps:

| What | Where | Who sees it |
|---|---|---|
| Read marks `{marks: {"<section>~<row>": hash}}` | `data/users/<id>/reads-<page id>` | the viewer only |
| Row notes `{notes: {"<section>~<row>": text}}` | `data/users/<id>/notes-<page id>` | the viewer only |
| Question answers `{id, question, choice, choices, other, note, at}` (`choices` may hold the key `other`, its text is in `other`) | `answers/<question id>` | everyone with access |
| "Done" on a form `{id, done, at}` | `answers/done.<page id>` | everyone with access |
| Row picks `{page, row, pick, at}` | `picks/<page id>.<row id>` | everyone with access |

Writes are one at a time per document, only after a change, coalesced after a short pause, and never made from a snapshot callback. Loading has a 10 second limit after which the page says the saved data cannot be reached yet. Without the capabilities, or outside an artifact, the same data is kept in the browser's `localStorage` under `bf:<page id>:...`; without that, the page still renders and works for the visit.

Private notes are private: an agent reads them only if the viewer says them in chat. Answers and picks are shared so an agent can read them.
