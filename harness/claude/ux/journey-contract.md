# Journey contract

What a journey record promises and how the verifier proves it. Written new for this harness; see `NOTICE.md`.

## A journey

A journey `J` is a list of steps. Each step says what the user does and what must be visible afterwards. A step is met by a screen state of a screen `S`.

- The user's action is a thing a person can do on that interface: click, tap, type, press a key, run a command.
- The expected result is observable in the capture: a page text, a screen element, an exit code, a line of output.
- A step names no copy as a fixed text. It names the label by its variable; the verifier reads the label from the project's localisation source.

## What the verifier does for each step

1. Start from the state the step needs (the previous step's result, or the start of the journey).
2. Act as the step says, with the driver of the screen's interface.
3. Capture one file that shows the result (a screenshot, a text dump, a transcript).
4. Settle the step: `bf evidence attach <check> <file> --outcome met|failed|unreachable --says "<J-id> step <n>: <what was seen>" --token <token>`.

## Outcomes

- `met`: the expected result is in the capture, and nothing on the screen contradicts the contract.
- `failed`: the action ran and the result is missing or wrong. Say what was seen and what the contract says.
- `unreachable`: the step could not be run (tool missing, product would not start, state not reachable). It is never a pass.

## Screen states

Every screen is also checked in each state the contract names: empty, loading, filled, error. Settle each as `<S-id> state <label>: <what was seen>`. A state the product cannot be driven into is unreachable, and the sentence says why.

## Rules

- One file and one outcome for one step. Do not bundle steps.
- Evidence comes from outside the built product: capture into a scratch folder, never into the checked-out result.
- A journey is done only when every step has a `met` receipt at the current content.
