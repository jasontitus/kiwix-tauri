import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

// Execute the production bridge in a minimal archive document. Speech engines
// are simulated at their public callback boundary: desktop Web Speech emits
// `onboundary`/`onstart`; native mobile speech posts `speech-text` and then
// absolute offsets through `speech-highlight`. The assertions are about the
// DOM paint and the spoken text, not about audible output.
const bridge = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const parentOrigin = "http://shell.localhost:8123";
const archiveOrigin = "http://archive.localhost:8123";
const articleUrl = `${archiveOrigin}/index.html`;
const sessionId = "read-aloud-session";
const requestId = randomUUID();
const FILTER_ACCEPT = 1;
const FILTER_REJECT = 2;
const FILTER_SKIP = 3;

function text(value, parent = null) {
  return {
    nodeType: 3,
    nodeValue: value,
    parentElement: parent,
    isConnected: parent ? parent.isConnected : true,
    get length() { return this.nodeValue.length; },
  };
}

function element(tagName, { parent = null, children = [], style = {}, attributes = {} } = {}) {
  const node = {
    nodeType: 1,
    tagName: tagName.toUpperCase(),
    localName: tagName.toLowerCase(),
    namespaceURI: "http://www.w3.org/1999/xhtml",
    style: { ...style },
    dataset: {},
    attributes: new Map(Object.entries(attributes)),
    children: [],
    parentElement: parent,
    isConnected: parent ? parent.isConnected : true,
    hidden: Object.prototype.hasOwnProperty.call(attributes, "hidden"),
    shadowRoot: null,
    listeners: new Map(),
    getAttribute(name) { return this.attributes.has(name) ? this.attributes.get(name) : null; },
    setAttribute(name, value) { this.attributes.set(name, String(value)); },
    hasAttribute(name) { return this.attributes.has(name); },
    removeAttribute(name) { this.attributes.delete(name); },
    matches(selector) {
      if (selector.includes("[data-kiwix-read-aloud-ui]") &&
          (this.dataset.kiwixReadAloudUi !== undefined ||
           this.hasAttribute("data-kiwix-read-aloud-ui"))) return true;
      if (selector.includes("[data-kiwix-annotation-ui]") &&
          (this.dataset.kiwixAnnotationUi !== undefined ||
           this.hasAttribute("data-kiwix-annotation-ui"))) return true;
      return false;
    },
    closest(selector) {
      for (let node = this; node; node = node.parentElement)
        if (node.nodeType === 1 && node.matches(selector)) return node;
      return null;
    },
    contains(candidate) {
      for (let current = candidate; current; current = current.parentElement)
        if (current === this) return true;
      return false;
    },
    append(...nodes) { for (const child of nodes) this.appendChild(child); },
    appendChild(child) {
      if (child.nodeType === 11) {
        for (const grandchild of [...child.children]) this.appendChild(grandchild);
        child.children = [];
        return child;
      }
      if (child.parentElement) {
        const siblings = child.parentElement.children;
        const index = siblings.indexOf(child);
        if (index >= 0) siblings.splice(index, 1);
      }
      this.children.push(child);
      child.parentElement = this;
      child.isConnected = this.isConnected;
      return child;
    },
    insertBefore(child, reference) {
      const index = reference ? this.children.indexOf(reference) : -1;
      if (index < 0) return this.appendChild(child);
      this.children.splice(index, 0, child);
      child.parentElement = this;
      child.isConnected = this.isConnected;
      return child;
    },
    replaceChildren(...nodes) {
      this.children = [];
      this.append(...nodes);
    },
    remove() {
      const siblings = this.parentElement?.children;
      const index = siblings ? siblings.indexOf(this) : -1;
      if (index >= 0) siblings.splice(index, 1);
      this.parentElement = null;
      this.isConnected = false;
    },
    attachShadow() {
      const root = element("#shadow");
      root.host = this;
      this.shadowRoot = root;
      return root;
    },
    querySelectorAll() { return []; },
    querySelector() { return null; },
    getBoundingClientRect() {
      const css = this.style.cssText || "";
      const left = Number.parseFloat(/left:([-\d.]+)px/.exec(css)?.[1] ?? this.style.left) || 0;
      const top = Number.parseFloat(/top:([-\d.]+)px/.exec(css)?.[1] ?? this.style.top) || 0;
      return { left, top, right: left, bottom: top, width: 0, height: 0 };
    },
    getClientRects() { return []; },
    addEventListener(type, callback) {
      const callbacks = this.listeners.get(type) ?? [];
      callbacks.push(callback);
      this.listeners.set(type, callbacks);
    },
    removeEventListener() {},
    dispatch(type, event = {}) {
      for (const callback of this.listeners.get(type) ?? []) callback(event);
    },
  };
  for (const child of children) node.appendChild(child);
  return node;
}

function descendants(root) {
  const nodes = [];
  const visit = (node) => {
    for (const child of node.children ?? []) {
      nodes.push(child);
      visit(child);
    }
  };
  visit(root);
  return nodes;
}

function textNodes(root) {
  return descendants(root).filter((node) => node.nodeType === 3);
}

