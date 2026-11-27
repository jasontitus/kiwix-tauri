import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Reader.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let frameElement;
let receive;
const callbacks = new Map();
function visit(node) {
  if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(tree) === "iframe" &&
      node.attributes.getText(tree).includes("rw-archive-frame")) frameElement = node;
  if (ts.isVariableDeclaration(node) && node.name.getText(tree) === "receive") receive = node.initializer;
  if (ts.isVariableDeclaration(node) &&
      ["clearHistoryTraversal", "historySourceCurrent", "reopenHistoryTraversal", "travel", "navigate",
        "clearDocumentProbe", "initializeDocument", "documentReplaced", "resessionFrame", "resessionUnproven", "probeDocument",
        "photoSecretFor"].includes(node.name.getText(tree)))
    callbacks.set(node.name.getText(tree), node.initializer.arguments[0]);
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(frameElement && receive, "exercise the production frame and message receiver");
const helpers = new Set(["object", "text", "readerDocumentUrl", "articlePath", "canonicalLocation", "passageDocumentKey", "passageDocumentPath", "currentLocation", "lastLocationIndex", "sameHistoryStack", "sensorKey", "boundedHistory", "validExternal"]);
const moduleVariables = new Set(["MISS_MARKER", "nextNativeRequest", "MAX_TAB_HISTORY", "DOCUMENT_PROOF_TIMEOUT"]);
const declarations = tree.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && helpers.has(node.name?.text) ||
  ts.isVariableStatement(node) && node.declarationList.declarations.some((item) => moduleVariables.has(item.name.getText(tree))));
const compiled = ts.transpileModule(`${declarations.map((node) => node.getText(tree)).join("\n")}
  ${[...callbacks].map(([name, callback]) => `globalThis.${name} = ${callback.getText(tree)};`).join("\n")}
  globalThis.renderFrame = () => { const frame = displayedFrame.current; return (${frameElement.getText(tree)}); };
  globalThis.receive = ${receive.getText(tree)};`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
}).outputText;

const ref = (current) => ({ current });
const turn = () => new Promise((resolve) => setImmediate(resolve));
function deferred() {
  let resolve;
  const promise = new Promise((accept) => { resolve = accept; });
  return { promise, resolve };
}

