//! The usage text printed by `bf help`, `bf --help` and `bf -h`.

pub(crate) const HELP: &str = "bf: the Black Factory engine

State:    init | record put|get|list | id new | ledger check|impact | plan check
Run:      run start|stop | status | progress | log | next | brief | report | unit start|land|release
Proof:    evidence run|attach|verify|fail|unreachable | checkpoint close
Fixes:    checkpoint finding|review|fix|sync | unit scope
Humans:   gate open|approve|reject|status | human-turn | test-mode grant
Checks:   checks list|state|propose|set | checks wave|unit question|propose|set|state | checkpoint wave-check
Roles:    roles list|show|token
Pages:    view <kind> | render <kind>
More:     ask | answer | questions | amend open|revise|narrow|approve|withdraw | cost | stuck | doctor [--perf] | guard

Every command that prints data takes --json. Exit codes: 0 done, 2 refused, 1 error.
See docs/engine.md.";