function walker(root, whatToShow, filter) {
  // Lazy, like a real TreeWalker: filter side effects must interleave with
  // nextNode() in document order. Real Elements expose text only through
  // childNodes, so the mock must descend the same way.
  const children = (node) => node.childNodes ?? node.children ?? [];
  const stack = [...children(root)].reverse();
  let currentNode = root;
  return {
    get currentNode() { return currentNode; },
    nextNode() {
      while (stack.length) {
        const node = stack.pop();
        const elementNode = node.nodeType === 1;
        const textNode = node.nodeType === 3;
        const shown = (elementNode && (whatToShow & 1)) || (textNode && (whatToShow & 4));
        const decision = !shown ? FILTER_SKIP : filter ? filter.acceptNode(node) : FILTER_ACCEPT;
        if (decision === FILTER_REJECT) continue;
        if (elementNode)
          for (const child of [...children(node)].reverse()) stack.push(child);
        if (decision === FILTER_ACCEPT) {
          currentNode = node;
          return node;
        }
      }
      return null;
    },
  };
}

function range(document, rect) {
  return {
    startContainer: null,
    startOffset: 0,
    endContainer: null,
    endOffset: 0,
    rect: rect ?? { left: 10, top: 20, right: 110, bottom: 40, width: 100, height: 20 },
    setStart(node, offset) { this.startContainer = node; this.startOffset = offset; },
    setEnd(node, offset) { this.endContainer = node; this.endOffset = offset; },
    collapse() { this.endContainer = this.startContainer; this.endOffset = this.startOffset; },
    get collapsed() {
      return this.startContainer === this.endContainer && this.startOffset === this.endOffset;
    },
    getClientRects() { return [this.rect]; },
    getBoundingClientRect() { return this.rect; },
    toString() {
      const nodes = textNodes(document.body);
      const startIndex = nodes.indexOf(this.startContainer);
      const endIndex = nodes.indexOf(this.endContainer);
      if (startIndex < 0 || endIndex < 0) return "";
      let value = "";
      for (let index = startIndex; index <= endIndex; index++) {
        const node = nodes[index];
        const from = index === startIndex ? this.startOffset : 0;
        const to = index === endIndex ? this.endOffset : node.nodeValue.length;
        value += node.nodeValue.slice(from, to);
      }
      return value;
    },
    intersectsNode(node) {
      if (node.nodeType === 1) {
        return textNodes(node).some((child) => this.intersectsNode(child));
      }
      const nodes = textNodes(document.body);
      const startIndex = nodes.indexOf(this.startContainer);
      const endIndex = nodes.indexOf(this.endContainer);
      const index = nodes.indexOf(node);
      if (startIndex < 0 || endIndex < 0 || index < 0) return false;
      if (index < startIndex || index > endIndex) return false;
      if (startIndex === endIndex) return index === startIndex && this.startOffset < this.endOffset;
      if (index === startIndex) return this.startOffset < node.nodeValue.length;
      if (index === endIndex) return this.endOffset > 0;
      return true;
    },
    comparePoint(node, offset) {
      if (node === this.startContainer)
        return offset < this.startOffset ? -1 : offset > this.startOffset ? 1 : 0;
      const nodes = textNodes(document.body);
      const candidate = nodes.indexOf(node);
      const boundary = nodes.indexOf(this.startContainer);
      if (candidate < 0 || boundary < 0) return 0;
      return candidate < boundary ? -1 : candidate > boundary ? 1 : 0;
    },
    compareBoundaryPoints() { return 0; },
  };
}