// These tests execute the real JSX/onLoad/message handlers and admission state.
// They do not emulate a native AX tree or claim screen-reader acceptance.
function reader() {
  const origin = "http://archive.localhost:8123";
  const location = { origin, bookId: "book", path: "first", url: `${origin}/book/first`, baseUrl: `${origin}/book/` };
  const session = { frameId: "frame-1", contextId: "context-1", sessionId: "session-1",
    privacyGeneration: 7, installationId: "installation-1", location, status: {} };
  const tab = { id: "tab-1", bookId: "book", path: "first", title: "First", scroll: 0, back: [], forward: [] };
  const messages = [];
  const calls = [];
  const failures = [];
  const sourceWindow = { postMessage(data, target) { messages.push({ data, target }); } };
  const node = { contentWindow: sourceWindow, contentDocument: null, dataset: { frameId: session.frameId } };
  let deactivateGate;
  let activateGate;
  let navigationAllowed = true;
  let pending = Promise.resolve();
  const noop = () => {};
  // Owned timers: the proof window and the traversal fallback fire only when a
  // test advances the clock.
  let now = 0;
  let nextTimer = 0;
  const timers = new Map();
  const notices = [];
  const tocs = [];
  const context = vm.createContext({
    URL, crypto, clearTimeout: (id) => { timers.delete(id); },
    window: { setTimeout(callback, delay) { timers.set(++nextTimer, { callback, at: now + delay }); return nextTimer; } },
    pendingHistoryTraversal: ref(null), freshOrigin: ref(false),
    documentAnnounced: ref(null), documentProbe: ref(null), panAuthority: ref(null),
    awaitingGestureHello: ref(true), gestureHello: ref(null),
    photoSecrets: ref(new Map()), heldPaint: "",
    setHeldPaint(value) { context.heldPaint = typeof value === "function" ? value(context.heldPaint) : value; },
    setNotice(value) { notices.push(value); }, setToc(value) { tocs.push(value); },
    React: { createElement(tag, attributes) { return { tag, ...attributes }; } },
    frameReady: false, displayedFrame: ref(session), activeFrame: ref(session), frameRef: ref(node),
    frameNavigation: ref({ node, frameId: session.frameId, session }), initializedFrame: ref(""),
    tab, tabRef: ref(tab), book: { title: "Book" }, bindFrame: noop, settings: { readOnly: false },
    latest: ref({ privacyGeneration: 7, suspended: false, interactive: true, settings: { privateMode: true, readOnly: false },
      books: [{ id: "book", installationId: "installation-1" }], clearMediaCheckpoint: noop }),
    alive: ref(true), openSequence: ref(0), retainedActivation: ref(null), consentRef: ref(null),
    historyNavigationPending: ref(false), scrollPosition: ref(0), restoredDocument: ref(""),
    restoredMediaDocument: ref(""), watchdogFrame: ref(""), resolvedKey: ref(""), reload: 0,
    passageDocumentRef: ref(null), previewSequence: ref(0), scrollTimer: ref(undefined), bookmarkWrite: ref(false),
    settingsRef: ref({ zoom: 1, readerTheme: "original", fontFamily: "original", fontSize: 16 }),
    isMobile: true, isDesktop: false, isWindows: false, isMacos: false, isIOS: false,
    pendingSensors: ref(new Map()), sensorReplies: [], sensorPrompts: [], sensorGrants: [],
    replySensor(request, data) { context.sensorReplies.push({ request, data }); },
    promptSensor(request) { context.sensorPrompts.push(request); },
    confirmSensorGrant(request) { context.sensorGrants.push(request); },
    ERROR_SITES: {}, t: (text) => text, report: (error) => failures.push(error),
    canNavigateReaderStorage: () => navigationAllowed,
    updateReaderStorageLocation: () => navigationAllowed,
    lifecycle(operation) { pending = pending.then(operation); return pending; },
    async api(command, args) {
      calls.push({ command, args });
      if (command === "sensor_deactivate") await deactivateGate?.promise;
      else if (command === "sensor_activate") await activateGate?.promise;
      else throw new Error(`Unexpected native command ${command}`);
      return {};
    },
    setFrameReady(value) { context.frameReady = value; },
    setFrame(value) { context.displayedFrame.current = value; },
    publishTab(value) { context.tabRef.current = value; },
    cancelSensors: noop, resetFind: noop, setLoading: noop, setLinkContext: noop,
    setPassageDocument: noop, setPassageSelection: noop, setSelection: noop,
    setPassageSelectionUnavailable: noop, setPassageStatuses: noop, updateWorkspace: noop,
    setProviderUnavailable: noop, setSpeaking: noop, setError: noop, send: noop,
    rememberVisit: noop, persistVisit: noop, setPreview: noop, setAttachment: noop,
    setPanel: noop, setBookmarkBusy: noop, setExportBusy: noop, setExternal: noop,
    leaveDocument() { calls.push({ command: "leaveDocument" }); },
    reestablishAuthority: noop,
    quarantineReaderStorage: noop,
    async retireReaderDocuments() {},
  });
  vm.runInContext(compiled, context, { filename: "Reader-frame-admission.tsx" });
  function ready(overrides = {}, eventOverrides = {}) {
    context.receive({ source: sourceWindow, origin, data: {
      channel: "kiwix-reader", version: 1, type: "ready", sessionId: context.activeFrame.current?.sessionId,
      url: context.activeFrame.current?.location.url, title: "Article", ...overrides,
    }, ...eventOverrides });
  }
  // What a genuine bridge does: announce at bootstrap, bind its private token
  // while answering reader-init, and answer a probe of the session it holds.
  const post = (data, eventOverrides = {}) => context.receive({
    source: sourceWindow, origin, isTrusted: true,
    data: { channel: "kiwix-reader", version: 1, ...data }, ...eventOverrides });
  const hello = (token = crypto.randomUUID(), eventOverrides = {}) =>
    post({ type: "reading-gesture-hello", token }, eventOverrides);
  const bind = (token) => post({ type: "reading-pan-bind", token,
    sessionId: context.activeFrame.current?.sessionId });
  const probes = () => messages.filter((item) => item.data.type === "document-probe");
  const proof = (token, overrides = {}, eventOverrides = {}) => post({ type: "document-proof",
    sessionId: context.activeFrame.current?.sessionId, nonce: probes().at(-1)?.data.nonce, token, ...overrides },
    eventOverrides);
  function advance(ms) {
    const until = now + ms;
    for (;;) {
      const due = [...timers.entries()].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      now = due[1].at; timers.delete(due[0]); due[1].callback();
    }
    now = until;
  }
  return { context, node, messages, calls, failures, notices, tocs, ready, hello, bind, proof, probes, advance, post,
    inits: () => messages.filter((item) => item.data.type === "reader-init"),
    load: () => context.renderFrame().onLoad({ currentTarget: node }),
    drain: () => pending,
    gateDeactivate() { deactivateGate = deferred(); return deactivateGate; },
    gateActivate() { activateGate = deferred(); return activateGate; },
    forbidNavigation() { navigationAllowed = false; },
  };
}

function assertPending(reader) {
  assert.equal(reader.context.frameReady, false);
}

test("load initializes the archive without publishing readiness before its authenticated reply", () => {
  const r = reader();
  assertPending(r);
  r.load();
  assertPending(r);
  assert.equal(r.messages[0].data.type, "reader-init");
  assert.equal(r.messages[0].target, r.context.activeFrame.current.location.origin);
  r.ready();
  assert.equal(r.context.frameReady, true);
});

