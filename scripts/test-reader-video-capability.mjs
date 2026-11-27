import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { randomUUID } from "node:crypto";
import ts from "typescript";
import { ReaderMediaCheckpoint, validMediaPlaybackPositions } from "../src/reader-media-checkpoint.ts";
import { shouldKeepReaderForBackgroundMedia } from "../src/media-policy.ts";

// Execute the production bridge in a minimal archive document. Simulate a
// WebKit without native WebM support; current iPhone WebKit also has a native
// WebM path, which must remain under the archive player's control.
const bridge = await readFile(
  new URL("../src/reader-bridge.js", import.meta.url),
  "utf8",
);
const parentOrigin = "http://shell.localhost:8123";
const archiveOrigin = "http://archive.localhost:8123";
const noticeText =
  "This video uses a format this device cannot play smoothly. It may stutter. The same video plays smoothly in the desktop app.";

function element(tagName, { className = "", parent = null } = {}) {
  const node = {
    nodeType: 1,
    tagName,
    className,
    dataset: {},
    style: {},
    attributes: new Map(),
    children: [],
    parentElement: parent,
    isConnected: parent ? parent.isConnected : true,
    textContent: "",
    listeners: new Map(),
    getAttribute(name) {
      return this.attributes.has(name) ? this.attributes.get(name) : null;
    },
    setAttribute(name, value) {
      this.attributes.set(name, String(value));
    },
    hasAttribute(name) {
      return this.attributes.has(name);
    },
    removeAttribute(name) {
      this.attributes.delete(name);
    },
    addEventListener(type, callback) {
      const callbacks = this.listeners.get(type) ?? [];
      callbacks.push(callback);
      this.listeners.set(type, callbacks);
    },
    removeEventListener() {},
    dispatch(type, event = {}) {
      for (const callback of this.listeners.get(type) ?? []) callback(event);
    },
    querySelectorAll(selector) {
      if (selector === "source")
        return this.children.filter((child) => child.tagName === "SOURCE");
      if (selector === "canvas")
        return this.children.filter((child) => child.tagName === "CANVAS");
      if (selector === "video")
        return this.children.filter((child) => child.tagName === "VIDEO");
      return [];
    },
    querySelector(selector) {
      return this.querySelectorAll(selector)[0] ?? null;
    },
    insertBefore(child, reference) {
      const index = reference ? this.children.indexOf(reference) : -1;
      if (index < 0) this.children.push(child);
      else this.children.splice(index, 0, child);
      child.parentElement = this;
      child.isConnected = this.isConnected;
      return child;
    },
    append(...nodes) {
      for (const child of nodes) this.insertBefore(child, null);
    },
    appendChild(child) {
      return this.insertBefore(child, null);
    },
    remove() {
      const siblings = this.parentElement?.children;
      const index = siblings ? siblings.indexOf(this) : -1;
      if (index >= 0) siblings.splice(index, 1);
      this.parentElement = null;
      this.isConnected = false;
    },
    closest() {
      return null;
    },
  };
  return node;
}