function reader({ cssHighlights = true, nativeEvents = false } = {}) {
  const messages = [];
  const highlights = new Map();
  const utterances = [];
  const scrolls = [];
  const documentListeners = new Map();
  const windowListeners = new Map();
  const observers = [];
  const frames = new Map();
  const timers = new Map();
  let nextTimer = 0;
  let nextFrame = 0;
  let currentSelection = null;
  let rangeRect = null;

  const body = element("body");
  const head = element("head");
  const root = element("html", { children: [head, body] });
  const document = {
    nodeType: 9,
    currentScript: { dataset: { parentOrigin, entryMime: "text/html" } },
    readyState: "complete",
    contentType: "text/html",
    referrer: "",
    documentElement: root,
    head,
    body,
    title: "Article",
    hidden: false,
    createElement(tag) { return element(tag); },
    createTextNode(value) { return text(value); },
    createDocumentFragment() {
      const fragment = element("#fragment");
      fragment.nodeType = 11;
      return fragment;
    },
    createTreeWalker(target, whatToShow, filter) { return walker(target, whatToShow, filter); },
    createRange() { return range(document, rangeRect); },
    getElementById() { return null; },
    getElementsByName() { return []; },
    querySelectorAll() { return []; },
    querySelector() { return null; },
    addEventListener(name, callback) {
      const callbacks = documentListeners.get(name) ?? [];
      callbacks.push(callback);
      documentListeners.set(name, callbacks);
    },
    removeEventListener() {},
    contains(node) { return root.contains(node); },
  };

  class Highlight {
    constructor(...ranges) { this.ranges = ranges; }
  }
  const speechSynthesis = {
    speaking: false,
    pending: false,
    getVoices: () => [],
    speak(utterance) { utterances.push(utterance); speechSynthesis.speaking = true; },
    cancel() { speechSynthesis.speaking = false; speechSynthesis.pending = false; },
    addEventListener() {},
  };
  class SpeechSynthesisUtterance {
    constructor(value) { this.text = value; }
  }
  const window = {
    parent: { postMessage(message) { messages.push(message); } },
    ...(cssHighlights ? { CSS: { highlights }, Highlight } : {}),
    getSelection: () => currentSelection,
    speechSynthesis,
    SpeechSynthesisUtterance,
    innerWidth: 800,
    innerHeight: 600,
    scrollX: 0,
    scrollY: 0,
    scrollTo(options) { scrolls.push(options); },
    scrollBy() {},
    matchMedia: () => ({ matches: false }),
    addEventListener(name, callback) {
      const callbacks = windowListeners.get(name) ?? [];
      callbacks.push(callback);
      windowListeners.set(name, callbacks);
    },
    removeEventListener() {},
  };
  function requestAnimationFrame(callback) {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  }
  function cancelAnimationFrame(id) { frames.delete(id); }
  function flushAnimationFrames() {
    for (const [id, callback] of [...frames]) {
      frames.delete(id);
      callback();
    }
  }

  const context = vm.createContext({
    window,
    ...(cssHighlights ? { CSS: window.CSS, Highlight } : {}),
    parent: window.parent,
    document,
    location: { origin: archiveOrigin, href: articleUrl, hash: "" },
    navigator: { language: "en", languages: ["en"] },
    history: { pushState() {}, replaceState() {}, state: null, go() {} },
    MutationObserver: class {
      constructor(callback) { this.callback = callback; this.active = false; }
      observe(target, options) {
        this.target = target;
        this.options = options;
        this.active = true;
        observers.push(this);
      }
      disconnect() { this.active = false; }
    },
    ResizeObserver: class {
      constructor(callback) { this.callback = callback; }
      observe() {}
      disconnect() {}
    },
    EventTarget: class {},
    Event: class { preventDefault() {} },
    MouseEvent: class {},
    WheelEvent: class {},
    DOMException,
    crypto: { randomUUID },
    URL,
    HTMLAnchorElement: class { get href() { return this.url; } },
    HTMLAreaElement: class { get href() { return this.url; } },
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3, DOCUMENT_FRAGMENT_NODE: 11 },
    NodeFilter: { SHOW_ELEMENT: 1, SHOW_TEXT: 4, FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3 },
    WeakRef,
    WeakSet,
    WeakMap,
    getComputedStyle: (node) => ({
      display: node.style?.display || "block",
      visibility: node.style?.visibility || "visible",
      contentVisibility: node.style?.contentVisibility || "visible",
    }),
    performance: { now: () => 0, getEntriesByType: () => [] },
    setTimeout: (callback) => { timers.set(++nextTimer, callback); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
    queueMicrotask,
    requestAnimationFrame,
    cancelAnimationFrame,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    speechSynthesis,
    SpeechSynthesisUtterance,
  });
  Object.defineProperty(context.MouseEvent.prototype, "ctrlKey", { get() { return false; } });
  Object.defineProperty(context.WheelEvent.prototype, "deltaY", { get() { return 0; } });
  // Real event classes, defined in the page's own realm before the bridge runs,
  // so the bridge captures their accessors the way it does in a webview and a
  // test can then replace them the way an archive script would.
  if (nativeEvents)
    vm.runInContext(`
      globalThis.Event = class Event {
        preventDefault() {}
        stopImmediatePropagation() { this.stopped = true; }
      };
      globalThis.MessageEvent = class MessageEvent extends Event {
        #init;
        constructor(init) { super(); this.#init = init; this.isTrusted = init.isTrusted; }
        get data() { return this.#init.data; }
        get source() { return this.#init.source; }
        get origin() { return this.#init.origin; }
      };
    `, context);
  // The embedding shell window, whatever the page later assigns to `parent`.
  const shell = window.parent;
  vm.runInContext(bridge, context, { filename: "reader-bridge.js" });

  function receive(data, event = {}) {
    const init = {
      source: shell,
      origin: parentOrigin,
      isTrusted: true,
      data: { channel: "kiwix-reader", version: 1, sessionId, ...data },
      ...event,
    };
    const dispatched = nativeEvents ? new context.MessageEvent(init) : init;
    // `window.event` while the listeners run, as in a webview.
    window.event = dispatched;
    try {
      for (const callback of windowListeners.get("message") ?? []) {
        callback(dispatched);
        if (dispatched.stopped) break;
      }
    } finally { window.event = undefined; }
    return dispatched;
  }
  function emit(name, event = {}) {
    for (const callback of documentListeners.get(name) ?? []) callback(event);
    for (const callback of windowListeners.get(name) ?? []) callback(event);
  }
  function triggerAnnotationMutation(record) {
    for (const observer of observers)
      if (observer.active && observer.options?.attributeFilter?.includes("contenteditable"))
        observer.callback([record]);
  }
  return {
    body,
    head,
    root,
    document,
    messages,
    highlights,
    utterances,
    scrolls,
    start(...children) {
      for (const child of children) body.appendChild(child);
      receive({ type: "reader-init" });
    },
    command(message) { receive({ type: "reader-command", ...message }); },
    receive,
    context,
    window,
    speechSynthesis,
    setSelection(selection) { currentSelection = selection; },
    setRangeRect(rect) { rangeRect = rect; },
    emit,
    triggerAnnotationMutation,
    flushAnimationFrames,
    speechText() {
      return messages.filter((message) => message.type === "speech-text").at(-1);
    },
    highlightRanges(name) {
      return (highlights.get(name)?.ranges ?? []).map((item) => item.toString());
    },
    select(range, value) {
      currentSelection = {
        rangeCount: 1,
        isCollapsed: false,
        getRangeAt: () => range,
        toString: () => value,
      };
    },
  };
}

