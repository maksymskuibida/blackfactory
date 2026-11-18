# Spaces and intents

A **space** holds the rules, the templates and the documents that every build in it shares. An **intent** is one build in a space, with its own records, log and working copies. A session is tied to one intent. Read the state from the engine, never from memory: `bf space list`, `bf space show` and `bf intent list` say what exists, and `bf status` says which intent this session is tied to.

- A session first tied to the workspace's current intent works there. `bf intent use <name>` ties a session that has no intent yet to an intent of its space with no words; moving a session that is already tied, or tying it to another space, needs the human's words: `bf intent use <name> --quote "<their words>"` and `bf space use <space> --quote "<their words>"`. The words must name the intent or the space.
- The files in force of a space (its rules, templates and documents) are never yours to change. The engine writes them, and an agent's write there is denied. Only a change the human accepts with their own words takes effect (`bf space edit` and its subcommands are for a session the human tied to the space).
- Rules reach every brief in layers: the organisation, the team, the project and the stage. A worker reads them in its brief; you add nothing of your own to a worker's prompt.
- A rule that one project or one machine needs belongs to that space's rules, written by the human, not to these instructions.
- `bf knowledge show` prints the note of code knowledge of a repository; the knowledge writer hands it in with `bf knowledge put`. `bf rules list`, `bf rules proposals`, `bf rules accept`, `bf rules decline` and `bf rules retire` hold the rule ledger; a proposal is accepted, declined or retired only on the human's words with `--quote`.
- A session tied to nothing acts in no space. A command that needs an intent is refused with the steps; follow them or ask the human which space to move to.
