# Report contract

The shape of the UI/UX verifier's last message. Written new for this harness; see `NOTICE.md`.

## Shape

One line per journey step and per screen state, then the accessibility line, then the verification line. Each line has the ID, the outcome and one sentence of evidence.

```
J-1 step 1: met - the sign-in page shows the form with both fields
J-1 step 2: failed - after submit the page shows no error text; the contract expects one
S-2 state empty: unreachable - the simulator tool `xcrun` is not installed
accessibility: met - every control has a name and a visible focus (WCAG 2.2 level AA)
verification: failed - 1 step failed, 1 state unreachable
```

## Rules

- Outcomes are exactly `met`, `failed` or `unreachable`.
- The sentence says what was looked at, not what was hoped.
- The verification line is `met` only when every line above it is `met`.
- A failed or unreachable line names the cause so the human can act: the tool missing, the state not reached, the text that differs.
- No line quotes user-facing copy as a fixed text; it names the label by its variable.
- The report claims nothing the receipts do not show.
