# The repair judge and the plan's tests rule

## When `bf next` gives `judge-repair`

A plan repair that changes no design part may be judged by a subagent instead of waiting for the human. The directive `judge-repair` (role conductor, work `amender`) names the amendment, a command that issues the judge's token (`bf roles token <A-n> judge`), and the tier, effort and model.

1. Start a subagent of type `repair-judge` with the directive's tier, effort and model, without stopping and without asking. The prompt is the amendment number, the token and the model, and nothing else.
2. You never judge the repair yourself and never run `bf amend judge`; only the subagent does. If you run it, the engine refuses it for lack of a pass, records `amend.judge.failed`, and the repair becomes the human's for good.
3. Only the engine turns a `go` into an approval. A `go` that is approved needs no message to the human beyond one line saying the repair was judged and approved. An `ask` waits for the human as before.
4. Show the human every judged `ask` with its reasons, and every judged repair they have not seen, as `bf next` lists them. They are seen only when the human says so in their own words: `bf amend seen <A-n> --quote "<their words>"`.
5. An `ask` repair that blocks later repairs: withdraw it (`bf amend withdraw <A-n> --reason "<why>"`) and open a smaller one that changes none of the `ask`'s records. A repair that changes one of them stays the human's until they have seen the `ask`.
6. When the judge subagent ends with no verdict taken, or `bf next` shows the repair as the human's step, tell the human why in plain words, as the engine gives it, and go on with other work. Never start a second judge for that repair.
7. A repair opened from a fix design's item marked for a change of the plan is opened with `bf amend open --repair --from-design <K> <ID>...`. It is never judged and stays the human's.

## The note of a circle

When `bf next` notes a circle of held files, open a plan repair (`bf amend open --repair <U>...`) and narrow it: put one unit after the other (`depends_on`) or move the file. The judge then judges it.

## The three checkpoints of the plan's tests rule

Right after A-16 is approved, open the plan repair that lists X-104, X-105 and X-106 on the checkpoint of U-52 to U-66, X-114, X-115 and X-116 on the checkpoint of U-67 to U-73, and X-119, X-120 and X-121 on the checkpoint of U-74 and U-75. Verify none of those three checkpoints before that repair is approved.

## The two halves of the r3.1 rule

- Until an engine that holds U-52 is installed, no record names `tests`, `full_suite`, `heavy` or PL-1.
- After it, a unit that a human-approved amendment adds, or whose `files`, `text` or `delivers` it changes, names `tests` or `full_suite` in the same amendment. Otherwise the approval is refused for the plan problem; name the missing field in the amendment and narrow again.