function reader({ nested = false, origin = archiveOrigin, path = "/index.html" } = {}) {
  const documentListeners = new Map();
  const windowListeners = new Map();
  const observers = [];
  const computed = [];
  const events = [];
  const videos = [];
  const childFrames = [];
  const messages = [];
  let deliverParent = () => {};
  const root = element("HTML");
  const head = element("HEAD");
  const body = element("BODY", { parent: root });
  root.children.push(head, body);
  const document = {
    currentScript: { dataset: { parentOrigin } },
    readyState: "loading",
    documentElement: root,
    head,
    body: null,
    title: "Archive",
    hidden: false,
    contentType: "text/html",
    createElement(tag) {
      const node = element(tag.toUpperCase());
      node.isConnected = false;
      return node;
    },
    querySelectorAll(selector) {
      if (selector === "video" || selector === "video,audio")
        return videos.filter((video) => video.isConnected);
      if (selector === "iframe,frame") return childFrames.filter((frame) => frame.isConnected);
      return [];
    },
    addEventListener(type, callback) {
      const callbacks = documentListeners.get(type) ?? [];
      callbacks.push(callback);
      documentListeners.set(type, callbacks);
    },
    removeEventListener(type, callback) {
      documentListeners.set(type, (documentListeners.get(type) || []).filter((entry) => entry !== callback));
    },
    dispatch(type, event = {}) {
      for (const callback of documentListeners.get(type) ?? []) callback(event);
    },
  };
  const parent = {
    location: { origin: nested ? origin : parentOrigin },
    postMessage(message, origin) {
      assert.equal(origin, nested ? parent.location.origin : parentOrigin);
      messages.push(message);
      deliverParent(message);
    },
  };
  const window = {
    parent,
    scrollX: 0,
    scrollY: 0,
    addEventListener(type, callback) {
      const callbacks = windowListeners.get(type) ?? [];
      callbacks.push(callback);
      windowListeners.set(type, callbacks);
    },
    removeEventListener() {},
    dispatch(type, event = {}) {
      for (const callback of windowListeners.get(type) ?? []) callback(event);
    },
    getSelection: () => null,
  };
  class FakeMutationObserver {
    constructor(callback) {
      this.callback = callback;
      this.options = null;
      observers.push(this);
    }
    observe(target, options) {
      this.target = target;
      this.options = options;
    }
    disconnect() {
      this.disconnected = true;
    }
  }
  const history = { pushState() {}, replaceState() {} };
  const context = vm.createContext({
    window,
    parent,
    document,
    location: {
      origin,
      href: new URL(path, origin).href,
    },
    navigator: {},
    history,
    MutationObserver: FakeMutationObserver,
    EventTarget: class {},
    Event: class {},
    MouseEvent: class {},
    WheelEvent: class {},
    DOMException,
    URL,
    HTMLAnchorElement: class { get href() { return this.url; } },
    HTMLAreaElement: class { get href() { return this.url; } },
    Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    crypto: { randomUUID },
    WeakRef,
    WeakSet,
    WeakMap,
    getComputedStyle(node) {
      computed.push(node);
      events.push({ op: "read", node });
      // A default border is `currentColor`: once the theme marks the element's
      // ink, its border colour follows the themed text colour.
      const borderColor =
        node.dataset?.kiwixInk !== undefined
          ? "rgb(232, 235, 239)"
          : "rgb(0, 0, 0)";
      return {
        backgroundColor:
          node.getAttribute?.("data-bg") ??
          (node.getAttribute?.("data-neutral") !== null
            ? "rgb(255, 255, 255)"
            : "rgba(0, 0, 0, 0)"),
        backgroundImage: "none",
        color: node.getAttribute?.("data-color") ?? "rgb(0, 0, 0)",
        borderTopColor: borderColor,
        borderRightColor: borderColor,
        borderBottomColor: borderColor,
        borderLeftColor: borderColor,
        display: "block",
        visibility: "visible",
        contentVisibility: "visible",
      };
    },
    performance: { now: () => 0 },
    setTimeout,
    clearTimeout,
    queueMicrotask,
    requestAnimationFrame() {},
    cancelAnimationFrame() {},
  });
  Object.defineProperty(context.MouseEvent.prototype, "ctrlKey", { get() { return false; } });
  Object.defineProperty(context.WheelEvent.prototype, "deltaY", { get() { return 0; } });
  vm.runInContext(bridge, context, { filename: "reader-bridge.js" });
  window.location = context.location;
  document.body = body;
  const mediaObserver = observers[0];
  const frame = {
    document,
    root,
    head,
    body,
    videos,
    observers,
    computed,
    events,
    documentListeners,
    windowListeners,
    mediaObserver,
    messages,
    window,
    parent,
    setParentTransport(deliver) { deliverParent = deliver; },
    location: context.location,
    receiveChild(child, data, { source = child.contentWindow, origin: messageOrigin = origin } = {}) {
      window.dispatch("message", { source, origin: messageOrigin,
        data: { channel: "kiwix-reader-media", version: 1, ...data } });
    },
    addChild() {
      const sent = [];
      const child = element("IFRAME");
      child.contentDocument = {};
      child.contentWindow = { location: { origin, href: new URL("/embedded", origin).href },
        postMessage(data, origin) {
          assert.equal(origin, child.contentWindow.location.origin);
          sent.push(data);
        } };
      child.sent = sent;
      childFrames.push(child);
      frame.receiveChild(child, { type: "hello", session: "" });
      return child;
    },
    sessionId: "session-1",
    init(mediaNotice = { text: noticeText, dismiss: "Dismiss" }) {
      window.dispatch("message", {
        isTrusted: true,
        source: parent,
        origin: parentOrigin,
        data: {
          channel: "kiwix-reader",
          version: 1,
          type: "reader-init",
          sessionId: frame.sessionId,
          sensorStatus: { nativeLocation: false, location: "notAsked" },
          mediaNotice,
        },
      });
    },
    command(command) {
      window.dispatch("message", {
        isTrusted: true,
        source: parent,
        origin: parentOrigin,
        data: {
          channel: "kiwix-reader",
          version: 1,
          type: "reader-command",
          sessionId: frame.sessionId,
          ...command,
        },
      });
    },
    theme(name = "dark") {
      frame.command({
        command: "appearance",
        theme: name,
        fontFamily: "original",
        fontSize: 16,
      });
      return observers.find(
        (observer) =>
          observer.options?.attributes === true &&
          observer.options?.attributeFilter?.includes("class"),
      );
    },
    addVideo({
      type = "",
      playable = false,
      error = null,
      autoplay = false,
      paused = true,
      src = "",
      className = "video-js",
    } = {}) {
      const container = element("DIV", { className });
      body.insertBefore(container, null);
      const video = element("VIDEO", { parent: container });
      video.autoplay = autoplay;
      if (autoplay) video.setAttribute("autoplay", "");
      video.paused = paused;
      video.pause = () => {
        video.paused = true;
      };
      video.canPlayType = (mime) => (playable ? "probably" : "");
      video.error = error;
      if (src) video.src = src;
      container.insertBefore(video, null);
      let source = null;
      if (type) {
        source = element("SOURCE", { parent: video });
        source.setAttribute("type", type);
        video.insertBefore(source, null);
      }
      videos.push(video);
      mediaObserver.callback([
        { type: "childList", addedNodes: [video], target: container },
      ]);
      return { container, video, source };
    },
    notices() {
      return body.children.filter(
        (child) => child.getAttribute("data-kiwix-notice") !== null,
      );
    },
  };
  return frame;
}

