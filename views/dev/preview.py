#!/usr/bin/env python3
"""Render view-contract JSON through the page template, for looking at and testing pages.

    python3 views/dev/preview.py                       every sample in views/samples/ into views/dev/out/, plus _preview.html
    python3 views/dev/preview.py --one FILE.json --out FILE.html [--title T] [--no-stub]
                                                       one page, by default with no stub (as it would be published)

By default the pages get a stub of window.claude (an in-memory db and user, kept in the browser's localStorage) so the
store paths run without publishing: read marks, notes, picks and answers can be exercised, and every write is logged in
window.__bfstub.log. Add ?nostub to a page's address to see the localStorage fallback. Standard library only.
"""
import argparse
import json
import sys
from pathlib import Path

VIEWS = Path(__file__).resolve().parent.parent
TEMPLATE = VIEWS / "templates" / "page.html"
SAMPLES = VIEWS / "samples"
OUT = VIEWS / "dev" / "out"
MARKER = "/*BF_DATA*/"
# what the publish skeleton adds to every artifact page; the preview adds the same so a phone width means the same thing
HEAD = '<meta charset="utf-8">\n<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">\n'

STUB = r"""<script>
/* preview stub of window.claude: an in-memory db and user, persisted in this browser's localStorage. Not part of any page. */
(function () {
  window.__bfErrors = [];
  window.addEventListener('error', function (e) { window.__bfErrors.push(String(e.message)); });
  if (/[?&]nostub/.test(location.search)) return;
  var KEY = 'bfstub:' + location.pathname, store = {}, subs = [], busy = {};
  var log = [];
  try { store = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch (e) { store = {}; }
  function save() { try { localStorage.setItem(KEY, JSON.stringify(store)); } catch (e) { } }
  function clone(x) { return JSON.parse(JSON.stringify(x)); }
  function snap(path) { var d = store[path]; return { id: path.split('/').pop(), exists: !!d, data: function () { return d ? clone(d) : undefined; }, metadata: { fromCache: false, hasPendingWrites: false } }; }
  function inColl(coll) { var n = coll.split('/').length; return Object.keys(store).filter(function (p) { return p.indexOf(coll + '/') === 0 && p.split('/').length === n + 1; }).sort(); }
  function notify(path) { setTimeout(function () { subs.slice().forEach(function (s) { if (s.match(path)) s.fire(); }); }, 5); }
  function write(path, fn) {
    if (busy[path]) { log.push({ op: 'OVERLAP', path: path }); }
    busy[path] = (busy[path] || 0) + 1;
    return new Promise(function (res) { setTimeout(function () { fn(); save(); busy[path]--; log.push({ op: 'write', path: path, at: Date.now() }); notify(path); res(); }, 120); });
  }
  function doc(path) {
    return {
      id: path.split('/').pop(), path: path,
      get: function () { return Promise.resolve(snap(path)); },
      set: function (d) { return write(path, function () { store[path] = clone(d); }); },
      update: function (d) { return write(path, function () { store[path] = Object.assign(store[path] || {}, clone(d)); }); },
      delete: function () { return write(path, function () { delete store[path]; }); },
      onSnapshot: function (next) { var s = { match: function (p) { return p === path; }, fire: function () { next(snap(path)); } }; subs.push(s); setTimeout(s.fire, 30); return function () { subs = subs.filter(function (x) { return x !== s; }); }; },
      collection: function (p) { return coll(path + '/' + p); }
    };
  }
  function coll(path) {
    function qsnap() { var docs = inColl(path).map(snap); return { docs: docs, size: docs.length, empty: !docs.length, docChanges: function () { return []; }, metadata: { fromCache: false, hasPendingWrites: false } }; }
    return {
      path: path,
      doc: function (id) { return doc(path + '/' + id); },
      get: function () { return Promise.resolve(qsnap()); },
      onSnapshot: function (next) { var s = { match: function (p) { return p.indexOf(path + '/') === 0; }, fire: function () { next(qsnap()); } }; subs.push(s); setTimeout(s.fire, 30); return function () { subs = subs.filter(function (x) { return x !== s; }); }; }
    };
  }
  var db = { doc: doc, collection: coll };
  var user = { id: function () { return Promise.resolve('u_preview'); }, isOwner: function () { return Promise.resolve(true); }, canEdit: function () { return Promise.resolve(true); }, can: function () { return Promise.resolve(true); } };
  window.claude = { use: function (n) { return Promise.resolve(n === 'db' ? db : n === 'user' ? user : null); } };
  window.__bfstub = { log: log, dump: function () { return clone(store); }, reset: function () { store = {}; save(); location.reload(); } };
})();
</script>
"""