function paragraph(value) {
  const node = element("p");
  node.appendChild(text(value, node));
  return node;
}

function article() {
  const frame = reader();
  frame.start(
    paragraph("First sentence. Second sentence! Last one?"),
    paragraph("Third paragraph without punctuation"),
  );
  return frame;
}

test("mobile extraction is deterministic and highlights the spoken sentence", () => {
  const frame = article();
  frame.command({ command: "speech-text", requestId });
  assert.equal(
    frame.speechText().text,
    "First sentence. Second sentence! Last one?\nThird paragraph without punctuation",
  );

  const offset = frame.speechText().text.indexOf("Second");
  frame.command({ command: "speech-highlight", requestId, offset });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud-word"), ["Second"]);
  assert.equal(frame.body.children[0].children[0].nodeValue, "First sentence. Second sentence! Last one?");
  assert.equal(frame.body.children[1].children[0].nodeValue, "Third paragraph without punctuation");

  const third = frame.speechText().text.indexOf("Third");
  frame.command({ command: "speech-highlight", requestId, offset: third });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Third paragraph without punctuation"]);

  frame.command({ command: "speech-highlight", requestId, clear: true });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);
  assert.equal(frame.highlights.has("kiwix-read-aloud-word"), false);
});

test("real DOM childNodes drive title and section checks", () => {
  const frame = reader();
  frame.document.title = "Albert Einstein - Wikipedia";
  const title = element("h1", { attributes: { id: "firstHeading" } });
  const titleText = text("Albert Einstein", title);
  title.childNodes = [titleText];
  title.children = [];
  const content = element("div", { attributes: { id: "mw-content-text", class: "mw-parser-output" } });
  const lead = element("p");
  const bold = element("b", { parent: lead });
  const boldText = text("Albert Einstein", bold);
  bold.childNodes = [boldText];
  bold.children = [];
  lead.append(bold, text(" was a physicist.", lead));
  const references = element("h2");
  const headingTextNode = text("References", references);
  references.childNodes = [headingTextNode];
  references.children = [];
  const tail = element("p");
  tail.append(text("A reference must not be read.", tail));
  content.append(lead, references, tail);
  frame.start(title, content);
  frame.command({ command: "speech-text", requestId });
  assert.equal(frame.speechText().text, "was a physicist.");
});

test("article extraction skips site chrome, duplicate title and trailing reference sections", () => {
  const frame = reader();
  frame.document.title = "Albert Einstein - Wikipedia";
  const header = element("header");
  header.append(text("Wikipedia menu", header));
  const nav = element("nav");
  nav.append(text("Main page Contents Random article", nav));
  const title = element("h1", { attributes: { id: "firstHeading" } });
  title.append(text("Albert Einstein", title));
  const content = element("div", { attributes: { id: "mw-content-text", class: "mw-parser-output" } });
  const lead = element("p");
  const bold = element("b", { parent: lead });
  bold.append(text("Albert Einstein", bold));
  lead.append(bold, text(" was a theoretical physicist. He developed relativity.", lead));
  const toc = element("div", { attributes: { id: "toc" } });
  toc.append(text("Contents", toc));
  const infobox = element("table", { attributes: { class: "infobox" } });
  infobox.append(text("Born 14 March 1879", infobox));
  const references = element("h2");
  references.append(text("References", references));
  const reference = element("p");
  reference.append(text("A reference that should not be read.", reference));
  content.append(lead, toc, infobox, references, reference);
  frame.start(header, nav, title, content);
  frame.command({ command: "speech-text", requestId });
  const spoken = frame.speechText().text;
  assert.equal(spoken, "was a theoretical physicist. He developed relativity.");
  assert.equal(spoken.includes("Wikipedia menu"), false);
  assert.equal(spoken.includes("Main page"), false);
  assert.equal(spoken.includes("Born 14 March"), false);
  assert.equal(spoken.includes("A reference"), false);
});

