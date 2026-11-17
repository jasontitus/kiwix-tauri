import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Node-only, ordered-message model, not browser/device or assistive-technology proof.
// Execute only after independent review under AGENTS.md. No real timers or IPC.
const bridgeSource = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const readerSource = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const bridgeTree = ts.createSourceFile("reader-bridge.js", bridgeSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const readerTree = ts.createSourceFile("Reader.tsx", readerSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const bridgeParts = new Map();
const readerParts = new Map();
let escapeRegistration;
let preferenceEffect;
function visitBridge(node) {
  if (ts.isFunctionDeclaration(node) && ["post", "linkHref", "setLinkPreviews", "previewLink", "leavePreviewLink"].includes(node.name?.text))
    bridgeParts.set(node.name.text, node.getText(bridgeTree));
  if (ts.isVariableStatement(node)) {
    for (const declaration of node.declarationList.declarations) {
      if (["previewTimer", "previewAnchor", "linkPreviews"].includes(declaration.name.getText(bridgeTree)))
        bridgeParts.set(declaration.name.getText(bridgeTree), node.getText(bridgeTree));
    }
  }
  if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression) &&
      node.expression.expression.getText(bridgeTree) === "document.addEventListener" &&
      node.expression.arguments[0]?.getText(bridgeTree) === '"keydown"' &&
      node.expression.arguments[1]?.getText(bridgeTree).includes('post("dismiss-reader-ui",')) {
    assert.equal(escapeRegistration, undefined, "exactly one archive Escape listener");
    escapeRegistration = node.getText(bridgeTree);
  }
  ts.forEachChild(node, visitBridge);
}
function visitReader(node) {
  if (ts.isVariableDeclaration(node) && ["drainPreview", "receive"].includes(node.name.getText(readerTree))) {
    assert.equal(readerParts.has(node.name.getText(readerTree)), false, "unique reader callback");
    readerParts.set(node.name.getText(readerTree), `const ${node.getText(readerTree)};`);
  }
  if (ts.isFunctionDeclaration(node) && ["object", "text"].includes(node.name?.text))
    readerParts.set(node.name.text, node.getText(readerTree));
  if (ts.isCallExpression(node) && node.expression.getText(readerTree) === "useEffect" &&
      node.arguments[1] && ts.isArrayLiteralExpression(node.arguments[1]) &&
      node.arguments[1].elements.some((item) => item.getText(readerTree) === "settings.linkPreviews")) {
    assert.equal(preferenceEffect, undefined, "unique preview preference effect");
    preferenceEffect = node.arguments[0].getText(readerTree);
  }
  ts.forEachChild(node, visitReader);
}
visitBridge(bridgeTree);
visitReader(readerTree);
for (const name of ["post", "linkHref", "setLinkPreviews", "previewLink", "leavePreviewLink", "previewTimer", "previewAnchor", "linkPreviews"])
  assert.ok(bridgeParts.has(name), `bridge extraction: ${name}`);
for (const name of ["drainPreview", "receive", "object", "text"])
  assert.ok(readerParts.has(name), `reader extraction: ${name}`);
assert.ok(escapeRegistration, "actual archive Escape listener must be exercised");
assert.ok(preferenceEffect, "actual preference effect must be exercised");

