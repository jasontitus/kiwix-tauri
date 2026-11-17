import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

// Execute the production bridge the way an archive document does: synchronously
// in the document head, before the body is parsed. The served `data-reader-*`
// attributes are the only appearance input at that point; the shell's message
// cannot arrive until after `ready`. These assertions therefore describe the
// first paint, not a later repaint.
const bridge = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const parentOrigin = "http://shell.localhost:8123";

function reader(dataset, originalFont = "", annotations = false, nested = false) {
  const listeners = new Map();
  const observers = [];
  const resizers = [];
  const messages = [];
  const highlights = new Map();
  const timers = new Map();
  let nextTimer = 0;
  let highlightAllocations = 0;
  const text = { nodeType: 3, nodeValue: "", parentElement: null };
  const body = {
    nodeType: 1, tagName: "BODY", style: {}, children: [],
    matches: () => false,
    querySelectorAll: () => [],
    contains: (node) => node === text || node === body,
  };
  text.parentElement = body;
  const head = {
    children: [],
    append(node) {
      this.children.push(node);
      node.isConnected = true;
    },
    appendChild(node) {
      this.append(node);
      return node;
    },
  };
  const root = {
    nodeType: 1,
    tagName: "HTML",
    style: { fontFamily: originalFont },
    dataset: {},
    querySelectorAll: () => [],
    appendChild(node) {
      head.append(node);
      return node;
    },
  };
  const document = {
    currentScript: { dataset: { parentOrigin, ...dataset } },
    readyState: "loading",
    documentElement: root,
    head,
    body: null,
    title: "Archive",
    hidden: false,
    createElement(tag) {
      return {
        tagName: tag.toUpperCase(),
        dataset: {},
        textContent: "",
        isConnected: false,
        remove() {
          this.isConnected = false;
        },
      };
    },
    createTreeWalker() {
      let pending = text.nodeValue ? text : null;
      return { nextNode() { const node = pending; pending = null; return node; } };
    },
    createRange() {
      let start = 0;
      let end = 0;
      return {
        setStart(_node, offset) { start = offset; },
        setEnd(_node, offset) { end = offset; },
        toString: () => text.nodeValue.slice(start, end),
      };
    },
    getElementById: () => null,
    getElementsByName: () => [],
    querySelectorAll: () => [],
    addEventListener(name, callback) {
      const callbacks = listeners.get(name) ?? [];
      callbacks.push(callback);
      listeners.set(name, callbacks);
    },
    removeEventListener() {},
  };
  class Highlight {
    constructor(...ranges) {
      highlightAllocations++;
      this.ranges = ranges;
    }
  }
  const window = {
    parent: {
      postMessage(message) { messages.push(message); },
      ...(nested ? { location: { origin: "http://archive.localhost:8123" } } : {}),
    },
    CSS: { highlights },
    Highlight,
    getSelection: () => null,
    addEventListener(name, callback) {
      const callbacks = listeners.get(name) ?? [];
      callbacks.push(callback);
      listeners.set(name, callbacks);
    },
    removeEventListener() {},
  };
  const context = vm.createContext({
    window,
    CSS: window.CSS,
    Highlight,
    parent: window.parent,
    document,
    location: {
      origin: "http://archive.localhost:8123",
      href: "http://archive.localhost:8123/index.html",
    },
    navigator: {},
    history: { pushState() {}, replaceState() {} },
    MutationObserver: class {
      constructor(callback) { this.callback = callback; observers.push(this); }
      observe(target, options) { this.target = target; this.options = options; this.active = true; }
      disconnect() { this.active = false; }
    },
    ResizeObserver: class {
      constructor(callback) { this.callback = callback; resizers.push(this); }
      observe(target) { this.target = target; this.active = true; }
      disconnect() { this.active = false; }
    },
    EventTarget: class {},
    Event: class {},
    MouseEvent: class {},
    WheelEvent: class {},
    DOMException,
    crypto: { randomUUID },
    URL,
    HTMLAnchorElement: class { get href() { return this.url; } },
    HTMLAreaElement: class { get href() { return this.url; } },
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    NodeFilter: { SHOW_ELEMENT: 1, SHOW_TEXT: 4, FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3 },
    WeakRef,
    WeakSet,
    WeakMap,
    getComputedStyle: () => ({
      backgroundColor: "rgba(0, 0, 0, 0)",
      backgroundImage: "none",
      color: "rgb(0, 0, 0)",
    }),
    performance: { now: () => 0, getEntriesByType: () => [] },
    setTimeout: annotations ? (callback) => { timers.set(++nextTimer, callback); return nextTimer; } : setTimeout,
    clearTimeout: annotations ? (id) => timers.delete(id) : clearTimeout,
    queueMicrotask,
    requestAnimationFrame() {},
    cancelAnimationFrame() {},
  });
  Object.defineProperty(context.MouseEvent.prototype, "ctrlKey", { get() { return false; } });
  Object.defineProperty(context.WheelEvent.prototype, "deltaY", { get() { return 0; } });
  vm.runInContext(bridge, context, { filename: "reader-bridge.js" });
  function receive(data) {
    for (const callback of listeners.get("message") ?? [])
      callback({ source: window.parent, origin: parentOrigin, isTrusted: true,
        data: { channel: "kiwix-reader", version: 1, sessionId: "theme-session", ...data } });
  }
  function emit(name) {
    for (const callback of listeners.get(name) ?? []) callback({});
  }
  const textObservers = () => observers.filter((observer) => observer.active && observer.options.characterData);
  return { root, head, listeners, document, messages, highlights,
    get highlightAllocations() { return highlightAllocations; },
    get activeTextObservers() { return textObservers().length; },
    get activeResizers() { return resizers.filter((observer) => observer.active).length; },
    start(content = "") {
      receive({ type: "reader-init" });
      text.nodeValue = content;
      document.body = body;
      document.readyState = "complete";
      emit("DOMContentLoaded");
    },
    command(message) { receive({ type: "reader-command", ...message }); },
    readyAgain() { receive({ type: "reader-init" }); },
    pagehide() { emit("pagehide"); },
    mutate(content) {
      text.nodeValue = content;
      for (const observer of textObservers())
        observer.callback([{ type: "characterData", target: text, addedNodes: [], removedNodes: [] }]);
    },
    flushTimers() {
      for (let budget = 100; timers.size && budget > 0; budget--) {
        const callbacks = [...timers.values()];
        timers.clear();
        for (const callback of callbacks) callback();
      }
      assert.equal(timers.size, 0, "bounded consumer timers settle");
    },
    navigate(url) {
      context.location.href = url;
      emit("popstate");
    },
    original() {
      receive({ type: "reader-init" });
      receive({ type: "reader-command", command: "appearance", theme: "original",
        fontFamily: "original", fontSize: 16 });
    } };
}