def render(template: str, data: dict, title: str | None = None, stub: bool = False) -> str:
    payload = json.dumps(data, ensure_ascii=False, indent=None)
    # keep the JSON inert inside its script element
    payload = payload.replace("<", "\\u003c").replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")
    if MARKER not in template:
        raise SystemExit("template has no " + MARKER + " marker")
    html = template.replace(MARKER, payload, 1)
    if title:
        html = html.replace("<title>Black Factory</title>", "<title>" + title.replace("<", "") + "</title>", 1)
    if stub:
        mark = '<script>\n(function () {\n  \'use strict\';'
        i = html.index(mark)
        html = html[:i] + STUB + html[i:]
    return html


def check(out: Path) -> int:
    """Checks that need no browser: schema, and the links of every page of kind index."""
    bad = 0
    try:
        import jsonschema
        schema = json.loads((VIEWS / "contract" / "view.schema.json").read_text())
    except ImportError:
        jsonschema, schema = None, None
        print("jsonschema is not installed: schema check skipped")
    for f in sorted(SAMPLES.glob("*.json")):
        data = json.loads(f.read_text())
        if jsonschema:
            errs = list(jsonschema.Draft202012Validator(schema).iter_errors(data))
            if errs:
                bad += 1
                print("SCHEMA", f.name, errs[0].message[:120])
        if data.get("kind") == "index":
            hrefs = [r["href"] for s in data["sections"] for r in s.get("rows", []) if r.get("href")]
            if not hrefs:
                bad += 1
                print("INDEX", f.name, "has no row with a link")
            for h in hrefs:
                if not (out / h.split("#")[0]).exists():
                    bad += 1
                    print("INDEX", f.name, "links to a page that was not rendered:", h)
    print("checks:", "all passed" if not bad else f"{bad} problem(s)")
    return 1 if bad else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--one", help="render one JSON file")
    ap.add_argument("--out", help="output file for --one, or the folder for the samples")
    ap.add_argument("--title")
    ap.add_argument("--stub", action="store_true", help="with --one: add the window.claude stub")
    ap.add_argument("--no-stub", action="store_true", help="samples: leave the stub out")
    ap.add_argument("--check", action="store_true", help="samples: also validate against the schema (if jsonschema is installed) and check that every link on the index page leads to a rendered page")
    a = ap.parse_args(argv)
    template = TEMPLATE.read_text()
    if a.one:
        data = json.loads(Path(a.one).read_text())
        out = Path(a.out or Path(a.one).with_suffix(".html"))
        out.write_text(render(template, data, a.title, stub=a.stub))
        print("wrote", out)
        return 0
    out = Path(a.out) if a.out else OUT
    out.mkdir(parents=True, exist_ok=True)
    links = []
    for f in sorted(SAMPLES.glob("*.json")):
        data = json.loads(f.read_text())
        dest = out / (f.stem + ".html")
        dest.write_text(HEAD + render(template, data, stub=not a.no_stub))
        links.append((f.stem, data.get("kind", ""), data["page"].get("title", "")))
        print("wrote", dest.relative_to(VIEWS.parent) if dest.is_relative_to(VIEWS.parent) else dest)
    rows = "".join(f'<li><a href="{n}.html">{n}</a> <small>{k}</small> <span>{t}</span></li>' for n, k, t in links)
    (out / "_preview.html").write_text(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>Black Factory previews</title>"
        "<style>body{font:15px/1.5 system-ui;margin:0;padding:24px;max-width:720px;background:#F5F6F8;color:#171A21}"
        "@media(prefers-color-scheme:dark){body{background:#111318;color:#E6E8EE}a{color:#8FA2FF}}li{padding:4px 0}small{opacity:.6;margin:0 8px}</style>"
        "<h1>Black Factory previews</h1><p>Every sample, rendered through views/templates/page.html with a stub of window.claude. "
        "Add <code>?nostub</code> to a page to see the browser-storage fallback.</p><ul>" + rows + "</ul>"
    )
    print("wrote", out / "_preview.html")
    return check(out) if a.check else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
