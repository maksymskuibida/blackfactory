# Driver: web pages

For screens whose `interface` is `web`. Command-line tools only.

## Tools needed

- `node` and `npx` (to run Playwright's command line)
- Playwright (its command line, with a browser installed; an npx package is enough)
- `curl` (to wait for the page to answer)

Check with `command -v node npx curl` and `npx --no-install playwright --version`. When one is missing, run `bf evidence unreachable <check> --reason "<tool> missing for web screen <S-id>"`. Never pass.

## Start the product

Start it as the brief or the project's README says (a dev server command), in the background, from the working copy. Wait until `curl -sf <url>` answers, with a fixed number of tries. If it never answers, the step is unreachable.

## Act

Write a small Playwright script in the scratch folder (outside the built product) and run it with `node`, or use `npx playwright test` with a test file kept in the scratch folder. Act as the journey step says: open the URL, click, fill, press keys. Find elements by role and accessible name first.

## Capture evidence

- `npx playwright screenshot <url> <scratch>/<id>.png` for a plain page, or `page.screenshot` in the script for a state after actions.
- The page's accessibility tree (`page.accessibility.snapshot()` or `ariaSnapshot`) written to a text file, for the accessibility result.
- Attach one file per step with the step evidence command.

## Stop

Stop the server you started (by its own process ID, kept when you started it). Never search the whole machine for processes.