test("child navigation withholds readiness until the replacement session authenticates its changed document", async () => {
  const { r } = settled();
  const outgoing = r.context.activeFrame.current.sessionId;
  const gate = r.gateDeactivate();
  r.hello(); r.load();
  assertPending(r);
  r.ready({ sessionId: outgoing });
  assertPending(r);
  await turn();
  assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate"]);
  gate.resolve(); await r.drain();
  const incoming = r.context.activeFrame.current.sessionId;
  assert.notEqual(incoming, outgoing);
  assertPending(r);
  r.ready({ sessionId: outgoing });
  assertPending(r);
  r.ready({}, { origin: "http://unrelated.localhost:8123" });
  assertPending(r);
  r.ready({}, { source: {} });
  assertPending(r);
  r.ready({ url: `${r.context.activeFrame.current.location.origin}/book/second` });
  assert.equal(r.context.frameReady, true);
  assert.equal(r.inits().at(-1).data.sessionId, incoming);
  assert.equal(r.failures.length, 0);
  assert.equal(r.context.tabRef.current.path, "second");
});

test("retired privacy or storage admission cannot reopen a pending document", async () => {
  for (const retire of [
    (r) => { r.context.latest.current.privacyGeneration++; },
    (r) => { r.context.frameRef.current = null; },
    (r) => r.forbidNavigation(),
    (r) => { r.context.openSequence.current++; },
  ]) {
    const { r } = settled();
    const gate = r.gateDeactivate();
    r.hello(); r.load(); await turn(); retire(r); gate.resolve(); await r.drain();
    assertPending(r);
    assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate"]);
    assert.equal(r.context.activeFrame.current, null);
  }
});

test("a stale activation is deactivated without publishing readiness or initializing the archive", async () => {
  const { r } = settled();
  const gate = r.gateActivate();
  r.hello(); r.load(); await turn();
  r.context.latest.current.privacyGeneration++;
  gate.resolve(); await r.drain();
  assertPending(r);
  assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate", "sensor_activate", "sensor_deactivate"]);
  assert.equal(r.context.activeFrame.current, null);
  assert.equal(r.inits().length, 1, "stale activation cannot init the archive");
  assert.equal(r.probes().length, 0);
});

// A session that completed its handshake: the bridge announced itself, bound
// its private token while answering reader-init, and reported ready.
function settled() {
  const r = reader();
  const token = crypto.randomUUID();
  r.hello(token); r.load(); r.bind(token); r.ready();
  assert.equal(r.context.frameReady, true);
  return { r, token, session: r.context.activeFrame.current.sessionId };
}
const pending = (r) => String(r.context.renderFrame().className).includes("is-pending");

test("a frame load no document announced keeps the session once its own bridge proves it", async () => {
  // WebKit fires the frame's load event for an in-page anchor, TOC or footnote jump.
  const { r, token, session } = settled();
  r.load();
  assert.equal(r.probes().length, 1, "the unannounced load is put to the session's bridge");
  assert.equal(r.probes()[0].data.sessionId, session);
  assert.equal(r.probes()[0].target, r.context.activeFrame.current.location.origin);
  assert.equal(r.context.frameReady, true, "the article stays on screen and usable while the proof is awaited");
  assert.equal(pending(r), false);
  r.proof(token);
  r.advance(60_000); await turn(); await r.drain();
  assert.equal(r.context.frameReady, true);
  assert.equal(r.context.activeFrame.current.sessionId, session, "the session survives");
  assert.deepEqual(r.calls, [], "no sensor session is torn down or rebuilt");
  assert.equal(r.inits().length, 1, "the document is not initialised a second time");
  // Every later jump is proved afresh.
  r.load(); r.proof(token); r.load(); r.proof(token);
  r.advance(60_000); await r.drain();
  assert.equal(r.probes().length, 3);
  assert.equal(r.context.activeFrame.current.sessionId, session);
  assert.deepEqual(r.calls, []);
});

test("an unannounced load nobody proves is re-sessioned when the proof window closes", async () => {
  // A document without a bridge, or one whose listeners document.open() erased, cannot answer.
  const { r, session } = settled();
  r.load();
  r.advance(999);
  assert.equal(r.context.activeFrame.current.sessionId, session);
  r.advance(1);
  assert.equal(r.context.activeFrame.current, null, "the old session loses admission");
  assertPending(r);
  await r.drain();
  assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate", "sensor_activate"]);
  assert.equal(r.calls[0].args.sessionId, session);
  assert.notEqual(r.context.activeFrame.current.sessionId, session);
  r.ready({ sessionId: session });
  assertPending(r);
});

