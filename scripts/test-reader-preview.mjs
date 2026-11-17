import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const tree = ts.createSourceFile("reader-bridge.js", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const declarations = [];
function visit(node) {
  if ((ts.isFunctionDeclaration(node) && ["linkHref", "previewLink", "leavePreviewLink", "setLinkPreviews"].includes(node.name?.text)) ||
      (ts.isVariableStatement(node) && node.declarationList.declarations.some((item) =>
        ["previewTimer", "previewAnchor", "linkPreviews"].includes(item.name.getText(tree)))))
    declarations.push(node.getText(tree));
  ts.forEachChild(node, visit);
}
visit(tree);

function bridge() {
  let now = 1000;
  let next = 0;
  const timers = new Map();
  const events = [];
  const context = vm.createContext({
    URL, Node: class Node {},
    location: { href: "http://archive.localhost:8123/index.html", origin: "http://archive.localhost:8123" },
    performance: { now: () => now },
    setTimeout(callback, delay) { const id = ++next; timers.set(id, { callback, at: now + delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    post(type, value) { events.push({ type, ...value }); },
  });
  vm.runInContext(declarations.join("\n"), context);
  return {
    context, events,
    anchor(path) { return { href: new URL(path, context.location.href).href, isConnected: true, contains: () => false }; },
    focus(anchor, trusted = true) {
      context.previewLink({ isTrusted: trusted, target: { closest: () => anchor } });
    },
    leave(anchor) { context.leavePreviewLink({ target: { closest: () => anchor }, relatedTarget: null }); },
    advance(ms) {
      const until = now + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, value]) => value.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        now = due[1].at; timers.delete(due[0]); due[1].callback();
      }
      now = until;
    },
  };
}

test("moving focus to a second article immediately after a preview does not lose its preview", () => {
  const r = bridge();
  r.context.setLinkPreviews(true);
  const first = r.anchor("first.html");
  const second = r.anchor("second.html");
  r.focus(first); r.advance(350);
  r.leave(first); r.focus(second); r.advance(750);
  assert.deepEqual(r.events.map((event) => event.url), [first.href, second.href]);
});

test("departed, disconnected, untrusted and external links cannot emit preview requests", () => {
  const r = bridge();
  r.context.setLinkPreviews(true);
  const departed = r.anchor("departed.html");
  r.focus(departed); r.advance(100); r.leave(departed); r.advance(500);
  const detached = r.anchor("detached.html");
  r.focus(detached); detached.isConnected = false; r.advance(500);
  r.focus(r.anchor("untrusted.html"), false); r.advance(500);
  r.focus(r.anchor("https://example.org/external")); r.advance(500);
  assert.deepEqual(r.events, []);
});

test("disabling cancels an imminent preview and re-enabling permits the same anchor", () => {
  const r = bridge();
  const anchor = r.anchor("first.html");
  r.context.setLinkPreviews(true);
  r.focus(anchor); r.advance(349);
  r.context.setLinkPreviews(false); r.advance(1000);
  r.focus(anchor); r.advance(1000);
  assert.deepEqual(r.events, []);
  r.context.setLinkPreviews(true);
  r.focus(anchor); r.advance(350);
  assert.deepEqual(r.events.map((event) => event.url), [anchor.href]);
});

const readerSource = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const readerTree = ts.createSourceFile("Reader.tsx", readerSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let drainDeclaration;
let preferenceEffect;
function visitReader(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(readerTree) === "drainPreview")
    drainDeclaration = node.getText(readerTree);
  if (ts.isCallExpression(node) && node.expression.getText(readerTree) === "useEffect" &&
      node.arguments[1] && ts.isArrayLiteralExpression(node.arguments[1]) &&
      node.arguments[1].elements.some((item) => item.getText(readerTree) === "settings.linkPreviews"))
    preferenceEffect = node.arguments[0].getText(readerTree);
  ts.forEachChild(node, visitReader);
}
visitReader(readerTree);

function shell() {
  let now = 1000;
  let next = 0;
  const timers = new Map();
  const calls = [];
  let preview = null;
  const session = { sessionId: "first-session", privacyGeneration: 1, location: { bookId: "book" } };
  const props = { settings: { linkPreviews: true }, suspended: false, interactive: true, privacyGeneration: 1 };
  const clearTimeout = (id) => timers.delete(id);
  const context = vm.createContext({
    useCallback: (callback) => callback,
    alive: { current: true }, latest: { current: props }, settings: props.settings,
    activeFrame: { current: session }, previewSequence: { current: 0 }, previewQueued: { current: null },
    previewGate: { current: { at: 0, pending: false, timer: undefined } },
    canNavigateReaderStorage: () => true, frameReady: true, send() {},
    performance: { now: () => now }, clearTimeout,
    window: { setTimeout(callback, delay) { const id = ++next; timers.set(id, { callback, at: now + delay }); return id; } },
    setPreview: (value) => { preview = value; },
    object: (value) => value !== null && typeof value === "object",
    text: (value, max) => typeof value === "string" && value.length <= max,
    api(_command, args) { return new Promise((resolve) => calls.push({ path: args.path, resolve })); },
  });
  const code = `const ${drainDeclaration}; globalThis.drain = drainPreview; globalThis.preference = ${preferenceEffect};`;
  vm.runInContext(ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return {
    context, props, session, calls, get preview() { return preview; },
    enabled(value) { props.settings.linkPreviews = value; context.preference(); },
    queue(path) {
      context.previewQueued.current = { session, path, sequence: ++context.previewSequence.current };
      context.drain();
    },
    async finish(index) {
      const call = calls[index];
      call.resolve({ title: call.path, excerpt: `Excerpt ${call.path}`, path: call.path, mime: "text/html", thumbnail: null });
      await new Promise((resolve) => setImmediate(resolve));
    },
    advance(ms) {
      const until = now + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, value]) => value.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        now = due[1].at; timers.delete(due[0]); due[1].callback();
      }
      now = until;
    },
  };
}