test("a sentence spanning inline nodes paints one continuous range", () => {
  const frame = reader();
  const block = element("p");
  const start = text("Hello ", block);
  const emphasis = element("em", { parent: block, style: { display: "inline" } });
  const middle = text("brave new ", emphasis);
  emphasis.appendChild(middle);
  const end = text("world. Next.", block);
  block.append(start, emphasis, end);
  frame.start(block);
  frame.command({ command: "speech-text", requestId });
  assert.equal(frame.speechText().text, "Hello brave new world. Next.");
  frame.command({ command: "speech-highlight", offset: 6 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Hello brave new world."]);
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud-word"), ["brave"]);
});

test("CJK word highlighting stops at the sentence terminator", () => {
  const frame = reader();
  frame.start(paragraph("你好世界。第二句话。"));
  frame.command({ command: "speech-text", requestId });
  assert.equal(frame.speechText().text, "你好世界。第二句话。");
  frame.command({ command: "speech-highlight", offset: 0 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["你好世界。"]);
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud-word"), ["你好世界"]);
  frame.command({ command: "speech-highlight", offset: 5 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["第二句话。"]);
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud-word"), ["第二句话"]);
});

test("normalization matches the previous regex pipeline", () => {
  const samples = [
    "  a  b ",
    "a\n\nb",
    "a \n b",
    "a\r\nb",
    "\n a \n",
    "a\u00a0\u00a0b",
    "a\tb\nc",
    "  \n  ",
  ];
  for (const sample of samples) {
    const frame = reader();
    const block = element("p");
    block.appendChild(text(sample, block));
    frame.start(block);
    frame.command({ command: "speech-text", requestId });
    const expected = sample
      .replace(/[^\S\n]+/g, " ")
      .replace(/ *\n */g, "\n")
      .replace(/\n{2,}/g, "\n")
      .trim();
    assert.equal(frame.speechText().text, expected, JSON.stringify(sample));
  }
});

test("whitespace normalization keeps offsets aligned with the live text", () => {
  const frame = reader();
  const block = element("p");
  block.append(
    text("  A  B \n C  ", block),
    element("br", { parent: block }),
    text("D", block),
  );
  frame.start(block);
  frame.command({ command: "speech-text", requestId });
  assert.equal(frame.speechText().text, "A B\nC\nD");
  frame.command({ command: "speech-highlight", requestId, offset: 4 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["C"]);
});

test("a new extraction clears previous paint and offsets use the current document", () => {
  const frame = article();
  // No document yet: an offset has nothing to paint.
  frame.command({ command: "speech-highlight", offset: 0 });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);

  frame.command({ command: "speech-text", requestId });
  frame.command({ command: "speech-highlight", offset: 0 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["First sentence."]);

  const nextRequest = randomUUID();
  frame.command({ command: "speech-text", requestId: nextRequest });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);
  frame.command({
    command: "speech-highlight",
    offset: frame.speechText().text.indexOf("Second"),
  });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
});

test("desktop speech follows boundary offsets and clears on stop", () => {
  const frame = article();
  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  const utterance = frame.utterances.at(-1);
  assert.equal(
    utterance.text,
    "First sentence. Second sentence! Last one?\nThird paragraph without punctuation",
  );
  utterance.onstart();
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["First sentence."]);
  utterance.onboundary({ charIndex: utterance.text.indexOf("Second") });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
  frame.command({ command: "stop-speech" });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);
  assert.equal(frame.highlights.has("kiwix-read-aloud-word"), false);
});

test("desktop selection speech uses the selection anchor and never rewrites text", () => {
  const frame = article();
  const first = frame.body.children[0].children[0];
  const selectedRange = range(frame.document);
  selectedRange.setStart(first, 0);
  selectedRange.setEnd(first, "First sentence. Second".length);
  frame.select(selectedRange, "First sentence. Second");
  frame.command({
    command: "speak",
    text: "First sentence. Second",
    start: 0,
    end: "First sentence. Second".length,
    rate: 1,
    voice: "",
    lang: "en",
  });
  const utterance = frame.utterances.at(-1);
  assert.equal(utterance.text, "First sentence. Second");
  utterance.onboundary({ charIndex: 16 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second"]);
  assert.equal(first.nodeValue, "First sentence. Second sentence! Last one?");
});

test("live selection mapping is a fallback when anchor offsets are absent", () => {
  const frame = article();
  const first = frame.body.children[0].children[0];
  const selected = range(frame.document);
  selected.setStart(first, 0);
  selected.setEnd(first, "First sentence. Second".length);
  frame.select(selected, "First sentence. Second");
  frame.command({
    command: "speak",
    text: "First sentence. Second",
    rate: 1,
    voice: "",
    lang: "en",
  });
  frame.utterances.at(-1).onboundary({ charIndex: 16 });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second"]);
});

test("a unique spoken passage can still be mapped after selection collapse", () => {
  const frame = article();
  frame.setSelection(null);
  frame.command({
    command: "speak",
    text: "Second sentence!",
    rate: 1,
    voice: "",
    lang: "en",
  });
  frame.utterances.at(-1).onstart();
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
});

test("an out-of-view sentence scrolls into place without touching text", () => {
  const frame = article();
  frame.setRangeRect({ left: 10, top: 700, right: 110, bottom: 740, width: 100, height: 40 });
  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  frame.utterances.at(-1).onstart();
  assert.equal(frame.scrolls.length, 1);
  assert.equal(frame.scrolls[0].top, 500);
  assert.equal(frame.body.children[0].children[0].nodeValue, "First sentence. Second sentence! Last one?");
});

test("background visibility clears paint but keeps the mapping for return", () => {
  const frame = article();
  frame.command({ command: "speech-text", requestId });
  frame.command({ command: "speech-highlight", offset: 0 });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), true);

  frame.document.hidden = true;
  frame.emit("visibilitychange");
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);

  // Native speech may survive Android backgrounding; the next offset after
  // return must repaint without a second extraction.
  frame.document.hidden = false;
  frame.command({
    command: "speech-highlight",
    offset: frame.speechText().text.indexOf("Second"),
  });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
});

test("a hidden window keeps desktop read-aloud going and only drops its paint", () => {
  // Minimising or covering the window hides the document; reading must not end.
  const frame = article();
  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  const utterance = frame.utterances.at(-1);
  utterance.onstart();
  assert.equal(frame.highlights.has("kiwix-read-aloud"), true);
  let cancelled = 0;
  const cancel = frame.speechSynthesis.cancel;
  frame.speechSynthesis.cancel = () => { cancelled += 1; cancel(); };
  const before = frame.messages.length;

  frame.document.hidden = true;
  frame.emit("visibilitychange");
  assert.equal(cancelled, 0, "the utterance is not cancelled");
  assert.equal(frame.speechSynthesis.speaking, true);
  assert.deepEqual(frame.messages.slice(before).filter((message) => message.type === "speech-state"), [],
    "the shell is not told that reading stopped");
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false, "the read-along paint is dropped while hidden");

  // The same utterance is still the live one: its next boundary repaints.
  frame.document.hidden = false;
  frame.emit("visibilitychange");
  utterance.onboundary({ charIndex: utterance.text.indexOf("Second") });
  assert.deepEqual(frame.highlightRanges("kiwix-read-aloud"), ["Second sentence!"]);
  // An explicit stop still ends it.
  frame.command({ command: "stop-speech" });
  assert.equal(cancelled, 1);
  assert.deepEqual(frame.messages.at(-1).type, "speech-state");
  assert.equal(frame.messages.at(-1).speaking, false);
});