test("only the bridge that holds the session can prove an unannounced load", async () => {
  const forgeries = [
    ["another token", (r) => r.proof(crypto.randomUUID())],
    ["no token", (r) => r.proof(undefined)],
    ["a stale nonce", (r, token) => r.proof(token, { nonce: crypto.randomUUID() })],
    ["another session", (r, token) => r.proof(token, { sessionId: "retired-session" })],
    ["another window", (r, token) => r.proof(token, {}, { source: {} })],
    ["another origin", (r, token) => r.proof(token, {}, { origin: "http://unrelated.localhost:8123" })],
    ["a synthetic event", (r, token) => r.proof(token, {}, { isTrusted: false })],
  ];
  for (const [name, forge] of forgeries) {
    const { r, token, session } = settled();
    r.load(); forge(r, token);
    assert.ok(r.context.documentProbe.current, `${name} leaves the load unproven`);
    r.advance(1000); await r.drain();
    assert.notEqual(r.context.activeFrame.current.sessionId, session, name);
    assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate", "sensor_activate"], name);
  }
  // A session whose bridge never bound a token has nothing to be proved with.
  const r = reader();
  r.hello(); r.load(); r.ready();
  const session = r.context.activeFrame.current.sessionId;
  r.load(); r.proof(crypto.randomUUID());
  r.advance(1000); await r.drain();
  assert.notEqual(r.context.activeFrame.current.sessionId, session);
});

test("the proof of an earlier load cannot settle a later one", async () => {
  const { r, token, session } = settled();
  r.load();
  const first = r.probes().at(-1).data.nonce;
  r.load();
  assert.notEqual(r.probes().at(-1).data.nonce, first);
  r.proof(token, { nonce: first });
  assert.ok(r.context.documentProbe.current);
  r.advance(1000); await r.drain();
  assert.notEqual(r.context.activeFrame.current.sessionId, session);
});

test("a document that announces itself after its load is re-sessioned at once, and no proof can undo it", async () => {
  const { r, token, session } = settled();
  r.load();
  assert.equal(r.context.activeFrame.current.sessionId, session);
  r.hello();
  assert.equal(r.context.activeFrame.current, null, "the late announcement does not wait for the proof window");
  assertPending(r);
  r.proof(token, { sessionId: session });
  await r.drain();
  const incoming = r.context.activeFrame.current.sessionId;
  assert.notEqual(incoming, session);
  assert.equal(r.inits().at(-1).data.sessionId, incoming);
  r.advance(60_000); await r.drain();
  assert.equal(r.context.activeFrame.current.sessionId, incoming, "the closed proof window is inert");
  assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate", "sensor_activate"]);
});

test("a forged announcement only costs the page a re-session", async () => {
  const { r, token, session } = settled();
  r.hello();
  assert.equal(r.context.activeFrame.current.sessionId, session, "an announcement alone changes nothing");
  assert.equal(r.context.frameReady, true);
  r.load();
  assert.equal(r.probes().length, 0, "an announced load is never offered the proof");
  assert.equal(r.context.activeFrame.current, null);
  r.proof(token, { sessionId: session });
  await r.drain();
  assert.notEqual(r.context.activeFrame.current.sessionId, session);
  // Announcements are tied to the frame's own window and origin.
  const other = settled();
  other.r.hello(crypto.randomUUID(), { source: {} });
  other.r.hello(crypto.randomUUID(), { origin: "http://unrelated.localhost:8123" });
  other.r.hello(crypto.randomUUID(), { isTrusted: false });
  other.r.hello("not-a-token");
  assert.equal(other.r.context.documentAnnounced.current, other.token, "none of those is recorded");
  other.r.load();
  assert.equal(other.r.probes().length, 1);
});

test("the session's own announcement may follow its load without costing a re-session", async () => {
  // WebKit delivers a quickly loaded document's load event before its bootstrap announcement.
  const r = reader();
  const token = crypto.randomUUID();
  r.load(); r.hello(token); r.bind(token); r.ready();
  const session = r.context.activeFrame.current.sessionId;
  r.load();
  assert.equal(r.probes().length, 1, "the anchor jump is proved, not treated as a navigation");
  // The same announcement arriving inside the proof window settles nothing either way.
  r.hello(token);
  assert.ok(r.context.documentProbe.current);
  assert.equal(r.context.activeFrame.current.sessionId, session);
  r.proof(token);
  r.advance(60_000); await r.drain();
  assert.equal(r.context.activeFrame.current.sessionId, session);
  assert.deepEqual(r.calls, []);
});

test("a navigation whose load precedes its announcement is still re-sessioned without waiting", async () => {
  const { r, session } = settled();
  // The replacing document loads first (it cannot prove the old session), then announces.
  r.load();
  assert.equal(r.context.activeFrame.current.sessionId, session);
  const next = crypto.randomUUID();
  r.hello(next);
  assert.equal(r.context.activeFrame.current, null);
  await r.drain();
  const incoming = r.context.activeFrame.current.sessionId;
  r.bind(next); r.ready();
  // Its own anchor jumps are then proved with its own token.
  r.load(); r.proof(next);
  assert.equal(r.context.documentProbe.current, null);
  assert.equal(r.context.activeFrame.current.sessionId, incoming);
});

