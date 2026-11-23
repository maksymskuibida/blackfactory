# Amendments

Once a stage's gate is approved its records are frozen: they change only through an amendment. `bf amend open [--repair] <ID>...` opens one. Only one amendment is open at a time: `bf amend open` is refused while one is open. Change the open one with `bf amend revise <ID>...` (it keeps its ID, needs narrowing again and a new human message before approval) or drop it with `bf amend withdraw <A-n> --reason "<why>"`.

A plan repair (a check's command, a unit's files or order, moving a unit, or `bf amend checkpoint before|after|split <K> <U>...`) needs no designer and no review. Any other amendment goes through these steps, each started without stopping or asking:
1. The general designer first: `bf roles token <A-n> designer`, then `bf brief <A-n> --role designer`. It designs the whole amendment, or reports a split into stages with `bf review split`.
2. After a split, each stage designer named: `bf review token --subject <A-n> --stage <stage> --role <role>`, then `bf review brief <stage> --subject <A-n> --role <role>`. The general designer then reconciles the drafts.
3. The review runs in the shape the design took: one review for each stage of a split, then the review that the stages agree; for a design with no split, the solo reviewer (`bf review token amendment --subject <A-n>`), and any stage review it asks for. Rounds, parking, accepting and releasing are as in the design review of `design-stages.md`, with the amendment ID.
4. Narrow it (`bf amend narrow --file <f>`), put the designer's calls made without asking, each with the other option, before the human, and only the human's words approve it: `bf amend approve --quote "<their words>"`. The human may lift the designer rule for one stage or one amendment in their own words: `bf roles lift <stage|A-n> --quote "<their words>"`.

An amendment names its new records: a record that does not exist yet is named in `bf amend open` like any other, and is put while the amendment waits for the human.

A kept record that must change goes in this order, the order the engine accepts: `bf amend revise` naming the IDs it names now and every record that changed, put the records, narrow, the human's yes, approve.

## After `bf amend approve`

The steps after an approved amendment, in this order:
1. Commit the factory's records (`git add -A && git commit -m "<what the amendment changed>"` in the project folder or the workspace, never in a code repository of a workspace). The step right after `bf amend approve` is the commit of the factory's records; you neither judge whether to commit nor ask the human.
2. Follow `bf next`: it offers exactly the re-opened work.
3. Tell the human in one line what the amendment re-opened, as `bf next` shows it, not as you judge it.

When a designer or a design reviewer names a fact it lacks, start an investigator for that one question and pass its report on. Never ask a designer to measure, build or test (D-111).