function heading(tag, title, attributes = {}) {
  return Object.assign(element(tag, { attributes }), { textContent: title, id: attributes.id ?? "" });
}

test("the contents list names every entry and skips blank or hidden headings", () => {
  const frame = reader();
  const headings = [
    heading("h1", "Earth"),
    heading("h2", ""),
    heading("h2", " \n "),
    heading("h2", "Hidden", { hidden: "" }),
    heading("h2", "Screen-reader hidden", { "aria-hidden": "true" }),
    heading("h2", "History", { id: "History" }),
    heading("h3", "Formation"),
  ];
  frame.document.querySelectorAll = (selector) => selector === "h1,h2,h3,h4,h5,h6" ? headings : [];
  frame.start();
  // Values cross the vm realm; compare their plain data.
  const toc = JSON.parse(JSON.stringify(frame.messages.find((message) => message.type === "ready").toc));
  assert.deepEqual(toc, [
    { id: "kiwix-heading-0", title: "Earth", level: 1 },
    { id: "History", title: "History", level: 2 },
    { id: "kiwix-heading-6", title: "Formation", level: 3 },
  ]);
  assert.ok(toc.every((entry) => entry.title.trim()), "no entry is an unnamed button");
  // Generated ids are those of the full heading list, so a jump still finds its heading.
  assert.equal(headings[6].id, "kiwix-heading-6");
});

test("the bridge proves an unchanged document only for the session it holds, with its private token", () => {
  const nonce = randomUUID();
  const fresh = reader();
  fresh.receive({ type: "document-probe", nonce });
  assert.deepEqual(fresh.messages.map((message) => message.type), ["reading-gesture-hello"],
    "a document that was never handed a session proves nothing");

  const frame = article();
  const hello = frame.messages.find((message) => message.type === "reading-gesture-hello");
  const bound = frame.messages.find((message) => message.type === "reading-pan-bind");
  assert.equal(bound.token, hello.token, "the session is bound to the token announced at bootstrap");
  const before = frame.messages.length;
  frame.receive({ type: "document-probe", nonce, sessionId: "another-session" });
  frame.receive({ type: "document-probe", nonce: "not-a-nonce" });
  frame.receive({ type: "document-probe", nonce }, { source: {} });
  frame.receive({ type: "document-probe", nonce }, { origin: archiveOrigin });
  frame.receive({ type: "document-probe", nonce }, { isTrusted: false });
  assert.equal(frame.messages.length, before, "foreign, malformed and forged probes are not answered");
  frame.receive({ type: "document-probe", nonce });
  assert.deepEqual(JSON.parse(JSON.stringify(frame.messages.slice(before))), [{
    channel: "kiwix-reader", version: 1, type: "document-proof", sessionId, nonce, token: hello.token,
  }]);
  // A replacement session ends the old one's proofs.
  frame.receive({ type: "reader-init", sessionId: "replacement-session" });
  const after = frame.messages.length;
  frame.receive({ type: "document-probe", nonce });
  assert.equal(frame.messages.length, after);
});