test("a served dark theme is applied while the body is still unparsed", () => {
  const frame = reader({ readerTheme: "dark" });
  // First paint: no body exists yet, and the document is already themed.
  assert.equal(frame.root.dataset.kiwixTheme, "dark");
  assert.equal(frame.root.style.colorScheme, "dark");
  assert.equal(frame.root.style.backgroundColor, "#171a1e");
  assert.equal(frame.head.children.length, 1, "one scoped theme stylesheet");
  const css = frame.head.children[0].textContent;
  assert.match(css, /:root\[data-kiwix-theme="dark"\]\{--kiwix-theme-bg:#171a1e;/);
  assert.match(
    css,
    /:root\[data-kiwix-theme\] body\{background-color:var\(--kiwix-theme-bg\)!important;color:var\(--kiwix-theme-text\)!important;\}/,
  );
});

test("the boot stylesheet stays theme-scoped and never restyles archive colours", () => {
  const css = reader({ readerTheme: "sepia" }).head.children[0].textContent;
  for (const rule of css.split("}")) {
    if (!rule.includes("body")) continue;
    assert.ok(
      rule.trimStart().startsWith(":root[data-kiwix-theme]"),
      `unscoped body rule: ${rule}`,
    );
  }
  assert.match(css, /:root\[data-kiwix-theme="sepia"\]\{--kiwix-theme-bg:#f4ecd8;/);
});

test("an original theme leaves the archive's own colours untouched", () => {
  const frame = reader({});
  assert.equal(frame.root.dataset.kiwixTheme, undefined);
  assert.equal(frame.root.style.backgroundColor, undefined);
  assert.equal(frame.root.style.colorScheme, undefined);
  assert.equal(frame.head.children.length, 0);
});

test("served type is applied before the body paints and out-of-range sizes are ignored", () => {
  const frame = reader({
    readerTheme: "light",
    readerFontFamily: "serif",
    readerFontSize: "21",
  });
  assert.equal(frame.root.style.fontFamily, "serif");
  assert.equal(frame.root.style.fontSize, "21px");
  const rejected = reader({ readerFontSize: "99", readerFontFamily: "comic" });
  assert.equal(rejected.root.style.fontSize, undefined);
  assert.equal(rejected.root.style.fontFamily, "");
});

for (const family of ["serif", "sans"]) {
  for (const originalFont of ["", "Georgia, serif"]) {
    test(`fresh original-color ${family} boot restores archive font ${originalFont || "(stylesheet)"}`, () => {
      const frame = reader({ readerFontFamily: family }, originalFont);
      assert.equal(frame.root.style.fontFamily, family === "sans" ? "sans-serif" : "serif");
      frame.original();
      assert.equal(frame.root.style.fontFamily, originalFont);
    });
  }
}

test("font-only prepaint does not capture unparsed body colours as archive originals", () => {
  const frame = reader({ readerFontFamily: "serif" }, "Georgia");
  frame.document.body = { style: { backgroundColor: "navy", color: "white" } };
  frame.original();
  assert.equal(frame.root.style.fontFamily, "Georgia");
  assert.equal(frame.document.body.style.backgroundColor, "navy");
  assert.equal(frame.document.body.style.color, "white");
});

const widthScopes = (frame) =>
  frame.head.children.filter((node) => node.isConnected && node.dataset.kiwixWidthUi !== undefined);

test("comfortable and full widths update one scoped measure stylesheet", () => {
  const frame = reader({});
  frame.original();
  frame.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16, width: "comfortable" });
  assert.equal(frame.root.dataset.kiwixWidth, "comfortable");
  let styles = widthScopes(frame);
  assert.equal(styles.length, 1, "one width stylesheet");
  assert.match(styles[0].textContent,
    /:root\[data-kiwix-width="comfortable"\] body\{max-width:52rem!important;margin-left:auto!important;margin-right:auto!important;\}/);
  assert.match(styles[0].textContent,
    /:root\[data-kiwix-width="comfortable"\] pre\{max-width:100%;\}/);
  frame.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16, width: "full" });
  assert.equal(frame.root.dataset.kiwixWidth, "full");
  styles = widthScopes(frame);
  assert.equal(styles.length, 1, "the stylesheet is replaced, not stacked");
  assert.match(styles[0].textContent,
    /:root\[data-kiwix-width="full"\] main\{max-width:none!important;\}/);
  assert.match(styles[0].textContent,
    /:root\[data-kiwix-width="full"\] body\{max-width:none!important;\}/);
  for (const wrapper of ["#page", ".container", "article"])
    assert.ok(
      styles[0].textContent.includes(`:root[data-kiwix-width="full"] ${wrapper}`),
      `full width releases ${wrapper}`,
    );
});

