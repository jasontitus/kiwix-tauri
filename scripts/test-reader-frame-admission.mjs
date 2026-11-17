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
      ["clearHistoryTraversal", "historySourceCurrent", "reopenHistoryTraversal", "travel"].includes(node.name.getText(tree)))
    callbacks.set(node.name.getText(tree), node.initializer.arguments[0]);
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(frameElement && receive, "exercise the production frame and message receiver");
const helpers = new Set(["object", "text", "readerDocumentUrl", "articlePath", "canonicalLocation", "passageDocumentKey", "passageDocumentPath", "currentLocation", "lastLocationIndex", "sameHistoryStack", "sensorKey"]);
const moduleVariables = new Set(["MISS_MARKER", "nextNativeRequest"]);
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
  const context = vm.createContext({
    URL, crypto, clearTimeout,
    window: { setTimeout }, pendingHistoryTraversal: ref(null), freshOrigin: ref(false),
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
    isMobile: true, isDesktop: false, isWindows: false, isMacos: false, isIOS: false, photoSecretFor: () => "photo-secret",
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
  return { context, node, messages, calls, failures, ready,
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
  const r = reader();
  r.load(); r.ready();
  const outgoing = r.context.activeFrame.current.sessionId;
  const gate = r.gateDeactivate();
  r.load();
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
  assert.equal(r.messages.at(-1).data.sessionId, incoming);
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
    const r = reader();
    r.load(); r.ready();
    const gate = r.gateDeactivate();
    r.load(); await turn(); retire(r); gate.resolve(); await r.drain();
    assertPending(r);
    assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate"]);
    assert.equal(r.context.activeFrame.current, null);
  }
});

test("a stale activation is deactivated without publishing readiness or initializing the archive", async () => {
  const r = reader();
  r.load(); r.ready();
  const gate = r.gateActivate();
  r.load(); await turn();
  r.context.latest.current.privacyGeneration++;
  gate.resolve(); await r.drain();
  assertPending(r);
  assert.deepEqual(r.calls.map((item) => item.command), ["sensor_deactivate", "sensor_activate", "sensor_deactivate"]);
  assert.equal(r.context.activeFrame.current, null);
  assert.equal(r.messages.length, 1, "stale activation cannot init the archive");
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
