# Checkpoints, fixes, syncs and the end

Checkpoint actions in `bf next --json`:
- `review-checkpoint` (reviewer; start the checkpoint reviewer).
- `wave-check` (engine; start `bf checkpoint wave-check` in the background; the end of that command is the notice you wait for, never a loop).
- `decide-checkpoint-review` (human; a parked review; their words come from a message written after the question was put to them).
- `decide-low-findings` (human; accept, decline or pick low findings).
- `decide-checkpoint-fix` (human; a fix needs a plan change, was turned down or was aborted; opening it again takes their words, from a message written after the question was put to them).
- `decide-wave-check` (human; a wave check failed and its fix was turned down).
- `answer-wave-checks` and `answer-unit-checks` (human; the two plan questions, see `build-loop.md`).

A checkpoint is briefed to its verifier, its reviewer and its fix builder (`bf brief <K> --role <role>`). `bf checkpoint finding|review|fix` hold the checkpoint's findings, its reviews and its fix. Say what a checkpoint's state is only as `bf status` or `bf next` gives it.

## The fix of a checkpoint

Aborting a fix that has a built round is the human's choice; never abort it yourself. When the engine offers the fix designer, start it like any worker on the model its directive names (see `roles.md`). A fix that needs a plan change goes to the human as `decide-checkpoint-fix`.

What to run on a checkpoint fix:
- `--open`: `bf checkpoint fix <K> --open` opens a fix for a failed check of the checkpoint's current content when none is open. A failed format or lint check ends with no review round once every change is accounted for.
- `low`: low findings of a review are the human's: `bf checkpoint finding <K> --accept low --quote "<their words>"` accepts them together, `--fix-findings <ids>` picks some to be fixed.
- The human may accept any finding of a parked fix or review in their own words.

## Syncs

- `main_ahead` is information only. When the human asks to take main in (or to update from the remote): pull into the local main branch only if they said so, let running units land, then run `bf checkpoint sync <K> --quote "<their words>"` for the open checkpoint at the tip of the stack.
- After `bf checkpoint sync`, follow `bf next`. No unit starts while a sync is open (`sync_open` in `bf status --json`). `resolve-sync` goes to a builder agent like a build directive (brief from `bf brief <K> --role builder`, working copy, token); it changes only the listed files, concludes the merge or repairs the failing check as its brief says, and reports with `bf report <K> --role builder ...`. `review-sync` goes to a reviewer agent the same way.
- `decide-sync` means it parked: tell the human why and act only on their words (`bf checkpoint sync <K> --release --quote "..."`, or abort). If the resolver reports blocked and the human is not needed to decide, you may run `bf checkpoint sync <K> --abort`; say so in chat. Agents never merge by hand outside these directives.

## End

When `bf next` says the run is done:
1. `bf ledger check`. It must pass. If it lists a gap, say which designed part is missing.
2. `bf status` for the summary. Tell the human in chat: what was built, what was proven, what was parked or amended, what it cost, each as the engine states it. Offer `bf render run` as a page.
3. The checkpoints are stacked, so the last one holds the whole result, on its branch `bf/<K>` in each code repository it touched. Ask the human once whether to merge it onto the main branch. Only if they say yes in chat, and the run was not a test run: `bf gate open checkpoint-step --ref <last K>`, then once their message is recorded `bf gate approve <gate> --quote "<their words>"`, then `bf checkpoint close <last K> --merge`, which brings every earlier checkpoint and merges in every repository involved (each main checkout must be on its main branch). The human may instead run `git merge` themself. In a workspace nothing but branches and commits is written into the code repositories.

## Test mode

Never suggest it. Only after the human writes in chat that they grant a test run, run `bf test-mode grant --quote "<their words>"` at once. It attaches to the active run, or to the next `bf run start`. Draft the records of every stage first, pass `bf plan check`, grant, `bf run start`, then open each gate in order (the order and the not-empty rules hold in a test run too) and satisfy it with `bf gate approve <gate-id> --simulated` instead of waiting. Say in each message that the approval is simulated. A test run can never be merged onto `main`. Without a grant `--simulated` satisfies nothing: do not use it.
