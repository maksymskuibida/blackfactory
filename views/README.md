# Pages

Every page the factory shows a person comes from one template and one JSON contract. The engine prints the data, the template draws it, and the person reads, marks what they have read, picks and writes notes. The decision itself is always given in the chat.

```
views/
  templates/page.html   the one template: inline CSS and JS, renders every kind
  tokens/tokens.json    colours, type, spacing, radii for both themes: the only place values live
  contract/             view.schema.json and README.md: what the data means
  samples/              one invented sample per kind
  dev/build.py          writes the tokens into the template
  dev/preview.py        renders the samples (and any JSON) for looking and testing
```

## Render a page

```bash
bf render <kind> [id] --template views/templates/page.html --out page.html
```

`bf render` puts the output of `bf view <kind> [id] --json` at the marker `/*BF_DATA*/` inside the `bf-data` script of the template and writes one self-contained HTML file. The file needs no server and no other file. It loads only the IBM Plex fonts from Google Fonts and falls back to system fonts without them.

## Publish it

Publish the file as a **private artifact** with the capabilities `db` and `user`:

```
capabilities: {db: {}, user: {}}
```

With them, read marks and notes are kept per viewer, and answers and picks are kept for everyone with access, so an agent can read them. Without them the page keeps the same data in the browser. Republishing to the same address keeps everything already saved, because the data is keyed by the page's `id`, never by its text.

## Change the look

Edit `tokens/tokens.json`, then run:

```bash
python3 views/dev/build.py
```

This rewrites the generated token block in the template. `python3 views/dev/build.py --check` exits 1 if the template is out of date. Component styles live in the template between the `COMPONENTS` markers; `build.py --css FILE` writes tokens and components as one stylesheet.

## Local viewing and the character set

The template declares `<meta charset="utf-8">` on its second line (after the `<title>`), so a rendered file shows the middle dot and arrow correctly when opened from disk or served without a charset header. The published form is unaffected: the artifact skeleton already declares UTF-8 in its head, and a second declaration inside the page body is ignored. Both were tested: the raw rendered file served with no charset header, and the same file wrapped in a copy of the skeleton. `preview.py` also adds charset and viewport meta to the sample pages so a phone width means the same thing as when published.

## Look at the samples

```bash
python3 views/dev/preview.py
python3 -m http.server 8731 --directory views/dev/out --bind 127.0.0.1
```

`python3 views/dev/preview.py --check` also validates the samples against the schema and checks that every link on the index sample leads to a rendered page. Each sample page records script errors in `window.__bfErrors`. Open `http://127.0.0.1:8731/_preview.html`. The samples get a stub of `window.claude` (an in-memory `db` and `user`) so marks, notes, picks and answers can be tried without publishing; every write is logged in `window.__bfstub.log`. Add `?nostub` to a page to see the browser-storage fallback. Check each page at desktop width and at 390 px, in light and dark. `views/dev/out/` is ignored by git.

Render any JSON the same way:

```bash
python3 views/dev/preview.py --one some-view.json --out some-view.html
```

## Check the data

The contract has a JSON Schema. With the `jsonschema` Python package installed:

```bash
python3 -c "import json,sys,jsonschema; s=json.load(open('views/contract/view.schema.json')); [jsonschema.validate(json.load(open(f)), s) for f in sys.argv[1:]]" views/samples/*.json
```