// What a hostile archive script can do once it runs, after the bridge: replace
// every accessor and method the bridge might look up, plant getters for absent
// fields, listen for the shell's messages itself, and peek at window.event from
// inside any call the bridge makes.
function hostile(frame) {
  const stolen = [];
  const run = (source) => vm.runInContext(source, frame.context);
  frame.context.stolen = stolen;
  run(`
    const apply = Reflect.apply;
    const stringify = JSON.stringify;
    const shell = parent;
    const data = Object.getOwnPropertyDescriptor(MessageEvent.prototype, "data").get;
    let busy = false;
    const steal = (value) => {
      if (busy) return;
      busy = true;
      try { stolen[stolen.length] = stringify(value); } catch (_) {} finally { busy = false; }
    };
    const read = (event) => { if (busy) return undefined; busy = true; try { return apply(data, event, []); } finally { busy = false; } };
    for (const key of ["data", "source", "origin"]) {
      const original = Object.getOwnPropertyDescriptor(MessageEvent.prototype, key).get;
      Object.defineProperty(MessageEvent.prototype, key, { configurable: true, get() {
        steal(read(this));
        // Also claim to come from the shell, whatever the event really says.
        return key === "source" ? shell : key === "origin" ? ${JSON.stringify(parentOrigin)} : apply(original, this, []);
      } });
    }
    Event.prototype.stopImmediatePropagation = function () { steal(read(this)); };
    for (const key of ["channel", "version", "type", "sessionId", "photoSecret", "nonce", "linkPreviews", "mediaNotice", "sensorStatus", "photoChooser"])
      Object.defineProperty(Object.prototype, key, { configurable: true, get() { steal(this); return undefined; }, set(value) {
        Object.defineProperty(this, key, { value, writable: true, configurable: true, enumerable: true });
      } });
    // Anything the bridge calls while handling the message can look at window.event.
    const peek = () => { if (!busy && window.event && typeof window.event === "object" && "stopped" in window.event || window.event?.isTrusted) steal(read(window.event)); };
    for (const [owner, name] of [[Array.prototype, "includes"], [RegExp.prototype, "test"], [Function.prototype, "call"], [Function.prototype, "apply"]]) {
      const original = owner[name];
      owner[name] = function (...args) { peek(); return apply(original, this, args); };
    }
    const querySelectorAll = document.querySelectorAll;
    document.querySelectorAll = function (...args) { peek(); return apply(querySelectorAll, this, args); };
    window.addEventListener("message", (event) => steal(read(event)), true);
    window.parent = window;
  `);
  return stolen;
}

test("an archive script that replaces the message accessors learns neither the session nor the photo secret", () => {
  const frame = reader({ nativeEvents: true });
  const stolen = hostile(frame);
  const photoSecret = randomUUID();
  frame.receive({ type: "reader-init", photoSecret, photoChooser: true, linkPreviews: true });
  frame.command({ command: "toc", id: "kiwix-heading-0" });
  const nonce = randomUUID();
  frame.receive({ type: "document-probe", nonce });
  const seen = stolen.join("\n");
  assert.equal(seen.includes(photoSecret), false, "the photo secret never reaches archive script");
  assert.equal(seen.includes(sessionId), false, "nor does the session id");
  assert.ok(stolen.length > 0, "the hooks did run: the bridge was observed and still gave nothing away");
  // The bridge itself still got everything: it is ready and answers for its session.
  const types = frame.messages.map((message) => message.type);
  assert.ok(types.includes("ready"));
  assert.equal(frame.messages.find((message) => message.type === "ready").sessionId, sessionId);
  const hello = frame.messages.find((message) => message.type === "reading-gesture-hello");
  assert.equal(frame.messages.at(-1).type, "document-proof");
  assert.equal(frame.messages.at(-1).token, hello.token);
});

test("the bridge binds its token to a new session before anything an archive script can reach", () => {
  const frame = reader({ nativeEvents: true });
  frame.receive({ type: "reader-init", photoSecret: randomUUID() });
  // The archive now makes the retirement of the outgoing session throw, and
  // posts its own binding from every hook it has.
  const stolen = hostile(frame);
  frame.speechSynthesis.cancel = () => { throw new Error("hostile"); };
  const before = frame.messages.length;
  const replacement = "replacement-session";
  frame.receive({ type: "reader-init", sessionId: replacement, photoSecret: randomUUID() });
  const posted = frame.messages.slice(before);
  const hello = frame.messages.find((message) => message.type === "reading-gesture-hello");
  assert.equal(posted[0].type, "reading-pan-bind", "the binding is the first thing posted for the new session");
  assert.equal(posted[0].sessionId, replacement);
  assert.equal(posted[0].token, hello.token);
  assert.equal(stolen.join("\n").includes(replacement), false, "before any hook could have learnt the session");
  assert.ok(posted.some((message) => message.type === "ready" && message.sessionId === replacement),
    "a failing retirement of the old session does not strand the new one");
});

test("replaced accessors cannot make the bridge accept a message that is not from the shell", () => {
  const frame = reader({ nativeEvents: true });
  hostile(frame);
  const before = frame.messages.length;
  // The hooked accessors report the shell as source and origin for every event.
  frame.receive({ type: "reader-init", sessionId: "forged-session" }, { source: frame.window });
  frame.receive({ type: "reader-init", sessionId: "forged-session" }, { origin: archiveOrigin });
  frame.receive({ type: "reader-init", sessionId: "forged-session" }, { isTrusted: false });
  assert.deepEqual(frame.messages.slice(before), [], "the bridge reads the real source and origin");
});

test("a document restored from the back-forward cache announces itself again", () => {
  const frame = article();
  const hellos = () => frame.messages.filter((message) => message.type === "reading-gesture-hello");
  assert.equal(hellos().length, 1);
  frame.emit("pageshow", { persisted: false });
  assert.equal(hellos().length, 1, "an ordinary first show is not a restore");
  frame.emit("pageshow", { persisted: true });
  assert.equal(hellos().length, 2);
  assert.equal(hellos()[1].token, hellos()[0].token, "with the same private token");
  assert.equal(hellos()[1].sessionId, undefined, "and without the session it used to hold");
});