test("an announcement for a session no bridge has bound waits for the proof window", async () => {
  const r = reader();
  r.hello(); r.load(); r.ready();
  const session = r.context.activeFrame.current.sessionId;
  r.load(); r.hello();
  assert.equal(r.context.activeFrame.current.sessionId, session, "nothing identifies the announcer yet");
  r.advance(1000); await r.drain();
  assert.notEqual(r.context.activeFrame.current.sessionId, session, "and nothing proved the load, so it re-sessions");
});

test("an announcement made while a re-session is in flight is answered by that re-session's initialisation", async () => {
  const { r, token, session } = settled();
  const gate = r.gateDeactivate();
  r.hello(); r.load(); await turn();
  // The frame navigated again before the new session was issued.
  const next = crypto.randomUUID();
  r.hello(next); r.load();
  assert.equal(r.inits().length, 1, "a load without an admitted session initialises nothing");
  gate.resolve(); await r.drain();
  const incoming = r.context.activeFrame.current.sessionId;
  assert.notEqual(incoming, session);
  assert.equal(r.inits().length, 2);
  r.bind(next); r.ready();
  assert.equal(r.context.frameReady, true);
  // That document now holds the session: its next anchor jump is proved, not re-sessioned.
  r.load();
  assert.equal(r.probes().length, 1);
  r.proof(token);
  assert.ok(r.context.documentProbe.current, "the replaced document's token proves nothing");
  r.proof(next);
  assert.equal(r.context.documentProbe.current, null);
  assert.equal(r.context.activeFrame.current.sessionId, incoming);
});

test("a proof window does not outlive the session it was opened for", async () => {
  for (const retire of [
    (r) => { r.context.latest.current.privacyGeneration++; },
    (r) => { r.context.frameRef.current = null; },
    (r) => r.forbidNavigation(),
    (r) => { r.context.activeFrame.current = { ...r.context.activeFrame.current, sessionId: "replacement", frameId: "frame-2" }; },
  ]) {
    const { r } = settled();
    r.load(); retire(r);
    const admitted = r.context.activeFrame.current;
    r.advance(1000); await r.drain();
    assert.deepEqual(r.calls, []);
    assert.equal(r.context.activeFrame.current, admitted);
  }
});

test("a child navigation keeps the page painted while it re-sessions, and hides it if that fails", async () => {
  const first = reader();
  first.hello(); first.load();
  assert.equal(pending(first), true, "a shell-opened article stays hidden until its ready");
  first.hello(); first.load(); await first.drain();
  assert.equal(pending(first), true, "a document that was never ready is not revealed by navigating");

  const { r } = settled();
  const gate = r.gateDeactivate();
  r.hello(); r.load();
  assertPending(r);
  assert.equal(r.context.activeFrame.current, null, "admission is still withheld");
  assert.equal(pending(r), false, "the page already on screen is not blanked for the handshake");
  gate.resolve(); await r.drain();
  assert.equal(pending(r), false);
  r.ready({ url: `${r.context.activeFrame.current.location.origin}/book/second` });
  assert.equal(r.context.frameReady, true);
  assert.equal(r.context.heldPaint, "", "readiness, not the hold, keeps the page visible afterwards");

  const failed = settled().r;
  failed.context.api = async () => { throw new Error("sensor backend unavailable"); };
  failed.hello(); failed.load();
  assert.equal(pending(failed), false);
  await failed.drain().catch(() => undefined); await turn();
  assert.equal(pending(failed), true, "a failed re-session falls back to the hidden, pending frame");
  assert.equal(failed.failures.length, 1);

  for (const retire of [
    (r) => { r.context.latest.current.privacyGeneration++; },
    (r) => { r.context.openSequence.current++; },
  ]) {
    const stale = settled().r;
    const hold = stale.gateDeactivate();
    stale.hello(); stale.load(); await turn(); retire(stale); hold.resolve(); await stale.drain();
    assert.equal(pending(stale), true, "an abandoned re-session does not leave an unadmitted page on screen");
  }
});

test("a tab's history keeps only its nearest entries", () => {
  const { r } = settled();
  const base = r.context.activeFrame.current.location.baseUrl;
  for (let index = 0; index < 250; index++)
    r.ready({ type: "navigation", navigationKind: "push", url: `${base}page${index}` });
  const back = [...r.context.tabRef.current.back];
  assert.equal(back.length, 200);
  assert.equal(back.at(-1).path, "page248", "the most recent page is the first way back");
  assert.equal(back[0].path, "page49");
  // Shell navigation and traversal obey the same bound.
  r.context.navigate({ bookId: "book", path: "elsewhere", title: "Elsewhere", scroll: 0 });
  assert.equal(r.context.tabRef.current.back.length, 200);
  assert.equal(r.context.tabRef.current.back.at(-1).path, "page249");
  r.context.frameReady = false;
  r.context.tabRef.current = { ...r.context.tabRef.current,
    forward: Array.from({ length: 200 }, (_, index) => ({ bookId: "book", path: `ahead-${index}`, title: "", scroll: 0 })) };
  r.context.travel("back", 3);
  const forward = [...r.context.tabRef.current.forward];
  assert.equal(forward.length, 200);
  assert.equal(forward.at(-1).path, "page248", "the pages just left are the first way forward");
});

