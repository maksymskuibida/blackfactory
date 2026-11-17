# Factory panel

A pane beside the chat in the Claude desktop app that shows where a Black Factory run stands, what waits on the human and the design chain from the idea to the build. It is a Claude Code mod of function hooks. Only the desktop app is supported: the terminal, an editor and a phone get nothing.

## What it shows

- **Overview.** The project and run, a stamp that says when the panel last read, a Refresh button, the progress line exactly as the engine prints it, a Now card (the open checkpoint, its units by state and the next thing that waits on the human, with a link to its page), the published pages newest first, and the design chain: Idea, Requirements, UI, Architecture, Plan and Build, each with its count of records and its latest gate round and state. Each step opens its own tab.
- **Status line.** While the pane is not shown, one row above the prompt says where the run stands and whether something waits on the human. Pressing anywhere on it opens the pane. It yields to a survey, the person can collapse it, and it is gone while the pane is shown.
- **States.** First read in progress, not a factory folder, engine not found, a read that failed (a red line under the header with the engine's message while the last good view stays), a design stage with no run, a run going, a parked or blocked unit, an open amendment and a finished run. Every screen reads from a narrow pane (one column under 48 columns) to a wide one (two columns from 80).

Colours, type sizes, spacing and the tones of tags come from the pages' tokens file. The panel uses the app's own text, background and accent colours, because the API does not say whether the app is light or dark, and keeps the tokens' tones for tags. Every tag also says its word.

## Options

Set them in the plugin's configuration.

| Option | Default | Meaning |
|---|---|---|
| `panel` | `auto` | `auto` opens the pane by itself in a desktop app session in a factory folder, once a read succeeded. `command` opens it only with `/factory-panel`. `off` draws nothing and reads nothing. |
| `pollSeconds` | `5` | How often the engine's event count is read while the pane or the status line is on screen. At least 3; `0` turns the poll off. |
| `bf` | `bf` | The name of the engine's command, when it is not `bf` on the path. |

A pane the person closed stays closed for the session. The command `/factory-panel` and the status line open it again. On any other surface the command answers that the panel is made for the desktop app. In a folder that is not a factory, or where the engine cannot be run, the command says which of the two it is and nothing opens.

## How it reads

The panel runs only the engine's read commands, as argument lists with no shell:

- `bf status --json`
- `bf progress --json`
- `bf record list --json`
- `bf view <kind> [id] --json`

`src/reads.ts` is the whole list, and the reader refuses any other argument list. The panel runs no other program and no command that changes state, reads no file and writes none. It hooks no tool call and no prompt, puts nothing into the prompt and sends nothing on the human's behalf. It keeps nothing beyond what the person chose in it (the tab, the open record and the way back, folds, the search), for the session.

## How it updates

It reads at session start, when the desktop app attaches, on Refresh and when a turn completes. While the pane is placed or the status line is on screen and not collapsed, it runs `bf status --json` once per interval and reads the rest only when the engine's event count changed. While both are hidden, closed or collapsed it reads nothing. A failed read keeps the last good view and shows the engine's own message under the header.

## How it is made

```
.claude-plugin/plugin.json   name, options
hooks/hooks.json             names hooks/register.tsx
hooks/register.tsx           the only file that touches the engine interface
types/index.d.ts             the model, the person's choices, the strings function, the tokens
src/reads.ts                 the list of commands
src/reader.ts                runs a read through one port
src/model.ts                 joins the reads: records with links both ways, traces, units, gates, amendments, pages
src/refresh.ts               reads and polls
src/panel.ts                 when the pane opens, the command, the presses
src/render/                  one renderer per screen
src/strings/en.ts            every word the panel writes itself
src/tokens.ts                written by views/dev/build.py from views/tokens/tokens.json
tests/ fixtures/             run with claude plugin test
```

Do not edit `src/tokens.ts` by hand: run `python3 views/dev/build.py`, and `python3 views/dev/build.py --check` fails when the file is out of date. Record texts, section and page titles, states and the progress line come from the engine; the panel's own words (button labels, tab names the engine does not give, empty and error lines) are in `src/strings/en.ts`, read through one function, and an empty string hides its element.

## Check it

```bash
claude plugin validate harness/claude/panel
claude plugin test harness/claude/panel
python3 views/dev/build.py --check
```
