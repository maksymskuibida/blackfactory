//! The usage text printed by `bf help`, `bf --help` and `bf -h`.

pub(crate) const HELP: &str = "bf: the Black Factory engine

State:    init | record put|get|list | id new | ledger check|impact | plan check
Run:      run start|stop | status | progress | log | next | brief | report | unit start|land|release|pause|resume|sync
Proof:    evidence run|attach|verify|fail|unreachable | checkpoint close
Fixes:    checkpoint finding|review|fix|sync | unit scope|restore|format
Review:   review token <kind>|brief <kind>|split <A-n> --stage <s>=<IDs>|finding <gate or A-n> --accept <F-ids> --quote <words>|release <gate or A-n> --quote <words>
Humans:   gate open|approve|reject|status | human-turn | hook <event> | test-mode grant
Checks:   checks list|state|propose|set | checks wave|unit question|propose|set|state | checkpoint wave-check
Roles:    roles list|show|token
Pages:    view <kind> | render <kind>
More:     ask | answer | questions | amend open [--repair]|revise|checkpoint before|after|split <K-n> <U-n>... [--title <t>]|narrow|approve|withdraw | cost | stuck | doctor [--perf] | clean [--dry-run] | guard

Every command that prints data takes --json. Exit codes: 0 done, 2 refused, 1 error.
See docs/engine.md.";