const [appSource, readerSource] = await Promise.all([
  readFile(new URL("../src/App.tsx", import.meta.url), "utf8"),
  readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8"),
]);
const appTree = ts.createSourceFile("App.tsx", appSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const readerTree = ts.createSourceFile("Reader.tsx", readerSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
function findNodes(tree, predicate) {
  const matches = [];
  function visit(node) { if (predicate(node)) matches.push(node); ts.forEachChild(node, visit); }
  visit(tree);
  return matches;
}
const rootCheckpointCallbacks = ["mediaCheckpointOwner", "recordMediaCheckpoint", "readMediaCheckpoint"].map((name) => {
  const declaration = findNodes(appTree, (node) => ts.isVariableDeclaration(node) && node.name.getText(appTree) === name)[0];
  return `const ${name} = ${declaration.initializer.arguments[0].getText(appTree)};`;
}).join("\n");
const readerIdentityHelpers = findNodes(readerTree, (node) =>
  ts.isFunctionDeclaration(node) && ["object", "text", "canonicalLocation", "readerDocumentUrl", "articlePath"].includes(node.name?.text) ||
  ts.isVariableStatement(node) && node.declarationList.declarations.some((entry) => entry.name.getText(readerTree) === "MISS_MARKER"),
).map((node) => node.getText(readerTree)).join("\n");
const receiveCheckpoint = findNodes(readerTree, (node) => ts.isVariableDeclaration(node) &&
  node.name.getText(readerTree) === "receive" && ts.isArrowFunction(node.initializer) &&
  node.initializer.getText(readerTree).includes('"media-checkpoint"'))[0].initializer;
const receivePrefix = [];
for (const statement of receiveCheckpoint.body.statements) {
  receivePrefix.push(statement.getText(readerTree));
  if (ts.isIfStatement(statement) && statement.expression.getText(readerTree) === 'data.type === "media-checkpoint"') break;
}
const readyCheckpoint = findNodes(readerTree, (node) => ts.isVariableDeclaration(node) &&
  node.name.getText(readerTree) === "mediaDocument")[0].parent.parent;
const readyStatements = readyCheckpoint.parent.statements;
const readyIndex = readyStatements.indexOf(readyCheckpoint);
const readyRestore = readyStatements.slice(readyIndex, readyIndex + 2).map((node) => node.getText(readerTree)).join("\n");
const leaveDocument = findNodes(readerTree, (node) => ts.isVariableDeclaration(node) &&
  node.name.getText(readerTree) === "leaveDocument")[0].initializer.arguments[0].getText(readerTree);
const mediaSender = findNodes(readerTree, (node) => ts.isVariableDeclaration(node) &&
  node.name.getText(readerTree) === "send")[0].initializer.arguments[0].getText(readerTree);
const suspensionPause = findNodes(readerTree, (node) => ts.isCallExpression(node) &&
  node.expression.getText(readerTree) === "useEffect" && node.arguments[0]?.getText(readerTree).includes('"pause-video"'))[0].arguments[0].getText(readerTree);
const checkpointConsumer = ts.transpileModule(`${rootCheckpointCallbacks}
  ${readerIdentityHelpers}
  const send = ${mediaSender};
  globalThis.scheduleSuspensionPause = ${suspensionPause};
  globalThis.receiveCheckpoint = (event) => { ${receivePrefix.join("\n")} };
  globalThis.authenticatedReady = () => {
    const session = activeFrame.current;
    const data = { url: session.location.url };
    const path = articlePath(data.url, session.location);
    ${readyRestore}
  };
  globalThis.leaveDocument = ${leaveDocument};
  globalThis.savedPositions = (authority) => readMediaCheckpoint("tab-1", authority);
`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

function checkpointHost() {
  const rootSession = { activeTab: "tab-1", activeSurface: "reader", privacyGeneration: 7,
    tabs: [{ id: "tab-1", bookId: "book-1", path: "article" }],
    settings: { privateMode: false, readOnly: false } };
  const books = [{ id: "book-1", installationId: "installation-1", path: "/owned/archive.zim" }];
  const checkpoint = new ReaderMediaCheckpoint();
  const commands = [];
  const ref = (current) => ({ current });
  const timers = [];
  let frame;
  const context = vm.createContext({
    URL, crypto: { randomUUID }, validMediaPlaybackPositions,
    mediaCheckpoint: checkpoint, latestSession: ref(rootSession), mobileBooks: ref(books),
    checkpointTabIndex: { current: { source: null, byId: new Map() } },
    checkpointBookIndex: { current: { source: null, byId: new Map() } },
    mounted: ref(true), privacyPending: ref(false), quitRequest: ref(null),
    observedPrivacy: ref({ mode: false, generation: 1 }), activeFrame: ref(null), frameRef: ref(null),
    latest: ref(null), restoredMediaDocument: ref(""), pendingExport: ref(null),
    panAuthority: ref(null),
    // The receiver's document-identity bookkeeping; no frame is displayed here.
    displayedFrame: ref(null), documentAnnounced: ref(false), documentProbe: ref(null),
    clearDocumentProbe() {}, resessionUnproven() {},
    reestablishAuthority() {},
    isMobile: true, nativeSpeechStop: ref(null), props: { suspended: false },
    setTimeout: (operation) => { timers.push(operation); return timers.length; },
    clearTimeout: () => {},
    setAttachment() {}, previewSequence: ref(0), setPreview() {}, setPanel() {}, report() {},
    deactivateFrame: async () => { context.activeFrame.current = null; },
  });
  vm.runInContext(checkpointConsumer, context);
  const authority = () => ({ bookId: books[0].id, installationId: books[0].installationId,
    article: rootSession.tabs[0].path, privacyGeneration: rootSession.privacyGeneration,
    privacyPolicyGeneration: context.observedPrivacy.current.generation });
  return {
    context, checkpoint, rootSession, books, commands, authority,
    saved: () => context.savedPositions(authority()),
    attach(next, generation = rootSession.privacyGeneration) {
      frame = next;
      context.props.suspended = false;
      rootSession.privacyGeneration = generation;
      context.activeFrame.current = { sessionId: frame.sessionId, frameId: frame.sessionId,
        installationId: books[0].installationId, privacyGeneration: generation,
        location: { bookId: books[0].id, path: "article", url: frame.location.href,
          origin: frame.location.origin, baseUrl: `${frame.location.origin}/` } };
      context.frameRef.current = { contentWindow: frame.window };
      frame.window.postMessage = (message, origin) => {
        assert.equal(origin, next.location.origin);
        commands.push(message);
        next.window.dispatch("message", { isTrusted: true, source: next.parent, origin: parentOrigin, data: message });
      };
      context.latest.current = { settings: rootSession.settings, privacyGeneration: generation,
        privacyPolicyGeneration: context.observedPrivacy.current.generation,
        onMediaCheckpoint: (owner, positions) => context.recordCheckpoint(owner, positions),
        readMediaCheckpoint: (owner) => context.savedPositions(owner),
        clearMediaCheckpoint: () => checkpoint.clear() };
      // Run the production root callback through a bound, root-owned tab.
      vm.runInContext("globalThis.recordCheckpoint = (owner, positions) => recordMediaCheckpoint('tab-1', owner, positions);", context);
      frame.setParentTransport((message) => {
        context.receiveCheckpoint({ isTrusted: true, data: message, source: next.window, origin: next.location.origin });
      });
      frame.init();
    },
    ready() { context.authenticatedReady(); },
    backgroundAdmission() { context.activeFrame.current = null; },
    runSuspensionPause() {
      context.props.suspended = true;
      const cleanup = context.scheduleSuspensionPause();
      timers.shift()?.();
      cleanup?.();
    },
    explicitRetirement() { context.leaveDocument(); },
  };
}

function embeddedPlayer(top, time = 0, duration = 100) {
  const child = reader({ nested: true, origin: top.location.origin, path: "/embedded" });
  const { video } = child.addVideo({ paused: time === 0, playable: true,
    src: `${top.location.origin}/movie.mp4` });
  Object.assign(video, { currentTime: time, duration, muted: false, volume: 1, readyState: 4 });
  const entry = top.addChild();
  const previous = child.parent.postMessage.bind(child.parent);
  child.parent.postMessage = (message, origin) => {
    previous(message, origin);
    top.receiveChild(entry, message);
  };
  let delivered = 0;
  return { child, entry, video,
    pump() {
      while (delivered < entry.sent.length) child.window.dispatch("message", {
        source: child.parent, origin: top.location.origin, data: entry.sent[delivered++],
      });
    } };
}

const sleep = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

test("unsupported WebM shows one dismissible quiet notice", async () => {
  const frame = reader();
  // The bridge runs before the shell's reader-init: safeguards apply, but no
  // notice can be painted until the localized copy arrives.
  const { container, video } = frame.addVideo({ type: "video/webm" });
  await sleep(20);
  assert.equal(frame.notices().length, 0, "no copy before reader-init");

  frame.init({ text: "Localized copy from the shell", dismiss: "Dismiss" });
  await sleep(20);
  const notices = frame.notices();
  assert.equal(notices.length, 1, "one notice per element");
  const notice = notices[0];
  assert.equal(notice.getAttribute("role"), "status");
  assert.equal(notice.children[0].textContent, "Localized copy from the shell");
  assert.equal(notice.children[1].textContent, "Dismiss");
  assert.equal(notice.children[1].getAttribute("aria-label"), "Dismiss");
  assert.equal(notice.parentElement, frame.body);
  assert.equal(
    frame.body.children.indexOf(notice) + 1,
    frame.body.children.indexOf(container),
    "the notice sits in flow above the player, never over the controls",
  );
  assert.doesNotMatch(notice.style.cssText, /red|#f00|rgb\(2[0-5]\d,\s*0/i);
  assert.doesNotMatch(notice.style.cssText, /position\s*:/i);
  assert.equal(video.getAttribute("playsinline"), "");
  assert.equal(video.getAttribute("webkit-playsinline"), "");
  assert.equal(container.getAttribute("data-kiwix-media-surface"), "");

  // Dismissing is final for that element, even if the archive re-adds it.
  notice.children[1].dispatch("click");
  assert.equal(frame.notices().length, 0);
  frame.mediaObserver.callback([
    { type: "childList", addedNodes: [video], target: container },
  ]);
  await sleep(20);
  assert.equal(frame.notices().length, 0);
});

test("an H.264 MP4 on the same WebKit keeps native playback untouched", async () => {
  const frame = reader();
  frame.init();
  const { video } = frame.addVideo({
    type: "video/mp4",
    playable: true,
    autoplay: true,
    paused: false,
  });
  await sleep(20);
  assert.equal(frame.notices().length, 0);
  assert.equal(video.getAttribute("preload"), null, "no forced buffering change");
  assert.equal(video.hasAttribute("autoplay"), true);
  assert.equal(video.paused, false);
  assert.equal(video.getAttribute("playsinline"), "");
});

test("an unsupported source error still shows the notice without a declared type", async () => {
  const frame = reader();
  frame.init();
  const { video } = frame.addVideo({ error: { code: 4 } });
  await sleep(20);
  assert.equal(frame.notices().length, 1);
});

test("a network error or an unknown blob source never claims a codec problem", async () => {
  const frame = reader();
  frame.init();
  frame.addVideo({ error: { code: 2 } });
  frame.addVideo({ src: "blob:http://archive.localhost/1234" });
  await sleep(20);
  assert.equal(frame.notices().length, 0);
});

test("gesture-started media pauses when the reader hides", async () => {
  const frame = reader();
  frame.init();
  // A person has already interacted with the article, so the start decision
  // belongs to the archive and only the visibility policy may stop it.
  frame.document.dispatch("pointerdown", { isTrusted: true });
  const { video } = frame.addVideo({
    type: "video/webm",
    autoplay: true,
    paused: false,
  });
  await sleep(20);
  assert.equal(video.paused, false, "a gesture-initiated start is not killed");
  frame.document.hidden = true;
  frame.document.dispatch("visibilitychange", {});
  assert.equal(video.paused, true, "the existing media policy pauses it");
});

test("the theme pass never visits the player's decoder nodes", async () => {
  const frame = reader();
  frame.init();
  const article = element("DIV", { parent: frame.body });
  article.setAttribute("data-neutral", "");
  frame.body.insertBefore(article, null);
  const { container, video } = frame.addVideo({ type: "video/webm" });
  await sleep(20);
  const canvas = element("CANVAS", { parent: container });
  container.insertBefore(canvas, null);
  const source = video.children[0];
  const notice = frame.notices()[0];
  assert.ok(notice, "the notice exists before theming");

  const themeObserver = frame.theme("dark");
  assert.ok(themeObserver, "theme observer is running");
  // video.js repaints class/style on the wrapper every progress tick and appends
  // canvas nodes; the theme scan must ignore all of it.
  themeObserver.callback([
    { type: "attributes", target: container, attributeName: "class" },
    { type: "childList", target: container, addedNodes: [canvas] },
    { type: "childList", target: video, addedNodes: [element("SOURCE")] },
  ]);
  await sleep(40);

  for (const node of [video, container, canvas, source, notice])
    assert.ok(
      !frame.computed.includes(node),
      `${node.tagName} must never reach getComputedStyle`,
    );
  for (const node of [video, container, canvas, source, notice]) {
    assert.equal(node.dataset.kiwixSurface, undefined);
    assert.equal(node.dataset.kiwixInk, undefined);
    assert.equal(node.dataset.kiwixEdge, undefined);
  }
  assert.equal(
    article.dataset.kiwixSurface,
    "page",
    "ordinary article surfaces are still themed",
  );
});

function watchThemeWrites(frame, node) {
  node.dataset = new Proxy(node.dataset, {
    set(target, key, value) {
      frame.events.push({ op: "write", kind: "set", node });
      target[key] = value;
      return true;
    },
    deleteProperty(target, key) {
      frame.events.push({ op: "write", kind: "delete", node });
      delete target[key];
      return true;
    },
  });
}

test("the theme scan batches style reads instead of alternating per element", async () => {
  const frame = reader();
  frame.init();
  const article = element("DIV", { parent: frame.body });
  frame.body.insertBefore(article, null);
  const nodes = [];
  const watch = (node) => {
    node.setAttribute("data-neutral", "");
    watchThemeWrites(frame, node);
    nodes.push(node);
  };
  watch(article);
  for (let index = 0; index < 300; index++) {
    const child = element("DIV", { parent: article });
    article.insertBefore(child, null);
    watch(child);
  }
  frame.events.length = 0;
  frame.theme("dark");
  await sleep(100);

  const tree = new Set(nodes);
  const ops = frame.events
    .filter((event) => tree.has(event.node))
    .map((event) => event.op);
  assert.equal(
    ops.filter((op) => op === "read").length,
    nodes.length * 2,
    "each surface is sampled once for its paint and once for its borders",
  );
  let readRun = 0;
  let readRuns = 0;
  let maxReadRun = 0;
  for (const op of ops) {
    if (op === "read") {
      readRun++;
      maxReadRun = Math.max(maxReadRun, readRun);
    } else if (readRun) {
      readRuns++;
      readRun = 0;
    }
  }
  if (readRun) readRuns++;
  // Batching means many consecutive getComputedStyle calls with no dataset
  // write between them. Per-element scanning would never exceed a run of one.
  assert.ok(
    maxReadRun >= 32,
    `style reads must be batched, not interleaved per element (longest run ${maxReadRun})`,
  );
  assert.ok(
    readRuns <= Math.ceil(nodes.length / 32),
    `reads must be grouped into batches (${readRuns} runs)`,
  );
  for (const node of nodes) {
    assert.equal(node.dataset.kiwixSurface, "page");
    assert.equal(node.dataset.kiwixInk, "");
    assert.equal(
      node.dataset.kiwixEdge,
      "",
      "borders are sampled after ink, matching the per-element scan",
    );
  }
});

test("an inverted mutation rescan classifies an ancestor before its descendant", async () => {
  const frame = reader();
  frame.init();
  const ancestor = element("DIV", { parent: frame.body });
  frame.body.insertBefore(ancestor, null);
  const descendant = element("DIV", { parent: ancestor });
  ancestor.insertBefore(descendant, null);
  const nodes = [ancestor, descendant];
  for (const node of nodes) watchThemeWrites(frame, node);
  // A coloured backdrop is preserved, so the child inherits a preserved
  // backdrop and its neutral text keeps its colour.
  ancestor.setAttribute("data-bg", "rgb(0, 0, 255)");
  const themeObserver = frame.theme("dark");
  assert.ok(themeObserver, "theme observer is running");
  await sleep(40);
  assert.equal(ancestor.dataset.kiwixSurface, undefined, "coloured backdrop stays authored");
  assert.equal(descendant.dataset.kiwixInk, undefined, "text over a preserved backdrop keeps its colour");

  // Flip the ancestor to a neutral surface and deliver the mutation records
  // descendant-first in separate callbacks, so only the batch reorder can
  // protect the descendant from the stale preserved backdrop.
  ancestor.setAttribute("data-bg", "rgb(255, 255, 255)");
  frame.events.length = 0;
  themeObserver.callback([
    { type: "attributes", target: descendant, attributeName: "class" },
  ]);
  themeObserver.callback([
    { type: "attributes", target: ancestor, attributeName: "class" },
  ]);
  await sleep(40);

  assert.equal(ancestor.dataset.kiwixSurface, "page");
  assert.equal(
    descendant.dataset.kiwixInk,
    "",
    "the descendant uses the ancestor's fresh backdrop",
  );
  const firstRead = new Map();
  for (let index = 0; index < frame.events.length; index++) {
    const event = frame.events[index];
    if (event.op === "read" && nodes.includes(event.node) && !firstRead.has(event.node))
      firstRead.set(event.node, index);
  }
  assert.ok(
    firstRead.get(ancestor) < firstRead.get(descendant),
    "the ancestor is classified before the descendant",
  );
  for (const node of nodes) {
    const first = frame.events.find((event) => event.node === node);
    assert.equal(first.op, "write", "marks are cleared before the style read");
    assert.equal(first.kind, "delete");
  }
});

const lastMedia = (frame) => frame.messages.findLast((message) =>
  message.type === "media-state" || message.type === "state");

test("descendant media requires the actual child source, origin and current document session", () => {
  const frame = reader();
  frame.init();
  const child = frame.addChild();
  const session = child.sent.findLast((message) => message.type === "init").session;
  const state = { type: "state", session, audible: true, pictureInPicture: true };
  frame.receiveChild(child, state, { source: {} });
  frame.receiveChild(child, state, { origin: "https://untrusted.invalid" });
  frame.receiveChild(child, { ...state, session: "retired" });
  assert.equal(lastMedia(frame).audible, false);
  frame.receiveChild(child, state);
  assert.equal(lastMedia(frame).audible, true);
  assert.equal(lastMedia(frame).pictureInPicture, true);
  // A child is never permitted to forward a shell command or a sensor request.
  const count = frame.messages.length;
  frame.receiveChild(child, { ...state, type: "sensor-request", sensor: "camera" });
  frame.window.dispatch("message", { source: child.contentWindow, origin: archiveOrigin,
    data: { channel: "kiwix-reader", version: 1, sessionId: frame.sessionId,
      type: "reader-init" } });
  assert.equal(frame.messages.length, count);
});

test("finishing an unchanged child document load preserves its audible media session", () => {
  const frame = reader();
  frame.init();
  const child = frame.addChild();
  const session = child.sent.findLast((message) => message.type === "init").session;
  frame.receiveChild(child, { type: "state", session, audible: true, pictureInPicture: false });
  const reportStart = frame.messages.length;
  const commandStart = child.sent.length;
  frame.document.dispatch("load", { target: child });
  assert.equal(child.sent.at(-1).type, "probe");
  assert.equal(lastMedia(frame).audible, true, "load completion never drops background ownership");
  frame.receiveChild(child, { type: "hello", session: "" });
  assert.equal(child.sent.at(-1).type, "init");
  assert.equal(child.sent.at(-1).session, session, "same document receives its existing token");
  assert.equal(child.sent.slice(commandStart).some((message) =>
    ["pause", "pause-video", "retire"].includes(message.type)), false);
  assert.equal(frame.messages.slice(reportStart).some((message) =>
    message.type === "media-state" && !message.audible), false);
  frame.receiveChild(child, { type: "state", session, audible: true, pictureInPicture: false });
  assert.equal(lastMedia(frame).audible, true);
});

test("child removal, document navigation, retirement and shell-session replacement clear ownership", () => {
  for (const retirement of ["removed", "navigated", "retired", "session"]) {
    const frame = reader();
    frame.init();
    const child = frame.addChild();
    const session = child.sent.findLast((message) => message.type === "init").session;
    const state = { type: "state", session, audible: true, pictureInPicture: false };
    frame.receiveChild(child, state);
    assert.equal(lastMedia(frame).audible, true);
    if (retirement === "removed") {
      child.isConnected = false;
      frame.mediaObserver.callback([{ addedNodes: [], removedNodes: [child] }]);
    } else if (retirement === "navigated") {
      child.contentDocument = {};
      frame.document.dispatch("load", { target: child });
    } else if (retirement === "retired") frame.receiveChild(child, { type: "retired", session });
    else {
      frame.sessionId = "session-2";
      frame.init();
    }
    assert.equal(lastMedia(frame).audible, false, retirement);
    frame.receiveChild(child, state);
    assert.equal(lastMedia(frame).audible, false, `late ${retirement} state`);
    if (retirement === "navigated") {
      frame.receiveChild(child, { type: "hello", session: "" });
      const replacement = child.sent.findLast((message) => message.type === "init").session;
      assert.notEqual(replacement, session);
      frame.receiveChild(child, { ...state, session: replacement });
      assert.equal(lastMedia(frame).audible, true, "new child document can acquire ownership");
    }
  }
});

test("an embedded archive aggregates grandchildren only on its media channel and retires on teardown", () => {
  const frame = reader({ nested: true });
  assert.equal(frame.messages[0].type, "hello");
  const child = frame.addChild();
  const grandchildSession = child.sent.findLast((message) => message.type === "init").session;
  frame.receiveChild(child, { type: "state", session: grandchildSession,
    audible: true, pictureInPicture: true });
  assert.equal(frame.messages.some((message) => message.type === "state"), false,
    "no state before authenticated parent initialization");
  frame.window.dispatch("message", { source: frame.parent, origin: archiveOrigin,
    data: { channel: "kiwix-reader-media", version: 1, type: "init", session: "parent-session" } });
  assert.equal(lastMedia(frame).audible, true);
  assert.equal(lastMedia(frame).pictureInPicture, true);
  assert.equal(lastMedia(frame).session, "parent-session");
  frame.window.dispatch("pagehide");
  assert.equal(frame.messages.at(-1).type, "retired");
  assert.equal(child.sent.at(-1).type, "retire");
  assert.equal(frame.messages.some((message) => message.channel === "kiwix-reader"), false);
});

test("native WebKit PiP survives background video policy and reports ownership without the standard PiP API", () => {
  const frame = reader();
  const { video } = frame.addVideo({ paused: false, playable: true });
  Object.assign(video, { muted: true, volume: 0, currentTime: 37, readyState: 4, webkitPresentationMode: "inline" });
  frame.init();
  assert.equal(lastMedia(frame).pictureInPicture, false);
  video.webkitPresentationMode = "picture-in-picture";
  frame.document.dispatch("webkitpresentationmodechanged", { target: video });
  assert.equal(lastMedia(frame).pictureInPicture, true, "native mode change owns PiP even for muted playback");
  frame.document.hidden = true;
  frame.document.dispatch("visibilitychange");
  frame.command({ command: "pause-video" });
  assert.equal(video.paused, false, "both visibility and delayed shell policy preserve actual native PiP");
  assert.equal(video.currentTime, 37);
  assert.equal(lastMedia(frame).pictureInPicture, true);
  video.webkitPresentationMode = "inline";
  frame.document.dispatch("webkitpresentationmodechanged", { target: video });
  assert.equal(lastMedia(frame).pictureInPicture, false, "leaving native PiP removes retention ownership");
  frame.command({ command: "pause-video" });
  assert.equal(video.paused, true, "non-PiP video still follows background pause policy");
});

test("native WebKit PiP does not survive explicit document/media retirement", () => {
  for (const exitThrows of [false, true]) {
    const frame = reader();
    const { video } = frame.addVideo({ paused: false, playable: true });
    const exits = [];
    Object.assign(video, { muted: false, volume: 1, currentTime: 12, readyState: 4,
      webkitPresentationMode: "picture-in-picture",
      webkitSetPresentationMode(mode) {
        exits.push(mode);
        if (exitThrows) throw new Error("native presentation is already closing");
        this.webkitPresentationMode = mode;
      } });
    frame.init();
    assert.equal(lastMedia(frame).pictureInPicture, true);
    frame.window.dispatch("pagehide");
    assert.deepEqual(exits, ["inline"]);
    assert.equal(video.paused, true, "failed native PiP exit must not bypass media pause");
  }
});

test("embedded inline playback checkpoints survive either background admission ordering and restore paused in a fresh authenticated reader", () => {
  for (const ordering of ["pause-first", "admission-first"]) {
    const host = checkpointHost();
    const old = reader({ path: "/article" });
    old.sessionId = "old-session";
    host.attach(old);
    host.ready();
    const player = embeddedPlayer(old, 6);
    player.pump();
    player.child.document.dispatch("timeupdate", { target: player.video });
    assert.equal(host.saved()[0].time, 6);
    if (ordering === "admission-first") host.backgroundAdmission();
    player.child.document.hidden = true;
    player.child.document.dispatch("visibilitychange");
    old.document.hidden = true;
    old.document.dispatch("visibilitychange");
    player.pump(); // Includes the real delayed shell policy's descendant pause.
    host.runSuspensionPause();
    player.pump();
    assert.equal(player.video.paused, true);
    assert.equal(lastMedia(old).audible, false, "paused data must never pretend to be audible");
    assert.equal(lastMedia(old).pictureInPicture, false);
    assert.equal(shouldKeepReaderForBackgroundMedia({
      ...lastMedia(old), privateMode: false, readOnly: false, retiring: false, quitting: false,
    }), false, "the existing quiet-origin retirement policy is unchanged");
    if (ordering === "pause-first") host.backgroundAdmission();
    old.window.dispatch("pagehide");
    player.pump();

    const fresh = reader({ path: "/article", origin: "http://replacement.localhost:9123" });
    fresh.sessionId = "new-session";
    host.attach(fresh, 8);
    const replacement = embeddedPlayer(fresh, 0, NaN);
    replacement.pump();
    host.ready();
    replacement.pump();
    assert.equal(replacement.video.currentTime, 0, "metadata has not authorized the cursor yet");
    replacement.video.duration = 100;
    replacement.child.document.dispatch("loadedmetadata", { target: replacement.video });
    replacement.pump();
    assert.equal(replacement.video.currentTime, 6, ordering);
    assert.equal(replacement.video.paused, true, "data restoration never calls play");
    assert.equal(lastMedia(fresh).audible, false);
    const restores = host.commands.filter((command) => command.command === "restore-media");
    assert.equal(restores.length, 1);
    assert.equal(host.commands.every((command) => ["restore-media", "pause-video"].includes(command.command)), true,
      "cursor restoration issues no native/speech command");
    assert.equal(restores[0].url, fresh.location.href);
    assert.doesNotMatch(JSON.stringify(restores[0].positions), /old-session|archive\.localhost:8123/);
    assert.equal(host.saved()[0].time, 6);
    replacement.video.currentTime = 0;
    replacement.child.document.dispatch("seeked", { target: replacement.video });
    assert.equal(host.saved()[0].time, 0, "a later explicit seek to zero replaces the old cursor");
  }
});

test("checkpoint messages cannot borrow a replacement frame, source, tab or policy owner", () => {
  const host = checkpointHost();
  const frame = reader({ path: "/article" });
  host.attach(frame);
  host.ready();
  const { video } = frame.addVideo({ paused: false, playable: true, src: `${frame.location.origin}/movie.mp4` });
  Object.assign(video, { currentTime: 6, duration: 100, muted: false, volume: 1, readyState: 4 });
  frame.document.dispatch("timeupdate", { target: video });
  const checkpoint = frame.messages.findLast((message) => message.type === "media-checkpoint");
  assert.equal(host.saved()[0].time, 6);
  const forged = { ...checkpoint, positions: [{ ...checkpoint.positions[0], time: 80 }] };
  host.context.receiveCheckpoint({ data: forged, source: {}, origin: frame.location.origin });
  host.context.receiveCheckpoint({ data: forged, source: frame.window, origin: "https://unrelated.invalid" });
  host.context.receiveCheckpoint({ data: { ...forged, sessionId: "retired" }, source: frame.window, origin: frame.location.origin });
  assert.equal(host.saved()[0].time, 6);
  host.backgroundAdmission();
  host.context.receiveCheckpoint({ data: forged, source: frame.window, origin: frame.location.origin });
  assert.equal(host.saved()[0].time, 6, "retired callbacks cannot recreate or overwrite the checkpoint");
  host.books[0].installationId = "replacement-installation";
  assert.deepEqual(host.saved(), [], "a replacement installation invalidates the old data");
  host.books[0].installationId = "installation-1";
  assert.deepEqual(host.saved(), [], "old authority cannot revive invalidated data");
});

test("explicit retirement and policy/source/article/tab changes invalidate the single bounded root checkpoint", () => {
  const owner = { tabId: "tab-1", bookId: "book-1", installationId: "installation-1",
    article: "article", sourceIdentity: "/owned/archive.zim", privacyPolicyGeneration: 1 };
  const position = { frames: [0], document: "/embedded", source: "/movie.mp4", index: 0, time: 6, duration: 100 };
  for (const changed of [
    { tabId: "tab-2" }, { bookId: "other-book" }, { installationId: "replacement" },
    { article: "other-article" }, { sourceIdentity: "/replacement/archive.zim" }, { privacyPolicyGeneration: 2 },
  ]) {
    const checkpoint = new ReaderMediaCheckpoint();
    checkpoint.record(owner, [position]);
    assert.deepEqual(checkpoint.read({ ...owner, ...changed }), []);
    assert.deepEqual(checkpoint.read(owner), []);
  }
  const host = checkpointHost();
  const frame = reader({ path: "/article" });
  host.attach(frame);
  host.ready();
  const { video } = frame.addVideo({ paused: false, playable: true, src: `${frame.location.origin}/movie.mp4` });
  Object.assign(video, { currentTime: 6, duration: 100 });
  frame.document.dispatch("timeupdate", { target: video });
  assert.equal(host.saved()[0].time, 6);
  host.rootSession.settings.readOnly = true;
  assert.deepEqual(Array.from(host.saved()), []);
  host.rootSession.settings.readOnly = false;
  host.rootSession.settings.privateMode = true;
  assert.deepEqual(Array.from(host.saved()), []);
  host.rootSession.settings.privateMode = false;
  host.explicitRetirement();
  assert.deepEqual(Array.from(host.saved()), []);
});

test("malformed/external/internal-authority cursors are dropped and root data remains bounded and detached", () => {
  const checkpoint = new ReaderMediaCheckpoint();
  const owner = { tabId: "tab-1", bookId: "book-1", installationId: "installation-1",
    article: "article", sourceIdentity: "source", privacyPolicyGeneration: 1 };
  const position = { frames: [], document: "/article", source: "/movie.mp4", index: 0, time: 6, duration: 100 };
  for (const changed of [
    { time: NaN }, { time: Infinity }, { time: -1 }, { time: 101 }, { duration: Infinity },
    { source: "https://external.invalid/movie.mp4" }, { source: "/.kiwix/old-token/raw/movie" }, { index: 16 },
    { frames: [64] }, { frames: [-1] }, { frames: [0.5] }, { frames: Array(33).fill(0) },
  ]) checkpoint.record(owner, [{ ...position, ...changed }]);
  assert.deepEqual(checkpoint.read(owner), []);
  checkpoint.record(owner, Array.from({ length: 17 }, (_, index) => ({ ...position, index })));
  assert.deepEqual(checkpoint.read(owner), []);
  checkpoint.record(owner, Array.from({ length: 16 }, (_, index) => ({ ...position, index })));
  checkpoint.record(owner, [{ ...position, source: "/another.mp4", oldToken: "not-data" }]);
  const read = checkpoint.read(owner);
  assert.equal(read.length, 16);
  assert.equal(read.some((position) => "oldToken" in position), false);
  assert.equal(read.find((position) => position.index === 0).source, "/another.mp4",
    "source replacement in one occurrence replaces its old resource rather than accumulating it");
  read[0].time = 90;
  assert.equal(checkpoint.read(owner)[0].time, 6, "callers cannot mutate root-owned checkpoint data");
  read[0].frames.push(1);
  assert.equal(checkpoint.read(owner)[0].frames.length, 0, "lineage data is detached from root ownership");
});

test("metadata callbacks cannot seek a replaced element or a retired document and mismatched duration never seeks", () => {
  for (const boundary of ["element", "session", "pagehide", "duration", "source"]) {
    const frame = reader({ path: "/article" });
    const { video } = frame.addVideo({ paused: true, playable: true, src: `${frame.location.origin}/movie.mp4` });
    Object.assign(video, { currentTime: 0, duration: NaN });
    frame.init();
    frame.command({ command: "restore-media", url: frame.location.href, requestId: randomUUID(),
      positions: [{ frames: [], document: "/article", source: "/movie.mp4", index: 0, time: 6, duration: 100 }] });
    const delayed = frame.observers.at(-1).callback;
    let target = video;
    if (boundary === "element") {
      video.isConnected = false;
      target = frame.addVideo({ paused: true, playable: true, src: `${frame.location.origin}/movie.mp4` }).video;
      Object.assign(target, { currentTime: 0, duration: 100 });
    } else if (boundary === "session") {
      frame.sessionId = "replacement-session";
      frame.init();
    } else if (boundary === "pagehide") frame.window.dispatch("pagehide");
    else if (boundary === "source") video.src = `${frame.location.origin}/replacement.mp4`;
    video.duration = boundary === "duration" ? 101 : 100;
    delayed();
    frame.document.dispatch("loadedmetadata", { target });
    assert.equal(target.currentTime, 0, boundary);
    assert.equal(target.paused, true);
    if (boundary === "source") {
      video.src = `${frame.location.origin}/movie.mp4`;
      frame.document.dispatch("loadedmetadata", { target: video });
      assert.equal(video.currentTime, 0, "returning to an old URL cannot revive a consumed callback");
    }
  }
});

test("a player refusing pause never receives a cursor seek or a later stale retry", () => {
  const frame = reader({ path: "/article" });
  const { video } = frame.addVideo({ paused: false, playable: true, src: `${frame.location.origin}/movie.mp4` });
  Object.assign(video, { currentTime: 0, duration: 100, pause() {} });
  frame.init();
  frame.command({ command: "restore-media", url: frame.location.href, requestId: randomUUID(),
    positions: [{ frames: [], document: "/article", source: "/movie.mp4", index: 0, time: 6, duration: 100 }] });
  assert.equal(video.currentTime, 0);
  assert.equal(video.paused, false);
  video.paused = true;
  frame.document.dispatch("loadedmetadata", { target: video });
  assert.equal(video.currentTime, 0, "a failed initial pause cannot rewind later user playback");
});

test("identical embedded player occurrences retain distinct cursors and independent metadata acknowledgments", () => {
  const host = checkpointHost();
  const old = reader({ path: "/article" });
  old.sessionId = "old-occurrences";
  host.attach(old);
  host.ready();
  const first = embeddedPlayer(old, 6);
  const second = embeddedPlayer(old, 21);
  first.pump();
  second.pump();
  assert.deepEqual(host.saved().map((position) => [position.frames[0], position.time]), [[0, 6], [1, 21]]);
  host.backgroundAdmission();
  old.window.dispatch("pagehide");
  first.pump();
  second.pump();

  const fresh = reader({ path: "/article", origin: "http://fresh-occurrences.localhost:9124" });
  fresh.sessionId = "new-occurrences";
  host.attach(fresh, 8);
  const nextFirst = embeddedPlayer(fresh, 0, NaN);
  const nextSecond = embeddedPlayer(fresh, 0, NaN);
  nextFirst.pump();
  nextSecond.pump();
  host.ready();
  nextFirst.pump();
  nextSecond.pump();
  nextFirst.video.duration = 100;
  nextFirst.child.document.dispatch("loadedmetadata", { target: nextFirst.video });
  nextFirst.pump();
  assert.equal(nextFirst.video.currentTime, 6);
  assert.equal(nextSecond.video.currentTime, 0, "the first occurrence cannot consume the second cursor");
  const restore = host.commands.find((message) => message.command === "restore-media");
  const firstSession = nextFirst.entry.sent.find((message) => message.type === "init").session;
  fresh.receiveChild(nextFirst.entry, { type: "restore-consumed", session: firstSession,
    requestId: restore.requestId, keys: [JSON.stringify([[1], "/embedded", "/movie.mp4", 0])] });
  nextSecond.video.duration = 100;
  nextSecond.child.document.dispatch("loadedmetadata", { target: nextSecond.video });
  nextSecond.pump();
  nextFirst.pump();
  assert.equal(nextSecond.video.currentTime, 21, "sibling acknowledgment cannot erase a pending seek");
  assert.equal(nextFirst.video.currentTime, 6);
  assert.equal(nextFirst.video.paused, true);
  assert.equal(nextSecond.video.paused, true);
});
