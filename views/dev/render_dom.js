#!/usr/bin/env node
/* Run the page template over one sample with a small DOM of its own and print the drawn tree as JSON.
 *
 *   node views/dev/render_dom.js TEMPLATE.html SAMPLE.json
 *
 * It is the engine-side stand-in for a browser, so the accessibility test (crates/bf/tests/page_accessibility.rs) can
 * look at what the template draws without one. Every row and section is opened first, so the whole page is drawn.
 * It draws no pixels: contrast is measured from the tokens file by the test. Standard library only. */
'use strict';
const fs = require('fs');
const vm = require('vm');

const [tplPath, samplePath] = process.argv.slice(2);
const tpl = fs.readFileSync(tplPath, 'utf8');
const data = fs.readFileSync(samplePath, 'utf8');

const byId = {};
class Node_ {
  constructor() { this.childNodes = []; this.parentNode = null; }
  get lastChild() { return this.childNodes[this.childNodes.length - 1] || null; }
}
class Text extends Node_ {
  constructor(t) { super(); this.nodeType = 3; this.data = t; }
  get textContent() { return this.data; }
}
class Elem extends Node_ {
  constructor(tag) {
    super(); this.nodeType = 1; this.tagName = tag.toUpperCase(); this.attrs = {}; this.listeners = {}; this.raw = null;
    this.style = new Proxy({}, { get: (t, k) => t[k] || '', set: (t, k, v) => { t[k] = v; return true; } });
    this.value = ''; this.open = false;
    const self = this;
    this.classList = {
      add(...c) { const s = new Set(self.cls()); c.forEach(x => s.add(x)); self.attrs['class'] = [...s].join(' '); },
      remove(...c) { self.attrs['class'] = self.cls().filter(x => !c.includes(x)).join(' '); },
      contains(c) { return self.cls().includes(c); },
      toggle(c, f) { const has = self.cls().includes(c); const want = f === undefined ? !has : !!f; if (want) this.add(c); else this.remove(c); return want; },
    };
  }
  cls() { return (this.attrs['class'] || '').split(/\s+/).filter(Boolean); }
  get className() { return this.attrs['class'] || ''; }
  set className(v) { this.attrs['class'] = String(v); }
  get id() { return this.attrs.id || ''; }
  set id(v) { this.setAttribute('id', v); }
  get hidden() { return 'hidden' in this.attrs; }
  set hidden(v) { if (v) this.attrs.hidden = ''; else delete this.attrs.hidden; }
  get children() { return this.childNodes.filter(n => n.nodeType === 1); }
  get textContent() { return this.childNodes.map(n => n.textContent).join(''); }
  set textContent(v) { this.childNodes = []; if (v !== '' && v != null) this.appendChild(new Text(String(v))); }
  set innerHTML(v) { this.childNodes = []; this.raw = String(v); }
  get innerHTML() { return this.raw || ''; }
  setAttribute(k, v) { this.attrs[k] = String(v); if (k === 'id') byId[String(v)] = this; }
  getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
  removeAttribute(k) { delete this.attrs[k]; }
  hasAttribute(k) { return k in this.attrs; }
  appendChild(c) { if (c.parentNode) c.parentNode.removeChild(c); c.parentNode = this; this.childNodes.push(c); return c; }
  removeChild(c) { this.childNodes = this.childNodes.filter(x => x !== c); c.parentNode = null; return c; }
  replaceChild(n, o) { const i = this.childNodes.indexOf(o); if (i < 0) return o; if (n.parentNode) n.parentNode.removeChild(n); n.parentNode = this; this.childNodes[this.childNodes.indexOf(o)] = n; o.parentNode = null; return o; }
  remove() { if (this.parentNode) this.parentNode.removeChild(this); }
  after(n) { const p = this.parentNode; if (!p) return; const i = p.childNodes.indexOf(this); if (n.parentNode) n.parentNode.removeChild(n); n.parentNode = p; p.childNodes.splice(p.childNodes.indexOf(this) + 1, 0, n); }
  contains(n) { for (let x = n; x; x = x.parentNode) if (x === this) return true; return false; }
  addEventListener(t, f) { (this.listeners[t] = this.listeners[t] || []).push(f); }
  dispatch(t) { (this.listeners[t] || []).forEach(f => f({ type: t, target: this, preventDefault() {}, stopPropagation() {} })); }
  focus() { doc.activeElement = this; }
  blur() {}
  scrollIntoView() {}
  select() {}
  closest(sel) { for (let x = this; x && x.nodeType === 1; x = x.parentNode) if (x.matches(sel)) return x; return null; }
  matches(sel) { return sel.split(',').some(s => matchCompound(this, s.trim())); }
  querySelectorAll(sel) {
    const out = [];
    const parts = sel.split(',').map(s => s.trim());
    const walk = n => n.childNodes.forEach(c => { if (c.nodeType === 1) { if (parts.some(p => matchDesc(c, p, this))) out.push(c); walk(c); } });
    walk(this);
    return out;
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}
function matchCompound(e, s) {
  const re = /(\.[\w-]+|#[\w-]+|\[[^\]]+\]|^[a-zA-Z][\w-]*)/g; let m, ok = true, any = false;
  while ((m = re.exec(s))) {
    any = true; const t = m[1];
    if (t[0] === '.') ok = ok && e.cls().includes(t.slice(1));
    else if (t[0] === '#') ok = ok && e.id === t.slice(1);
    else if (t[0] === '[') { const a = /\[([\w-]+)(?:="([^"]*)")?\]/.exec(t); ok = ok && (a[2] === undefined ? a[1] in e.attrs : e.attrs[a[1]] === a[2]); }
    else ok = ok && e.tagName === t.toUpperCase();
  }
  return any && ok;
}
function matchDesc(e, sel, root) {
  const parts = sel.split(/\s+/);
  if (!matchCompound(e, parts[parts.length - 1])) return false;
  let i = parts.length - 2, x = e.parentNode;
  while (i >= 0 && x && x !== root.parentNode) { if (x.nodeType === 1 && matchCompound(x, parts[i])) i--; x = x.parentNode; }
  return i < 0;
}

const doc = new Elem('#document');
doc.activeElement = null;
doc.title = '';
doc.createElement = t => new Elem(t);
doc.createTextNode = t => new Text(String(t));
doc.getElementById = id => byId[id] || null;
doc.querySelectorAll = s => doc.body.querySelectorAll(s);
doc.querySelector = s => doc.body.querySelector(s);
doc.body = new Elem('body');
doc.appendChild(doc.body);

const app = new Elem('div'); app.setAttribute('id', 'app'); doc.body.appendChild(app);
const boot = new Elem('div'); boot.setAttribute('id', 'boot'); app.appendChild(boot);
const toast = new Elem('div'); toast.setAttribute('id', 'toast'); toast.hidden = true; doc.body.appendChild(toast);
const bfdata = new Elem('script'); bfdata.setAttribute('id', 'bf-data');
// the same replacement `bf render` makes: the data at the marker, with a closing script tag made harmless
bfdata.childNodes = [Object.assign(new Text(data.replace(/<\//g, '<\\/')), { parentNode: bfdata })];
byId['bf-data'] = bfdata;

const store = {};
const win = {
  addEventListener() {}, matchMedia: () => ({ matches: false, addEventListener() {}, addListener() {} }), __bfErrors: [],
  localStorage: { getItem: k => (k in store ? store[k] : null), setItem: (k, v) => { store[k] = String(v); }, removeItem: k => { delete store[k]; } },
};
const sandbox = {
  document: doc, window: win, localStorage: win.localStorage, navigator: { userAgent: 'bf-dom' }, history: { replaceState() {} },
  location: { hash: '', search: '', pathname: '/' }, console, setTimeout: () => 0, clearTimeout() {}, setInterval: () => 0, clearInterval() {},
  Promise, JSON, Date, Math, Object, Array, String, Number, RegExp, Set, Map, encodeURIComponent, decodeURIComponent, isNaN, parseInt, parseFloat, Error,
  getComputedStyle: () => ({}), requestAnimationFrame: () => 0, IntersectionObserver: undefined,
};
sandbox.window.document = doc; sandbox.window.location = sandbox.location; sandbox.window.history = sandbox.history;
vm.createContext(sandbox);

const scripts = [...tpl.matchAll(/<script>\n([\s\S]*?)<\/script>/g)];
if (!scripts.length) { console.error('no script in the template'); process.exit(2); }
for (const s of scripts) vm.runInContext(s[1], sandbox, { filename: 'page.html' });

// open every section and every row, so the whole page is drawn
const all = [];
(function walk(n) { n.childNodes.forEach(c => { if (c.nodeType === 1) { all.push(c); walk(c); } }); })(doc.body);
all.filter(e => e.cls().includes('twist')).forEach(e => { if (e.getAttribute('aria-expanded') === 'false') e.dispatch('click'); });

function dump(n) {
  if (n.nodeType === 3) return { text: n.data };
  const o = { tag: n.tagName.toLowerCase(), attrs: n.attrs, kids: n.childNodes.map(dump) };
  if (n.raw) o.raw = n.raw;
  return o;
}
process.stdout.write(JSON.stringify({ title: doc.title, errors: win.__bfErrors, tree: dump(doc.body) }));
