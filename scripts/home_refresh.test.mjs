// The home page's live refresh, run against a stub page: `node --check` only parses the
// script, and whether a refresh counts as failed is behavior. The stub answers every call
// the script makes at load with an inert stand-in, and gives real stand-ins only for what
// a refresh reads and writes: the swapped parts, the live label, fetch and the stream.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const source = readFileSync(
  fileURLToPath(new URL('../crates/astrolabe/src/home/script.js', import.meta.url)),
  'utf8',
);

const STALE = 'Couldn’t refresh · showing older content';

// Anything the script asks of it is another inert stand-in; as text it is empty.
function inert() {
  const target = function () {};
  return new Proxy(target, {
    get(_, key) {
      if (key === Symbol.toPrimitive) return () => '';
      if (key === Symbol.iterator) return function* () {};
      if (key === 'length') return 0;
      if (key in target) return target[key];
      return (target[key] = inert());
    },
    set(_, key, value) {
      target[key] = value;
      return true;
    },
    apply: () => inert(),
    construct: () => inert(),
  });
}

// An element with real content and classes, inert in every other way.
function element(html) {
  const el = inert();
  const classes = new Set();
  el.innerHTML = html;
  el.scrollTop = 0;
  el.isConnected = true;
  el.classList = {
    add: (c) => classes.add(c),
    remove: (c) => classes.delete(c),
    contains: (c) => classes.has(c),
    toggle: (c, on) => (on ? classes.add(c) : classes.delete(c)),
  };
  return el;
}

// A page showing the parts in `shown`, loaded with the script. `answer` is what the next
// fetch resolves to: a status and the parts the returned page holds.
function page(shown) {
  const parts = Object.fromEntries(Object.entries(shown).map(([sel, html]) => [sel, element(html)]));
  const live = element('');
  const streams = [];
  const state = { answer: null, fetched: 0 };
  const store = () => ({ getItem: () => null, setItem() {} });
  const context = {
    document: Object.assign(inert(), {
      documentElement: element(''),
      activeElement: null,
      querySelector(sel) {
        if (sel === '#live') return live;
        if (['#tree', '#board', '.note'].includes(sel)) return parts[sel] || null;
        return element('');
      },
      querySelectorAll: () => [],
      getElementById: () => null,
      addEventListener() {},
    }),
    location: { pathname: '/', hash: '', protocol: 'http:', origin: 'http://127.0.0.1' },
    localStorage: store(),
    sessionStorage: store(),
    matchMedia: () => element(''),
    setInterval() {},
    setTimeout() {},
    clearTimeout() {},
    clearInterval() {},
    requestAnimationFrame() {},
    getComputedStyle: () => inert(),
    XMLSerializer: class {
      serializeToString() {
        return '';
      }
    },
    addEventListener() {},
    history: inert(),
    navigator: inert(),
    fetch() {
      state.fetched += 1;
      const { status, parts: returned } = state.answer;
      return Promise.resolve({ ok: status === 200, text: () => Promise.resolve(returned) });
    },
    // The parsed page is the object the fetch answered with: selector to content.
    DOMParser: class {
      parseFromString(returned) {
        return {
          querySelector: (sel) => (sel in returned ? { innerHTML: returned[sel] } : null),
        };
      }
    },
    EventSource: class {
      constructor() {
        this.listeners = {};
        streams.push(this);
      }
      addEventListener(name, fn) {
        this.listeners[name] = fn;
      }
    },
    Date,
    JSON,
    Math,
    Number,
    Array,
    Promise,
    Error,
    URL,
    encodeURIComponent,
    decodeURIComponent,
    console,
  };
  context.window = context;
  vm.runInNewContext(source, context);
  assert.equal(streams.length, 1, 'the script opened its event stream');
  const stream = streams[0];
  stream.onopen();
  // One change on the stream, and the refresh it starts run to its end.
  const change = async (answer) => {
    state.answer = answer;
    stream.listeners.changed();
    for (let i = 0; i < 20; i += 1) await new Promise((done) => setImmediate(done));
  };
  return { parts, live, change, state };
}

test('a good refresh swaps the parts and says live', async () => {
  const p = page({ '#tree': 'old tree', '#board': 'old board' });
  await p.change({ status: 200, parts: { '#tree': 'new tree', '#board': 'new board' } });
  assert.equal(p.state.fetched, 1);
  assert.equal(p.parts['#tree'].innerHTML, 'new tree');
  assert.equal(p.parts['#board'].innerHTML, 'new board');
  assert.equal(p.live.textContent, 'Live updates');
  assert.equal(p.live.classList.contains('stale'), false);
});

test('a failed status marks the page stale', async () => {
  const p = page({ '#tree': 'old tree', '#board': 'old board' });
  await p.change({ status: 500, parts: {} });
  assert.equal(p.live.textContent, STALE);
  assert.equal(p.parts['#board'].innerHTML, 'old board');
});

test('a 200 without the parts this page shows is a failed refresh', async () => {
  for (const returned of [{}, { '#tree': 'new tree' }, { '#board': 'new board' }]) {
    const p = page({ '#tree': 'old tree', '#board': 'old board' });
    await p.change({ status: 200, parts: returned });
    assert.equal(p.live.textContent, STALE, JSON.stringify(returned));
    assert.equal(p.live.classList.contains('stale'), true);
    // Nothing is swapped from a page that isn't whole.
    assert.equal(p.parts['#tree'].innerHTML, 'old tree', JSON.stringify(returned));
    assert.equal(p.parts['#board'].innerHTML, 'old board', JSON.stringify(returned));
  }
  // A note's page needs its note back, not a board.
  const note = page({ '#tree': 'old tree', '.note': 'old note' });
  await note.change({ status: 200, parts: { '#tree': 'new tree', '#board': 'a board' } });
  assert.equal(note.live.textContent, STALE);
  assert.equal(note.parts['.note'].innerHTML, 'old note');
});

test('the warning stays until a later refresh succeeds, with no retry of its own', async () => {
  const p = page({ '#tree': 'old tree', '#board': 'old board' });
  await p.change({ status: 200, parts: {} });
  assert.equal(p.live.textContent, STALE);
  assert.equal(p.state.fetched, 1, 'no timer retried');
  await p.change({ status: 200, parts: { '#tree': 'new tree', '#board': 'new board' } });
  assert.equal(p.state.fetched, 2);
  assert.equal(p.live.textContent, 'Live updates');
  assert.equal(p.parts['#board'].innerHTML, 'new board');
});
