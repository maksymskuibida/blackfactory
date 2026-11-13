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
  /* Behaves like the real db and user services, as db.d.ts describes them: data() is frozen and the same object while the
     document is unchanged, set() resolves to nothing after a delay, a first snapshot may come from the cache, your own write
     is echoed at once as pending and again when confirmed. ?stub=slow (set takes 1.5 s), fail (set rejects), hang (set never
     answers), late (the services answer after 1.5 s). */
  var MODE = (location.search.match(/[?&]stub=(\w+)/) || [0, 'ok'])[1];
  var KEY = 'bfstub:' + location.pathname, store = {}, subs = [], busy = {}, log = [], attempts = 0, frozen = {};
  try { store = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch (e) { store = {}; }
  function save() { try { localStorage.setItem(KEY, JSON.stringify(store)); } catch (e) { } }
  function clone(x) { return JSON.parse(JSON.stringify(x)); }
  function deepFreeze(o) { if (o && typeof o === 'object' && !Object.isFrozen(o)) { Object.keys(o).forEach(function (k) { deepFreeze(o[k]); }); Object.freeze(o); } return o; }
  function body(path) { var d = store[path]; if (!d) return undefined; if (!frozen[path] || frozen[path].src !== d) frozen[path] = { src: d, data: deepFreeze(clone(d)) }; return frozen[path].data; }
  function snap(path, meta) { var d = body(path); return { id: path.split('/').pop(), exists: !!d, data: function () { return d; }, metadata: meta || { fromCache: false, hasPendingWrites: false } }; }
  function inColl(coll) { var n = coll.split('/').length; return Object.keys(store).filter(function (p) { return p.indexOf(coll + '/') === 0 && p.split('/').length === n + 1; }).sort(); }
  function notify(path, pending) { setTimeout(function () { subs.slice().forEach(function (s) { if (s.match(path)) s.fire({ fromCache: false, hasPendingWrites: !!pending }); }); }, 5); }
  var LAT = MODE === 'slow' ? 1500 : 150;
  function write(path, make) {
    attempts++;
    if (busy[path]) log.push({ op: 'OVERLAP', path: path });
    busy[path] = (busy[path] || 0) + 1;
    var prev = store[path], had = path in store;
    return new Promise(function (res, rej) {
      if (MODE === 'hang') return;                         // the service never answers
      store[path] = make(prev); notify(path, true);        // your own write, echoed at once as pending
      setTimeout(function () {
        busy[path]--;
        if (MODE === 'fail') { if (had) store[path] = prev; else delete store[path]; notify(path, false); log.push({ op: 'rejected', path: path }); rej({ code: 'unavailable', message: 'stub: unavailable' }); return; }
        save(); log.push({ op: 'write', path: path, at: Date.now() }); notify(path, false); res(undefined);
      }, LAT);
    });
  }
  function doc(path) {
    return {
      id: path.split('/').pop(), path: path,
      get: function () { return Promise.resolve(snap(path)); },
      set: function (d) { return write(path, function () { return clone(d); }); },
      update: function (d) { return write(path, function (prev) { return Object.assign({}, prev || {}, clone(d)); }); },
      delete: function () { return write(path, function () { return undefined; }); },
      onSnapshot: function (next) {
        var s = { match: function (p) { return p === path; }, fire: function (meta) { next(snap(path, meta)); } };
        subs.push(s); setTimeout(function () { s.fire({ fromCache: true, hasPendingWrites: false }); }, 30); setTimeout(function () { s.fire({ fromCache: false, hasPendingWrites: false }); }, 300);
        return function () { subs = subs.filter(function (x) { return x !== s; }); };
      },
      collection: function (p) { return coll(path + '/' + p); }
    };
  }
  function coll(path) {
    function qsnap(meta) { var docs = inColl(path).map(function (p) { return snap(p); }); return { docs: docs, size: docs.length, empty: !docs.length, docChanges: function () { return []; }, metadata: meta || { fromCache: false, hasPendingWrites: false } }; }
    return {
      path: path,
      doc: function (id) { return doc(path + '/' + id); },
      get: function () { return Promise.resolve(qsnap()); },
      onSnapshot: function (next) {
        var s = { match: function (p) { return p.indexOf(path + '/') === 0; }, fire: function (meta) { next(qsnap(meta)); } };
        subs.push(s); setTimeout(function () { s.fire({ fromCache: true, hasPendingWrites: false }); }, 30); setTimeout(function () { s.fire({ fromCache: false, hasPendingWrites: false }); }, 300);
        return function () { subs = subs.filter(function (x) { return x !== s; }); };
      }
    };
  }
  /* self test: the read marks, the filters and the counts. Run by _selftest.html on a fresh page, never part of a published page. */
  window.__bfSelfTest = async function () {
    var fails = [], sleep = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };
    function ok(c, m) { if (!c) fails.push(m); }
    function vis(e) { return !e.hidden && e.getAttribute('data-show') !== '0'; }
    function shown() { return [].filter.call(document.querySelectorAll('.row'), vis); }
    function pill(f) { var p = document.querySelector('.pill[data-f=' + f + '] .n'); return p ? +p.textContent : -1; }
    function setf(f) { document.querySelector('.pill[data-f=' + f + ']').click(); }
    var total = document.querySelectorAll('.row').length;
    if (total < 3) return { skipped: true, fails: [] };
    setf('unread');
    var u0 = pill('unread');
    ok(shown().length === u0, 'unread filter: rows shown (' + shown().length + ') equal the count (' + u0 + ')');
    // 1. one row marked read under Unread
    var row = shown()[0], sec = row.closest('section'), secShown = [].filter.call(sec.querySelectorAll('.row'), vis).length;
    var dot = row.querySelector('.dot'); dot.focus(); ok(document.activeElement === dot, 'the dot can take focus');
    dot.click();
    ok(pill('unread') === u0 - 1, 'a mark under Unread: the count changes at once');
    ok(!vis(row), 'a mark under Unread: the row leaves the list at once');
    ok(shown().length === u0 - 1, 'a mark under Unread: one row fewer is shown');
    var ae = document.activeElement;
    ok(ae && ae !== document.body && !row.contains(ae), 'focus is not lost when the row goes');
    ok(ae && (ae.classList.contains('dot') || ae.classList.contains('pill')), 'focus goes to another dot or the filter');
    var nextDot = shown()[0] && shown()[0].querySelector('.dot'); ok(!nextDot || ae === nextDot, 'focus moves to the next row\'s dot, not to the top');
    if (secShown > 1) { var sh = sec.querySelector('.shown'); ok(sh && sh.textContent.indexOf((secShown - 1) + ' of ') === 0, 'the section header count is current (' + (sh && sh.textContent) + ')'); }
    await sleep(260);
    ok(row.hidden === true, 'the row is removed from the page after the fade');
    var undo = document.querySelector('#toast button'); ok(!!undo && !document.getElementById('toast').hidden, 'an Undo is offered');
    if (undo) { undo.click(); ok(pill('unread') === u0 && vis(row), 'Undo brings the row and the count back'); }
    // 2. a section
    var bar = [].filter.call(document.querySelectorAll('.secbar .btn'), function (b) { return b.textContent === 'Mark this section read' && vis(b.closest('section')); })[0];
    if (bar) {
      var s2 = bar.closest('section'), n2 = [].filter.call(s2.querySelectorAll('.row'), vis).length;
      bar.focus(); bar.click();
      ok(pill('unread') === u0 - n2, 'Mark this section read: the count falls by ' + n2 + ' at once');
      ok(!vis(s2), 'Mark this section read: the section leaves the list');
      ok(document.activeElement && !s2.contains(document.activeElement) && document.activeElement !== document.body, 'focus leaves the vanished section for a live control');
      await sleep(260); ok(s2.hidden === true, 'the section is removed after the fade');
      var u2 = document.querySelector('#toast button'); if (u2) u2.click(); ok(pill('unread') === u0, 'Undo restores the section');
    }
    // 3. everything
    document.getElementById('markall').click();
    ok(pill('unread') === 0 && shown().length === 0, 'Mark all read: nothing is left in the Unread list');
    var nn = document.getElementById('nothing'); ok(!!nn && !nn.hidden, 'Mark all read: a quiet line says all is read');
    var u3 = document.querySelector('#toast button'); if (u3) u3.click(); ok(pill('unread') === u0 && shown().length === u0, 'Undo restores every row');
    await sleep(50);
    // 4. All: the mark is visible and easy to undo
    setf('all');
    ok(shown().length === total, 'All shows every row');
    var r4 = shown()[0], d4 = r4.querySelector('.dot'); d4.click();
    ok(r4.classList.contains('read') && vis(r4), 'under All a read row stays, with its mark');
    ok(pill('unread') === u0 - 1, 'under All the count is current');
    d4.click(); ok(!r4.classList.contains('read') && pill('unread') === u0, 'the dot undoes a mark under All');
    // 5. Changed: nothing moves
    if (pill('changed') > 0) {
      setf('changed'); var n5 = shown().length, c5 = pill('changed'); shown()[0].querySelector('.dot').click();
      ok(shown().length === n5 && pill('changed') === c5, 'under Changed nothing moves when a row is marked read');
      shown()[0].querySelector('.dot').click();
    }
    // 6. Needs you: a pick keeps the row and lowers the count
    setf('needs');
    var picked = false, rows6 = shown();
    for (var i = 0; i < rows6.length && !picked; i++) {
      var tw = rows6[i].querySelector('.twist'); if (rows6[i].getAttribute('aria-expanded') !== 'true' && !rows6[i].classList.contains('open')) tw.click();
      var pb = rows6[i].querySelector('[data-pick]');
      if (pb && pb.getAttribute('aria-pressed') === 'false') {
        var n6 = shown().length, c6 = pill('needs'); pb.click();
        ok(shown().length === n6 && vis(rows6[i]), 'under Needs you a row that was picked stays');
        ok(pill('needs') === c6 - 1, 'under Needs you a pick lowers the count at once');
        var dt = rows6[i].querySelector('.tag.handled.ok'); ok(!!dt && !dt.hidden, 'a picked row carries a done mark');
        picked = true;
      }
    }
    return { skipped: false, fails: fails };
  };
  var db = { doc: doc, collection: coll };
  var user = { id: function () { return new Promise(function (r) { setTimeout(function () { r('u_preview'); }, 100); }); }, isOwner: function () { return Promise.resolve(true); }, canEdit: function () { return Promise.resolve(true); }, can: function () { return Promise.resolve(true); } };
  window.claude = { use: function (n) { return new Promise(function (res) { setTimeout(function () { res(n === 'db' ? db : n === 'user' ? user : null); }, MODE === 'late' ? 1500 : 20); }); } };
  window.__bfstub = { log: log, mode: MODE, attempts: function () { return attempts; }, dump: function () { return clone(store); }, reset: function () { store = {}; save(); location.reload(); } };
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