test("only the session being initialised keeps a photo secret", async () => {
  const { r, session } = settled();
  const secret = r.inits()[0].data.photoSecret;
  assert.deepEqual([...r.context.photoSecrets.current.keys()], [session]);
  for (let index = 0; index < 5; index++) {
    const next = crypto.randomUUID();
    r.hello(next); r.load(); await r.drain(); r.bind(next); r.ready();
  }
  assert.equal(r.inits().length, 6, "every navigation was handed a new session");
  const incoming = r.context.activeFrame.current.sessionId;
  assert.deepEqual([...r.context.photoSecrets.current.keys()], [incoming]);
  assert.notEqual(r.inits().at(-1).data.photoSecret, secret);
});

test("blank section titles never become contents entries", () => {
  const { r } = settled();
  r.ready({ toc: [
    { id: "a", title: "History", level: 2 },
    { id: "b", title: "", level: 2 },
    { id: "c", title: " \n\t", level: 3 },
    { id: "d", title: "Notes", level: 2 },
  ] });
  assert.deepEqual(r.tocs.at(-1).map((entry) => entry.id), ["a", "d"]);
});

test("a preview request for the page on screen is dropped, whatever its fragment", () => {
  const { r } = settled();
  const queued = [];
  r.context.latest.current.settings.linkPreviews = true;
  r.context.workspaceRef = ref({ getClientRects: () => [{}] });
  r.context.document = { querySelector: () => null };
  r.context.previewQueued = ref(null);
  r.context.drainPreview = () => queued.push(r.context.previewQueued.current.path);
  const base = r.context.activeFrame.current.location.baseUrl;
  const sessionId = r.context.activeFrame.current.sessionId;
  r.context.tabRef.current = { ...r.context.tabRef.current, path: "first#History" };
  for (const target of ["first", "first#History", "first#cite_note1"])
    r.post({ type: "article-preview", url: `${base}${target}`, sessionId });
  assert.deepEqual(queued, [], "the article already on screen is never previewed");
  r.post({ type: "article-preview", url: `${base}second#History`, sessionId });
  assert.deepEqual(queued, ["second#History"]);
});

test("a link the app cannot open is answered with a quiet notice, never an error band", () => {
  const { r } = settled();
  r.post({ type: "external-link", url: "tel:+15550100", newTab: false, sessionId: r.context.activeFrame.current.sessionId });
  r.post({ type: "unsupported-link", sessionId: r.context.activeFrame.current.sessionId });
  assert.deepEqual(r.failures, [], "nothing failed and nothing needs reloading");
  assert.deepEqual(r.notices, [
    "This type of external link is not permitted.",
    "This type of external link is not permitted.",
  ]);
  const externals = [];
  r.context.setExternal = (value) => externals.push(value);
  r.post({ type: "external-link", url: "mailto:reader@example.org", newTab: false, sessionId: r.context.activeFrame.current.sessionId });
  assert.deepEqual(externals.map((value) => value.url), ["mailto:reader@example.org"], "supported links still reach the confirmation");
});


function traversableReader() {
  const r = reader();
  r.load(); r.ready();
  const source = r.context.activeFrame.current;
  const location = (path, scroll) => ({ bookId: "book", path, title: path, scroll });
  r.context.tabRef.current = {
    ...r.context.tabRef.current, path: "route%2Dc?state=3", scroll: 900,
    back: [location("route%2Da?state=1", 100), location("route%2Db?state=2", 400), location("route%2Da?state=1", 700)],
    forward: [],
  };
  r.context.scrollPosition.current = 900;
  const session = { ...source, location: { ...source.location,
    path: "route%2Dc?state=3", url: `${source.location.baseUrl}route-c?state=3` } };
  r.context.activeFrame.current = session;
  r.context.displayedFrame.current = session;
  r.context.frameNavigation.current.session = session;
  return r;
}

test("owned multi-step traversal commits the selected occurrence once without reopening the live document", () => {
  const r = traversableReader();
  const session = r.context.activeFrame.current;
  r.context.travel("back", 3);
  const pending = r.context.pendingHistoryTraversal.current;
  assert.ok(pending);
  r.context.tabRef.current = {
    ...r.context.tabRef.current,
    back: JSON.parse(JSON.stringify(r.context.tabRef.current.back)),
    forward: JSON.parse(JSON.stringify(r.context.tabRef.current.forward)),
  };
  const reply = { type: "navigation", navigationKind: "pop", historyRequestId: pending.requestId,
    url: `${session.location.baseUrl}route-a?state=1` };
  r.ready(reply);
  const next = r.context.tabRef.current;
  assert.equal(next.path, "route%2Da?state=1");
  assert.equal(next.scroll, 100);
  assert.deepEqual([...next.back], []);
  assert.deepEqual([...next.forward].map((item) => item.path),
    ["route%2Dc?state=3", "route%2Da?state=1", "route%2Db?state=2"]);
  assert.equal(r.context.scrollPosition.current, 100);
  assert.equal(r.context.activeFrame.current.sessionId, session.sessionId);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
  assert.equal(r.context.pendingHistoryTraversal.current, null);
  r.ready(reply);
  assert.equal(r.context.tabRef.current, next, "a duplicate acknowledgment cannot traverse a second time");
});