function harness() {
  let now = 1000;
  let nextTimer = 0;
  let preview = null;
  let dismissals = 0;
  const timers = new Map();
  const requests = [];
  const messages = [];
  const listeners = new Map();
  const frame = {};
  const location = { origin: "http://archive.localhost:8123", href: "http://archive.localhost:8123/index.html" };
  const session = { sessionId: "frame-1", privacyGeneration: 1, location: { ...location, url: location.href, bookId: "book" } };
  const props = { settings: { linkPreviews: true }, suspended: false, interactive: true, privacyGeneration: 1 };
  const setTimeout = (callback, delay) => {
    const id = ++nextTimer;
    timers.set(id, { callback, at: now + delay });
    return id;
  };
  const clearTimeout = (id) => timers.delete(id);
  const shell = vm.createContext({
    URL, isMobile: false, useCallback: (callback) => callback,
    alive: { current: true }, latest: { current: props }, settings: props.settings,
    frameRef: { current: { contentWindow: frame } }, activeFrame: { current: session },
    previewSequence: { current: 0 }, previewQueued: { current: null },
    previewGate: { current: { at: 0, pending: false, timer: undefined } },
    workspaceRef: { current: { getClientRects: () => [1] } }, tabRef: { current: { path: "index.html" } },
    document: { querySelector: () => null }, canNavigateReaderStorage: () => true,
    // URL/archive path parsing is an existing-policy seam, not a claim tested here.
    articlePath: (url, current) => new URL(url).origin === current.origin ? new URL(url).pathname.slice(1) : null,
    performance: { now: () => now }, window: { setTimeout }, clearTimeout,
    setPreview(value) { preview = value; }, setLinkContext() {},
    dismissReaderChrome() { dismissals++; }, frameReady: true,
    send(message) { bridge.setLinkPreviews(message.enabled); },
    api(command, args) {
      assert.equal(command, "article_preview");
      return new Promise((resolve) => requests.push({ path: args.path, resolve }));
    },
  });
  const shellCode = `${[...readerParts.values()].join("\n")}\nglobalThis.receiveMessage = receive; globalThis.preference = ${preferenceEffect};`;
  vm.runInContext(ts.transpileModule(shellCode, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, shell);
  const bridgeContext = {
    URL, Node: class Node {}, location, channel: "kiwix-reader", sessionId: session.sessionId,
    parentOrigin: "tauri://localhost", setTimeout, clearTimeout,
    document: { addEventListener(type, callback) { assert.equal(listeners.has(type), false); listeners.set(type, callback); } },
    parent: { postMessage(data, origin) {
      assert.equal(origin, "tauri://localhost");
      messages.push({ at: now - 1000, type: data.type, url: data.url });
      shell.receiveMessage({ data, source: frame, origin: location.origin });
    } },
    // Current bridge helpers referenced by the extracted Escape listener; the
    // pan gesture itself is covered by its own runner.
    resetReadingPan() {},
  };
  bridgeContext.postToParent = bridgeContext.parent.postMessage.bind(bridgeContext.parent);
  const bridge = vm.createContext(bridgeContext);
  vm.runInContext(`${[...bridgeParts.values()].join("\n")}\n${escapeRegistration}`, bridge);
  bridge.setLinkPreviews(true);
  return {
    shell, session, props, requests, messages,
    get preview() { return preview; }, get dismissals() { return dismissals; },
    anchor(path = "target.html") { return { href: new URL(path, location.href).href, isConnected: true, contains: () => false }; },
    intent(anchor, trusted = true) { bridge.previewLink({ isTrusted: trusted, target: { closest: () => anchor } }); },
    leave(anchor) { bridge.leavePreviewLink({ target: { closest: () => anchor }, relatedTarget: null }); },
    // Trusted values model the listener contract; no synthetic browser trust is asserted.
    key(key = "Escape", trusted = true) { listeners.get("keydown")({ key, isTrusted: trusted }); },
    enabled(value) { props.settings.linkPreviews = value; shell.preference(); },
    message(data, overrides = {}) {
      shell.receiveMessage({ source: frame, origin: location.origin,
        data: { channel: "kiwix-reader", version: 1, sessionId: session.sessionId, ...data }, ...overrides });
    },
    async finish(index) {
      const request = requests[index];
      assert.ok(request, `preview request ${index} exists`);
      request.resolve({ title: request.path, path: request.path, excerpt: "Bounded excerpt", mime: "text/html", thumbnail: null });
      await new Promise((resolve) => setImmediate(resolve));
    },
    advance(ms) {
      assert.ok(ms >= 0 && ms <= 2000, "bounded deterministic clock");
      const until = now + ms;
      let callbacks = 0;
      for (;;) {
        const due = [...timers.entries()].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        assert.ok(++callbacks <= 100, "bounded timer drain");
        now = due[1].at;
        timers.delete(due[0]);
        due[1].callback();
      }
      now = until;
    },
  };
}

test("Escape during the last millisecond of the dwell cancels the future request", () => {
  const r = harness();
  r.intent(r.anchor()); r.advance(349); r.key();
  assert.equal(r.dismissals, 1, "trusted Escape still dismisses shell chrome");
  assert.equal(r.preview, null);
  r.advance(1000);
  assert.deepEqual(r.messages.map(({ at, type }) => ({ at, type })), [{ at: 349, type: "dismiss-reader-ui" }],
    "baseline defect is a second article-preview message at 350ms after dismissal");
  assert.equal(r.requests.length, 0, "no native preview work after cancellation");
  assert.equal(r.preview, null, "no loading card appears after Escape");
});

test("Escape cancellation permits a later deliberate intent on the same anchor", async () => {
  const r = harness();
  const anchor = r.anchor();
  r.intent(anchor); r.advance(349); r.key(); r.advance(1000);
  assert.equal(r.requests.length, 0);
  r.intent(anchor); r.advance(349);
  assert.equal(r.requests.length, 0);
  r.advance(1); await r.finish(0);
  assert.equal(r.preview.article.title, "target.html");
});

test("untrusted Escape and other trusted keys do not cancel the dwell", async () => {
  const r = harness();
  r.intent(r.anchor()); r.advance(175);
  r.key("Escape", false); r.key("ArrowRight"); r.advance(175);
  assert.equal(r.dismissals, 0);
  assert.equal(r.requests.length, 1);
  await r.finish(0);
  assert.equal(r.preview.article.title, "target.html");
});

test("Escape after dispatch invalidates an in-flight result without blocking recovery", async () => {
  const r = harness();
  r.intent(r.anchor("old.html")); r.advance(350);
  assert.equal(r.requests.length, 1);
  r.key(); await r.finish(0);
  assert.equal(r.preview, null);
  r.intent(r.anchor("new.html")); r.advance(400); await r.finish(1);
  assert.equal(r.preview.article.title, "new.html");
});

test("opt-out still cancels a dwell and opt-in requires a new intent", async () => {
  const r = harness();
  const anchor = r.anchor();
  r.intent(anchor); r.advance(349); r.enabled(false); r.advance(1000);
  r.intent(anchor); r.advance(350);
  assert.equal(r.requests.length, 0);
  r.enabled(true); r.advance(350);
  assert.equal(r.requests.length, 0);
  r.intent(anchor); r.advance(350); await r.finish(0);
  assert.equal(r.preview.article.title, "target.html");
});

test("departure still cancels a dwell", () => {
  const r = harness();
  const anchor = r.anchor();
  r.intent(anchor); r.advance(349); r.leave(anchor); r.advance(1000);
  assert.equal(r.requests.length, 0);
});

for (const authority of ["source", "origin", "session"]) {
  test(`a foreign ${authority} cannot dismiss an active preview`, async () => {
    const r = harness();
    r.intent(r.anchor()); r.advance(350);
    const data = { type: "dismiss-reader-ui" };
    if (authority === "session") data.sessionId = "retired-session";
    r.message(data, authority === "source" ? { source: {} } : authority === "origin" ? { origin: "https://foreign.invalid" } : {});
    assert.equal(r.dismissals, 0);
    await r.finish(0);
    assert.equal(r.preview.article.title, "target.html");
  });
}

test("session replacement still invalidates the prior session's in-flight result", async () => {
  const r = harness();
  r.intent(r.anchor()); r.advance(350);
  r.shell.activeFrame.current = { ...r.session, sessionId: "replacement-session" };
  await r.finish(0);
  assert.equal(r.preview?.article ?? null, null);
});