SELFTEST_PAGE = r"""<!doctype html><meta charset=utf-8><title>Black Factory self test</title>
<style>body{font:14px/1.5 ui-monospace,monospace;margin:16px;background:#fff;color:#111}.fail{color:#b00020}.pass{color:#1a7f37}iframe{position:fixed;left:-5000px;top:0;width:1000px;height:800px;border:0}</style>
<h1>Black Factory page self test</h1><pre id=out>running...</pre>
<script>
(async function () {
  var names = __NAMES__, out = document.getElementById('out'), lines = [], bad = 0;
  for (var i = 0; i < names.length; i++) {
    try { Object.keys(localStorage).filter(function (k) { return k.indexOf('bf:') === 0 || k.indexOf('bfstub:') === 0; }).forEach(function (k) { localStorage.removeItem(k); }); } catch (e) { }
    var f = document.createElement('iframe'); f.src = names[i] + '.html?selftest=' + Date.now(); document.body.appendChild(f);
    await new Promise(function (r) { f.onload = r; }); await new Promise(function (r) { setTimeout(r, 400); });
    var w = f.contentWindow, res;
    try { res = await w.__bfSelfTest(); } catch (e) { res = { fails: ['exception: ' + e.message] }; }
    var errs = (w.__bfErrors || []).length; if (errs) res.fails.push(errs + ' script error(s): ' + w.__bfErrors[0]);
    if (res.skipped && !errs) lines.push('skip  ' + names[i] + ' (no rows)');
    else if (res.fails.length) { bad++; lines.push('FAIL  ' + names[i] + '\n      ' + res.fails.join('\n      ')); }
    else lines.push('pass  ' + names[i]);
    f.remove();
  }

  // the saved-data service as the real one behaves: frozen data, delayed set(), echoes, slow, failing, late, none
  function clearAll() { try { Object.keys(localStorage).filter(function (k) { return k.indexOf('bf:') === 0 || k.indexOf('bfstub:') === 0; }).forEach(function (k) { localStorage.removeItem(k); }); } catch (e) { } }
  async function load(url) { var f = document.createElement('iframe'); f.src = url + (url.indexOf('?') < 0 ? '?' : '&') + 't=' + Date.now(); document.body.appendChild(f); await new Promise(function (r) { f.onload = r; }); return f; }
  var sleep = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };
  function nRead(f) { return f.contentDocument.querySelectorAll('.row.read').length; }
  function dot(f, i) { return f.contentDocument.querySelectorAll('.row .dot')[i]; }
  function stored(f) { var d = f.contentWindow.__bfstub ? f.contentWindow.__bfstub.dump() : {}; var k = Object.keys(d).filter(function (x) { return /reads-/.test(x); })[0]; return k ? Object.keys((d[k] || {}).marks || {}).length : 0; }
  function status(f) { var e = f.contentDocument.getElementById('savestat'); return e ? e.textContent : ''; }
  var sfails = [];
  function sok(c, m) { if (!c) sfails.push(m); }
  clearAll();
  var f = await load('gate.html'); await sleep(900);
  dot(f, 0).click(); await sleep(1300); dot(f, 1).click(); await sleep(60);
  sok(nRead(f) === 2, 'ok store: the second mark after the first save is applied (' + nRead(f) + ' read, expected 2)');
  await sleep(1500); sok(stored(f) === 2, 'ok store: both marks reach the store (' + stored(f) + ')'); sok(!(f.contentWindow.__bfErrors || []).length, 'ok store: no script error: ' + (f.contentWindow.__bfErrors || [])[0]);
  f.remove();
  f = await load('gate.html'); await sleep(1200);
  sok(nRead(f) === 2, 'reload: the saved marks come back (' + nRead(f) + ' read, expected 2)');
  dot(f, 2).click(); await sleep(60); sok(nRead(f) === 3, 'reload: a mark after loading saved data is applied (' + nRead(f) + ')');
  dot(f, 0).click(); await sleep(60); sok(nRead(f) === 2, 'reload: a mark can be taken back after loading saved data');
  await sleep(1500); sok(stored(f) === 2, 'reload: the store follows (' + stored(f) + ')'); sok(!(f.contentWindow.__bfErrors || []).length, 'reload: no script error: ' + (f.contentWindow.__bfErrors || [])[0]);
  f.remove();
  clearAll(); f = await load('gate.html'); await sleep(900);
  dot(f, 0).click(); await sleep(600); dot(f, 1).click(); await sleep(400);          // the second mark lands while the first write is on its way
  sok(nRead(f) === 2, 'a mark made while a write is in flight stays on the page (' + nRead(f) + ' read, expected 2)');
  await sleep(2500); sok(nRead(f) === 2 && stored(f) === 2, 'a mark made while a write is in flight is saved (' + nRead(f) + ' on the page, ' + stored(f) + ' stored)'); f.remove();
  clearAll(); f = await load('gate.html?stub=slow'); await sleep(900);
  dot(f, 0).click(); await sleep(40); dot(f, 1).click(); await sleep(40); dot(f, 2).click(); await sleep(60);
  sok(nRead(f) === 3, 'slow store: three quick marks are all on the page at once (' + nRead(f) + ')');
  await sleep(6000); sok(stored(f) === 3, 'slow store: all three are saved in the end (' + stored(f) + ')');
  sok(!f.contentWindow.__bfstub.log.some(function (l) { return l.op === 'OVERLAP'; }), 'slow store: never two writes at once to one document');
  sok(/Saved/.test(status(f)), 'slow store: the status ends as Saved (' + status(f) + ')'); f.remove();
  clearAll(); f = await load('gate.html?stub=fail'); await sleep(900);
  dot(f, 0).click(); await sleep(60); sok(nRead(f) === 1, 'failing store: the mark is on the page at once');
  await sleep(3200); sok(f.contentWindow.__bfstub.attempts() >= 2, 'failing store: the save is retried (' + f.contentWindow.__bfstub.attempts() + ' attempts)');
  sok(/Not saved/.test(status(f)), 'failing store: the page says not saved (' + status(f) + ')');
  dot(f, 1).click(); await sleep(60); sok(nRead(f) === 2, 'failing store: marks keep working'); f.remove();
  clearAll(); f = await load('gate.html?stub=late'); await sleep(200);
  dot(f, 0).click(); await sleep(60); sok(nRead(f) === 1, 'late store: a mark before the services are ready is applied at once');
  await sleep(4500); sok(stored(f) === 1, 'late store: it is saved when they are ready (' + stored(f) + ')'); f.remove();
  clearAll(); f = await load('gate.html?nostub'); await sleep(600);
  dot(f, 0).click(); await sleep(60); sok(nRead(f) === 1, 'no store: the mark works');
  sok(Object.keys(f.contentWindow.localStorage).some(function (k) { return /:marks$/.test(k); }), 'no store: the mark is kept in the browser'); f.remove();
  clearAll(); f = await load('questions.html'); await sleep(900);
  var qd = f.contentDocument, qi = qd.querySelectorAll('#q-Q-2 input');
  qi[0].click(); await sleep(1500); qi[1].click(); await sleep(60);
  sok(qd.querySelectorAll('#q-Q-2 input:checked').length === 2, 'questions: a second choice after the first save is applied');
  var qn = qd.getElementById('q-Q-3-note'); qn.value = 'one'; qn.dispatchEvent(new Event('input', { bubbles: true })); await sleep(2800); qn.value = 'one two'; qn.dispatchEvent(new Event('input', { bubbles: true })); await sleep(2800);
  var qs = f.contentWindow.__bfstub.dump()['answers/Q-3']; sok(qs && qs.note === 'one two', 'questions: a note typed after the first save is saved (' + JSON.stringify(qs && qs.note) + ')');
  sok(!(f.contentWindow.__bfErrors || []).length, 'questions: no script error: ' + (f.contentWindow.__bfErrors || [])[0]); f.remove();
  clearAll(); f = await load('gate.html'); await sleep(900);
  var gd = f.contentDocument, rw = gd.querySelectorAll('.row')[1]; rw.querySelector('.twist').click();
  [].filter.call(rw.querySelectorAll('button'), function (b) { return b.textContent === 'Add a note'; })[0].click();
  var gn = rw.querySelector('textarea'); gn.value = 'a'; gn.dispatchEvent(new Event('input', { bubbles: true })); await sleep(3200);
  gn.value = 'ab'; gn.dispatchEvent(new Event('input', { bubbles: true })); await sleep(3200);
  var nk = Object.keys(f.contentWindow.__bfstub.dump()).filter(function (x) { return /notes-/.test(x); })[0];
  sok(nk && Object.values(f.contentWindow.__bfstub.dump()[nk].notes)[0] === 'ab', 'row note: a note edited after the first save is saved');
  sok(!(f.contentWindow.__bfErrors || []).length, 'row note: no script error: ' + (f.contentWindow.__bfErrors || [])[0]); f.remove();
  clearAll();
  if (sfails.length) { bad++; lines.push('FAIL  saved data\n      ' + sfails.join('\n      ')); } else lines.push('pass  saved data (ok, reload, slow, failing, late, none)');
  out.textContent = lines.join('\n') + '\n' + (bad ? bad + ' page(s) failed' : 'all passed');
  document.body.setAttribute('data-status', bad ? 'fail' : 'pass');
})();
</script>"""