test("unrelated navigation cancels an owned traversal and makes its later acknowledgment inert", () => {
  const r = traversableReader();
  r.context.travel("back");
  const pending = r.context.pendingHistoryTraversal.current;
  const base = pending.session.location.baseUrl;
  r.ready({ type: "navigation", navigationKind: "push", url: `${base}unrelated` });
  const next = r.context.tabRef.current;
  assert.equal(next.path, "unrelated");
  assert.equal(r.context.pendingHistoryTraversal.current, null);
  r.ready({ type: "navigation", navigationKind: "pop",
    historyRequestId: pending.requestId, url: `${base}route-a?state=1` });
  assert.equal(r.context.tabRef.current, next);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
});

test("an explicit live-segment refusal uses the ordinary reopen path once", () => {
  const r = traversableReader();
  r.context.travel("back");
  const pending = r.context.pendingHistoryTraversal.current;
  const reply = { type: "history-result", accepted: false, requestId: pending.requestId };
  r.ready(reply);
  assert.equal(r.context.tabRef.current.path, "route%2Da?state=1");
  assert.equal(r.context.tabRef.current.scroll, 700);
  assert.equal(r.context.pendingHistoryTraversal.current, null);
  r.ready(reply);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 1);
});

test("a changed privacy generation prevents timeout fallback from reviving the old history source", () => {
  const r = traversableReader();
  r.context.travel("back");
  const pending = r.context.pendingHistoryTraversal.current;
  const before = r.context.tabRef.current;
  r.context.latest.current.privacyGeneration++;
  r.context.reopenHistoryTraversal(pending, true);
  assert.equal(r.context.tabRef.current, before);
  assert.equal(r.context.pendingHistoryTraversal.current, null);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
});

test("timeout fallback waits for genuine retirement and rejects a source changed during that wait", async () => {
  const r = traversableReader();
  const gate = deferred();
  r.context.retireReaderDocuments = () => {
    r.context.activeFrame.current = null;
    r.context.openSequence.current++;
    return gate.promise;
  };
  r.context.travel("back");
  const pending = r.context.pendingHistoryTraversal.current;
  const before = r.context.tabRef.current;
  r.context.reopenHistoryTraversal(pending, true);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
  r.context.latest.current.privacyGeneration++;
  gate.resolve(); await turn();
  assert.equal(r.context.tabRef.current, before);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
});

test("same-entry router replacement commits the owned history move with its final safe location", () => {
  const r = traversableReader();
  r.context.travel("back");
  const pending = r.context.pendingHistoryTraversal.current;
  r.ready({ type: "navigation", navigationKind: "pop", historyRequestId: pending.requestId,
    url: `${pending.session.location.baseUrl}route-a?state=normalized` });
  const next = r.context.tabRef.current;
  assert.equal(next.path, "route%2Da?state=normalized");
  assert.deepEqual([...next.back].map((item) => item.path), ["route%2Da?state=1", "route%2Db?state=2"]);
  assert.deepEqual([...next.forward].map((item) => item.path), ["route%2Dc?state=3"]);
  assert.equal(r.calls.filter((call) => call.command === "leaveDocument").length, 0);
  assert.equal(r.context.pendingHistoryTraversal.current, null);
});

test("resource context accepts only the live archive document and interaction owner", () => {
  function setup() {
    const r = reader();
    const menus = [];
    r.context.workspaceRef = ref({ getClientRects: () => [{}] });
    r.context.document = { querySelector: () => null };
    r.context.setLinkContext = (menu) => menus.push(menu);
    return { r, menus };
  }
  const live = setup();
  const url = `${live.r.context.activeFrame.current.location.baseUrl}media/movie.mp4`;
  const documentUrl = live.r.context.activeFrame.current.location.url;
  live.r.ready({ type: "resource-context", url, documentUrl });
  assert.deepEqual(live.menus.map((menu) => [menu.resource, decodeURIComponent(menu.path)]), [[true, "media/movie.mp4"]]);
  for (const boundary of ["external", "document", "source", "origin", "session", "privacy", "suspended", "noninteractive", "storage", "modal"]) {
    const { r, menus } = setup();
    const message = { type: "resource-context", url, documentUrl };
    const event = {};
    if (boundary === "external") message.url = "https://example.org/movie.mp4";
    if (boundary === "document") message.documentUrl += "?retired";
    if (boundary === "source") event.source = {};
    if (boundary === "origin") event.origin = "http://other.localhost:8123";
    if (boundary === "session") message.sessionId = "retired-session";
    if (boundary === "privacy") r.context.latest.current.privacyGeneration++;
    if (boundary === "suspended") r.context.latest.current.suspended = true;
    if (boundary === "noninteractive") r.context.latest.current.interactive = false;
    if (boundary === "storage") r.forbidNavigation();
    if (boundary === "modal") r.context.document.querySelector = () => ({});
    r.ready(message, event);
    assert.deepEqual(menus, [], boundary);
  }
});

