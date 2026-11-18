# Asking the human

## How a report is written

Write a longer report only when you hand back or need the human, in the order needs you, done, next, risks, and put everything the human needs before a question. Text after a question box is not seen, so commands, links and context come first and the question carries only itself. Everything you say about a state is read from the engine first.

## Asking questions

Ask only what you cannot decide and what changes the result. Give a proposal for each question with a reason; never preselect it. Write the questions to a JSON file (a list in the `questions` shape of the page data: `title`, `context`, `kind` single, multi or text, `options` with `key`, `text`, `proposed`, `why`), then `bf ask --file <q.json> --stage <gate kind>`. With no questions, go on. Otherwise:
1. `bf render questions <stage> --template .claude/skills/factory/page.html --out .bf/pages/questions-<stage>.html` (the stage is the gate kind given to `bf ask`; the page holds no time)
2. Publish it (see below). Show the link. Tell the human to say "done" in chat when finished. End your turn.
3. When the human says done, read the saved answers from the page database (collection `answers`, one document per question ID, with `ArtifactData` list; ignore the `done.*` document). Write each to `.bf/drafts/` and run `bf answer <Q> --file <a.json>`: `{"choice": "<key>", "note": "<text>"}` for a single question, `{"choices": ["<key>", ...]}` for a multi one, the text in `note` for a text question. When the human gave their own answer, the page saved `choices` containing the reserved key `other` and a field `other` with their text: write `{"choice": "other", "other": "<text>", "note": "<note>"}` (multi: `{"choices": ["A", "other"], "other": "<text>"}`). The engine needs a human message after the question, which "done" is.
4. If the page cannot be published, ask the questions in chat, in one message, always offering "or your own answer" besides the options, and record what the human types with `bf answer` (their own answer as `{"choice": "other", "other": "<text>"}`).
5. `bf questions --open` must be empty before the gate opens.

**Open questions already asked.** If `pages` has a `questions-<stage>` link, read that page again with the Artifact tool and check that it still matches `bf questions --open`. If it does, give the human the link and, when they say done, read the saved answers from it. If the questions or the page template changed, render and re-publish to the same link. If no link is recorded, render and publish, then record it.

## Checks the human decides

A check setting, a plan repair, a park, a release, a sync and the six acts of rule 6 of `SKILL.md` are the human's. Tell them what the engine refused and why, in its words, then stop for their words. Only a message the human typed is a decision.

## Publishing a page

1. Render to one self-contained HTML file with `bf render` (never edit the HTML by hand).
2. Publish it with the Artifact tool as a private artifact with the capabilities `db` and `user` (the page saves read marks, picks, notes and answers in its database). Before the first publish, load the skills the Artifact tool names for design and capabilities.
3. Later rounds republish the same file path so the link stays the same.
4. Show the link as visible text in chat, on its own line, and record it at once: `bf page set <name> <url>` (for questions, `bf page set questions-<stage> <url>`), with the names the engine's index uses (`questions-<stage>`, `gate-<gate-id>`, `plan`, `run`, `ledger`, `coverage`, `checkpoint-<K>`, `index`). `bf status --json` then shows it in `pages` for the next session. Never put a page link into a code repository.
5. The page is for reading, picks and notes. The decision is always given in chat.
6. If the Artifact tool is not available in this Claude app, say so once and carry on in chat: ask the questions in chat, and for a gate give the title, one line per item and the round's changes, then stop and wait for the human.