test("original width removes the attribute and its stylesheet", () => {
  const frame = reader({});
  frame.original();
  frame.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16, width: "comfortable" });
  frame.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16, width: "original" });
  assert.equal(frame.root.dataset.kiwixWidth, undefined);
  assert.equal(widthScopes(frame).length, 0);
  const untouched = reader({});
  untouched.original();
  untouched.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16 });
  assert.equal(untouched.root.dataset.kiwixWidth, undefined, "absent width keeps the archive layout");
  assert.equal(widthScopes(untouched).length, 0);
});

test("media-only documents never receive the reading-column stylesheet", () => {
  const frame = reader({ entryMime: "video/mp4" });
  frame.original();
  frame.command({ command: "appearance", theme: "original", fontFamily: "original",
    fontSize: 16, width: "comfortable" });
  assert.equal(frame.root.dataset.kiwixWidth, undefined);
  assert.equal(widthScopes(frame).length, 0);
});

for (const entryMime of ["application/xhtml+xml", "text/plain", "application/json", "application/xml"]) {
  test(`${entryMime} documents receive the reading-column stylesheet`, () => {
    const frame = reader({ entryMime });
    frame.original();
    frame.command({ command: "appearance", theme: "original", fontFamily: "original",
      fontSize: 16, width: "comfortable" });
    assert.equal(frame.root.dataset.kiwixWidth, "comfortable");
    assert.equal(widthScopes(frame).length, 1);
  });
}