def browser_check(out: Path) -> int | None:
    """Runs _selftest.html in a headless Chrome if one is installed. Returns None when there is no browser."""
    import shutil, subprocess, threading, http.server, functools, re
    exe = next((c for c in ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", shutil.which("google-chrome"), shutil.which("chromium"), shutil.which("chrome")] if c and Path(c).exists()), None)
    if not exe:
        return None
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a, **k):
            pass

    handler = functools.partial(Quiet, directory=str(out))
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        url = f"http://127.0.0.1:{srv.server_address[1]}/_selftest.html"
        r = subprocess.run([exe, "--headless=new", "--disable-gpu", "--no-sandbox", "--virtual-time-budget=90000", "--dump-dom", url], capture_output=True, text=True, timeout=180)
    except Exception as e:  # noqa: BLE001
        print("browser self test could not run:", e)
        return None
    finally:
        srv.shutdown()
    m = re.search(r'data-status="(pass|fail)"', r.stdout)
    body = re.search(r'<pre id="out">(.*?)</pre>', r.stdout, re.S)
    text = (body.group(1) if body else "").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
    print(text.strip())
    return 0 if m and m.group(1) == "pass" else 1


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
    br = browser_check(out)
    if br is None:
        print("browser self test not run (no Chrome found): open _selftest.html from the preview server and read the result")
    elif br:
        bad += 1
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
    names = [n for n, _, _ in links]
    (out / "_selftest.html").write_text(SELFTEST_PAGE.replace("__NAMES__", json.dumps(names)))
    print("wrote", out / "_selftest.html")
    return check(out) if a.check else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