test("off then on cannot resurrect an earlier response and a new request recovers", async () => {
  const r = shell();
  r.queue("old.html");
  r.enabled(false); r.enabled(true);
  await r.finish(0);
  assert.equal(r.preview, null);
  r.queue("new.html");
  r.advance(400);
  assert.deepEqual(r.calls.map((call) => call.path), ["old.html", "new.html"]);
  await r.finish(1);
  assert.equal(r.preview.article.title, "new.html");
});

test("disabling cancels the root's queued timer without blocking a later preview", async () => {
  const r = shell();
  r.queue("first.html"); await r.finish(0);
  r.queue("cancelled.html"); r.advance(100);
  r.enabled(false); r.advance(1000);
  assert.deepEqual(r.calls.map((call) => call.path), ["first.html"]);
  assert.equal(r.preview, null);
  r.enabled(true); r.queue("recovered.html");
  await r.finish(1);
  assert.equal(r.preview.article.title, "recovered.html");
});

test("while one preview is pending only the newest queued article dispatches", async () => {
  const r = shell();
  r.queue("first.html"); r.queue("middle.html"); r.queue("last.html");
  await r.finish(0);
  assert.equal(r.preview.article, null);
  r.advance(399);
  assert.deepEqual(r.calls.map((call) => call.path), ["first.html"]);
  r.advance(1);
  assert.deepEqual(r.calls.map((call) => call.path), ["first.html", "last.html"]);
  await r.finish(1);
  assert.equal(r.preview.article.title, "last.html");
});

for (const mode of ["disabled", "suspended", "noninteractive", "privacy", "replacement"]) {
  test(`a deferred preview cannot publish after ${mode} authority changes`, async () => {
    const r = shell();
    r.queue("retired.html");
    if (mode === "disabled") r.enabled(false);
    if (mode === "suspended") r.props.suspended = true;
    if (mode === "noninteractive") r.props.interactive = false;
    if (mode === "privacy") r.props.privacyGeneration++;
    if (mode === "replacement") r.context.activeFrame.current = { ...r.session, sessionId: "replacement-session" };
    await r.finish(0);
    assert.equal(r.preview?.article ?? null, null);
  });
}