function click(frame, href, { trusted = true, type = "click", button = 0 } = {}) {
  const anchor = { namespaceURI: "http://www.w3.org/1999/xhtml", localName: "a", url: href, target: "" };
  const event = {
    type, button, isTrusted: trusted, defaultPrevented: false, target: { closest: () => anchor },
    composedPath: () => [], preventDefault() { this.defaultPrevented = true; }, stopImmediatePropagation() {},
  };
  const before = frame.messages.length;
  frame.emit(type, event);
  return { event, posted: frame.messages.slice(before) };
}

test("links the app cannot open stay blocked and are answered with a notice request, not an external-link", () => {
  const frame = article();
  for (const href of ["tel:+15550100", "sms:+15550100", "geo:48.85,2.35", "data:text/plain,hello"]) {
    for (const [type, button] of [["click", 0], ["auxclick", 1]]) {
      const { event, posted } = click(frame, href, { type, button });
      assert.equal(event.defaultPrevented, true, `${href} never navigates the frame (${type})`);
      assert.deepEqual(posted.map((message) => message.type), ["unsupported-link"], `${href} (${type})`);
      assert.deepEqual(Object.keys(posted[0]).sort(), ["channel", "sessionId", "type", "version"],
        "the notice request carries no address");
    }
  }
  // Page controls and script-made clicks are blocked silently.
  assert.deepEqual(click(frame, "javascript:void(0)").posted, []);
  assert.deepEqual(click(frame, "tel:+15550100", { trusted: false }).posted, []);
  // Supported external schemes still ask the shell to confirm.
  for (const href of ["https://example.org/page", "mailto:reader@example.org"]) {
    const { event, posted } = click(frame, href);
    assert.equal(event.defaultPrevented, true);
    assert.deepEqual(posted.map((message) => [message.type, message.url]), [["external-link", href]]);
  }
});

test("navigation and pagehide clear the highlight", () => {
  const frame = article();
  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  const utterance = frame.utterances.at(-1);
  utterance.onstart();
  assert.equal(frame.highlights.has("kiwix-read-aloud"), true);
  frame.emit("popstate");
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);

  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  frame.utterances.at(-1).onstart();
  assert.equal(frame.highlights.has("kiwix-read-aloud"), true);
  frame.emit("pagehide");
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);
});

const savedPassage = {
  id: "00000000-0000-4000-8000-000000000001",
  bookmarkId: "00000000-0000-4000-8000-000000000002",
  documentPath: "index.html",
  exact: "First sentence.",
  prefix: "",
  suffix: " Second",
  start: 0,
  end: 15,
};

test("saved passage highlights survive read-aloud start and stop", () => {
  const frame = article();
  frame.command({ command: "highlights-begin", url: articleUrl, documentPath: "index.html" });
  frame.command({ command: "highlights", url: articleUrl, highlights: [savedPassage] });
  frame.command({ command: "highlights-end", url: articleUrl });
  assert.equal(frame.highlights.get("kiwix-passages")?.ranges.length, 1);

  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), true);
  const style = frame.head.children.find((node) => node.dataset?.kiwixReadAloudUi !== undefined);
  assert.ok(style, "read-aloud stylesheet is present");
  frame.triggerAnnotationMutation({
    type: "childList", target: frame.head, addedNodes: [style], removedNodes: [],
  });
  assert.equal(
    frame.highlights.get("kiwix-passages")?.ranges.length,
    1,
    "read-aloud stylesheet insertion does not clear saved passages",
  );

  const overlay = element("div");
  overlay.dataset.kiwixReadAloudUi = "";
  frame.triggerAnnotationMutation({
    type: "childList", target: frame.root, addedNodes: [overlay], removedNodes: [],
  });
  assert.equal(
    frame.highlights.get("kiwix-passages")?.ranges.length,
    1,
    "read-aloud overlay insertion does not clear saved passages",
  );

  frame.command({ command: "stop-speech" });
  assert.equal(frame.highlights.has("kiwix-read-aloud"), false);
  frame.triggerAnnotationMutation({
    type: "childList", target: frame.head, addedNodes: [], removedNodes: [style],
  });
  assert.equal(
    frame.highlights.get("kiwix-passages")?.ranges.length,
    1,
    "read-aloud stylesheet removal does not clear saved passages",
  );
});

test("a WebView without CSS highlights paints an aria-hidden overlay", () => {
  const frame = reader({ cssHighlights: false });
  frame.start(paragraph("Only one sentence here."));
  frame.command({ command: "speak", rate: 1, voice: "", lang: "en" });
  const utterance = frame.utterances.at(-1);
  utterance.onstart();
  frame.flushAnimationFrames();
  const layer = frame.root.children.find((child) => child.dataset.kiwixReadAloudUi === "");
  assert.ok(layer, "read-aloud overlay is appended to the document");
  assert.equal(layer.getAttribute("aria-hidden"), "true");
  assert.match(layer.style.cssText, /pointer-events:none/);
  const marks = layer.shadowRoot.children.at(-1).children;
  assert.equal(marks.length, 2);
  assert.match(marks[0].style.cssText, /background:#f2ce687d/);
  assert.match(marks[1].style.cssText, /background:#e0a92ecc/);
});