for (const mime of ["application/pdf", "image/svg+xml"]) {
  test(`${mime} documents stay as authored`, () => {
    const frame = reader({ entryMime: mime });
    frame.original();
    frame.command({ command: "appearance", theme: "original", fontFamily: "original",
      fontSize: 16, width: "full" });
    assert.equal(frame.root.dataset.kiwixWidth, undefined);
    assert.equal(widthScopes(frame).length, 0);
  });
}

test("served width is applied before the body paints, with no appearance message", () => {
  for (const width of ["comfortable", "full"]) {
    const frame = reader({ readerWidth: width });
    assert.equal(frame.document.readyState, "loading");
    assert.equal(frame.messages.length, 1, "only the private reading-gesture hello is posted");
    assert.equal(frame.messages[0].type, "reading-gesture-hello");
    assert.match(frame.messages[0].token, /^[0-9a-f-]{36}$/);
    assert.equal(frame.root.dataset.kiwixWidth, width);
    const styles = widthScopes(frame);
    assert.equal(styles.length, 1, "width stylesheet exists at first paint");
    assert.match(styles[0].textContent, new RegExp(`:root\\[data-kiwix-width="${width}"\\] body\\{`));
  }
  const rejected = reader({ readerWidth: "wide" });
  assert.equal(rejected.root.dataset.kiwixWidth, undefined);
  assert.equal(widthScopes(rejected).length, 0);
});

test("served width covers xhtml and text documents before first paint", () => {
  for (const entryMime of ["application/xhtml+xml", "text/plain"]) {
    const frame = reader({ readerWidth: "comfortable", entryMime });
    assert.equal(frame.root.dataset.kiwixWidth, "comfortable");
    assert.equal(widthScopes(frame).length, 1);
  }
});

test("nested same-origin archive frames never receive the reading-width stylesheet", () => {
  const frame = reader({ readerWidth: "comfortable", entryMime: "text/html" }, "", false, true);
  assert.equal(frame.root.dataset.kiwixWidth, undefined);
  assert.equal(widthScopes(frame).length, 0);
});

const annotation = {
  id: "00000000-0000-4000-8000-000000000001",
  bookmarkId: "00000000-0000-4000-8000-000000000002",
  documentPath: "index.html",
  exact: "Animal",
  prefix: "",
  suffix: "",
  start: 0,
  end: 6,
};
const articleUrl = "http://archive.localhost:8123/index.html";

function sendAnnotations(frame, items) {
  frame.command({ command: "highlights-begin", url: articleUrl, documentPath: "index.html" });
  frame.command({ command: "highlights", url: articleUrl, highlights: items });
  frame.command({ command: "highlights-end", url: articleUrl });
}