function sensorRequest(r, sensor, requestId = `${sensor}-1`) {
  r.context.receive({ source: r.node.contentWindow, origin: r.context.activeFrame.current.location.origin, data: {
    channel: "kiwix-reader", version: 1, type: "sensor-request",
    sessionId: r.context.activeFrame.current.sessionId, requestId, sensor,
  } });
}

test("location without the app's provider is answered at once, without a consent prompt", () => {
  // A build whose backend reports no provider (status.nativeLocation false).
  for (const status of [{}, { location: "notAsked", nativeLocation: false }, { location: "granted", nativeLocation: false }, { location: "denied" }]) {
    const r = reader();
    r.context.activeFrame.current.status = status;
    sensorRequest(r, "location");
    assert.equal(r.context.sensorPrompts.length, 0, "no prompt for a provider that does not exist");
    assert.equal(r.context.sensorGrants.length, 0, "no cached grant replay either");
    assert.equal(r.context.sensorReplies.length, 1);
    const { data } = r.context.sensorReplies[0];
    assert.equal(data.allowed, false);
    assert.equal(data.error.code, 2, "POSITION_UNAVAILABLE, not a user denial");
    assert.equal(data.cancelled, true, "a watchPosition ends instead of polling");
    assert.equal(r.context.pendingSensors.current.size, 0, "no request is left pending");
  }
});

test("wherever the app has a location provider, it asks first, and cameras are unaffected", () => {
  // The platform flags no longer decide: Windows, macOS and iOS all report a provider.
  for (const platform of [null, "isWindows", "isIOS"]) {
    const r = reader();
    if (platform) r.context[platform] = true;
    r.context.activeFrame.current.status = { location: "notAsked", nativeLocation: true };
    sensorRequest(r, "location");
    assert.equal(r.context.sensorReplies.length, 0, `${platform}`);
    assert.equal(r.context.sensorPrompts.length, 1, `${platform}`);
    assert.equal(r.context.pendingSensors.current.size, 1, `${platform}`);
  }
  const granted = reader();
  granted.context.activeFrame.current.status = { location: "granted", nativeLocation: true };
  sensorRequest(granted, "location");
  assert.equal(granted.context.sensorGrants.length, 1, "a remembered grant is re-checked, not re-prompted");
  assert.equal(granted.context.sensorPrompts.length, 0);
  const r = reader();
  r.context.activeFrame.current.status = { camera: "notAsked", nativeLocation: false };
  sensorRequest(r, "camera");
  assert.equal(r.context.sensorReplies.length, 0);
  assert.equal(r.context.sensorPrompts.length, 1, "a camera request still reaches the consent prompt");
});

test("the bridge only intercepts single-file inputs whose accept list is supported photos", async () => {
  const bridge = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
  const start = bridge.indexOf("function photoInput(element)");
  const end = bridge.indexOf("\n  function userActive()", start);
  assert.ok(start >= 0 && end > start, "locate the production photo input gate");
  const context = vm.createContext({
    photoNatives: {
      type: (element) => element.type,
      disabled: (element) => element.disabled,
      accept: (element) => element.accept,
      multiple: (element) => element.multiple,
    },
    HTMLInputElement: class {},
  });
  vm.runInContext(`${bridge.slice(start, end)}\nglobalThis.photoInput = photoInput;`, context);
  const gate = (patch = {}) => context.photoInput(Object.assign(new context.HTMLInputElement(), {
    type: "file", disabled: false, accept: "image/*", multiple: false, ...patch,
  }));
  assert.equal(gate(), true);
  assert.equal(gate({ accept: "image/png" }), true);
  assert.equal(gate({ accept: "image/gif" }), true);
  assert.equal(gate({ accept: "image/*,application/pdf" }), false, "mixed lists keep the browser picker");
  assert.equal(gate({ accept: "image/svg+xml" }), false, "SVG is not a photo");
  assert.equal(gate({ accept: "image/tiff" }), false, "unsupported image types keep the browser picker");
  assert.equal(gate({ multiple: true }), false, "multi-select keeps the browser picker");
  assert.equal(gate({ accept: "" }), false, "an unconstrained input is not known to be a photo");
  assert.equal(gate({ disabled: true }), false);
  assert.equal(gate({ type: "text" }), false);
});