test("an empty document highlight stream allocates no annotation observers or paint", () => {
  const frame = reader({}, "", true);
  assert.equal(frame.activeTextObservers, 0, "no parser mutation observation");
  frame.start("Animal");
  sendAnnotations(frame, []);
  assert.equal(frame.activeTextObservers, 0);
  assert.equal(frame.activeResizers, 0);
  assert.equal(frame.highlightAllocations, 0);
  assert.equal(frame.head.children.filter((node) => node.isConnected).length, 0);
});

test("accepted highlights retain mutation revalidation and clear stale paint immediately", () => {
  const frame = reader({}, "", true);
  frame.start("Animal");
  sendAnnotations(frame, [annotation]);
  assert.equal(frame.activeTextObservers, 1);
  assert.equal(frame.activeResizers, 1);
  assert.equal(frame.highlights.get("kiwix-passages").ranges.length, 1);
  frame.mutate("Changed text");
  assert.equal(frame.highlights.has("kiwix-passages"), false, "stale range is not painted");
  frame.flushTimers();
  assert.equal(frame.highlights.has("kiwix-passages"), false);
  assert.equal(frame.activeTextObservers, 1, "unresolved annotations still observe content");
  frame.mutate("Animal returns");
  frame.flushTimers();
  assert.equal(frame.highlights.get("kiwix-passages").ranges.length, 1);
});

test("annotation replacement assembles chunks before observing and empty replacement releases paint", () => {
  const frame = reader({}, "", true);
  frame.start("Animal");
  sendAnnotations(frame, [annotation]);
  frame.command({ command: "highlights-begin", url: articleUrl, documentPath: "index.html" });
  assert.equal(frame.activeTextObservers, 0);
  assert.equal(frame.activeResizers, 0);
  assert.equal(frame.highlights.has("kiwix-passages"), false);
  frame.command({ command: "highlights", url: articleUrl, highlights: [annotation] });
  assert.equal(frame.activeTextObservers, 0, "do not observe a partial stream");
  frame.command({ command: "highlights-end", url: articleUrl });
  assert.equal(frame.activeTextObservers, 1);
  const allocations = frame.highlightAllocations;
  sendAnnotations(frame, []);
  assert.equal(frame.activeTextObservers, 0);
  assert.equal(frame.activeResizers, 0);
  assert.equal(frame.highlights.has("kiwix-passages"), false);
  assert.equal(frame.highlightAllocations, allocations, "clearing does not allocate an empty Highlight");
  assert.equal(frame.head.children.filter((node) => node.isConnected).length, 0);
});

test("unavailable accepted quotes observe later content but invalid streams do not", () => {
  const frame = reader({}, "", true);
  frame.start("");
  frame.command({ command: "highlights", url: articleUrl, highlights: [annotation] });
  frame.command({ command: "highlights-end", url: articleUrl });
  assert.equal(frame.activeTextObservers, 0);
  sendAnnotations(frame, [annotation]);
  assert.equal(frame.activeTextObservers, 1);
  assert.equal(frame.highlightAllocations, 0);
  frame.mutate("Animal");
  frame.flushTimers();
  assert.equal(frame.highlights.get("kiwix-passages").ranges.length, 1);
});

test("pagehide suspends annotation observation and ready reconnects only retained annotations", () => {
  const frame = reader({}, "", true);
  frame.start("Animal");
  sendAnnotations(frame, [annotation]);
  frame.pagehide();
  assert.equal(frame.activeTextObservers, 0);
  assert.equal(frame.activeResizers, 0);
  frame.readyAgain();
  assert.equal(frame.activeTextObservers, 1);
  frame.navigate(`${articleUrl}#next`);
  assert.equal(frame.activeTextObservers, 0, "navigation drops old document observation");
  assert.equal(frame.activeResizers, 0);
  assert.equal(frame.highlights.has("kiwix-passages"), false);
  frame.readyAgain();
  assert.equal(frame.activeTextObservers, 0, "empty navigation state stays unobserved");
});
