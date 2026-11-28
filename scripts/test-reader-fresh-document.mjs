import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// "Open in restricted mode", "Leave restricted mode" and "Reload article" must
// end in a genuinely new document. Assigning a live frame the fragment address
// it already shows fetches nothing (a same-document navigation), so these
// actions retire the document and reopen it in a new browsing context. The
// tests execute the production handlers; the browser primitive itself (a new
// frame element always requests its document) is an engine fact, not emulated.
const source = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Reader.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const declarations = new Map();
let reload;
function visit(node) {
  if (ts.isVariableDeclaration(node) &&
      ["reopenFreshDocument", "toggleRestricted"].includes(node.name.getText(tree)))
    declarations.set(node.name.getText(tree), node.initializer);
  if (ts.isJsxElement(node) && node.openingElement.tagName.getText(tree) === "button" &&
      node.children.some((child) => child.getText(tree).includes('t("Reload article")')))
    reload = node.openingElement.attributes.properties
      .find((attribute) => attribute.name?.getText(tree) === "onClick")?.initializer.expression;
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(declarations.size === 2 && reload, "exercise the production restricted-mode and reload handlers");
const compiled = ts.transpileModule(`
  ${[...declarations].map(([name, value]) => `globalThis.${name} = ${value.getText(tree)};`).join("\n")}
  globalThis.reloadArticle = ${reload.getText(tree)};
`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const ref = (current) => ({ current });
const turn = () => new Promise((resolve) => setImmediate(resolve));

function reader({ registered = true } = {}) {
  const events = [];
  const errors = [];
  const sourceWindow = {};
  // A document whose address carries a fragment: the case that used to be
  // "reloaded" by a same-document navigation.
  const tab = { id: "tab-1", bookId: "book", path: "A#History", title: "A", scroll: 0, back: [], forward: [] };
  const session = { sessionId: "session-1", frameId: "frame-1", privacyGeneration: 7,
    location: { restricted: false, url: "http://archive.localhost:8123/book/A#History" } };
  let finish;
  let fail;
  const retirement = new Promise((resolve, reject) => { finish = resolve; fail = reject; });
  const context = vm.createContext({
    tabRef: ref(tab), latest: ref({ privacyGeneration: 7 }), alive: ref(true),
    frameRef: ref({ contentWindow: sourceWindow }), resolvedKey: ref("resolved"),
    activeFrame: ref(session), openSequence: ref(0),
    frame: session, restricted: false, restrictedBusy: false,
    t: (text) => text,
    deactivateFrame() {
      events.push("deactivate");
      context.openSequence.current++;
      context.activeFrame.current = null;
      return Promise.resolve();
    },
    leaveDocument() { events.push("leave"); void context.deactivateFrame(); },
    discardQuarantinedReaderStorage() { events.push("discard-quarantine"); },
    retireReaderDocuments(options) {
      events.push(["retire", { ...options }]);
      // Retirement synchronously deactivates every frame that registered storage.
      if (registered) void context.deactivateFrame();
      return retirement;
    },
    quarantineReaderStorage(target) { events.push(["quarantine", target === sourceWindow]); },
    async setReaderRestricted(bookId, next) { events.push(["native-restricted", bookId, next]); return next; },
    publishTab(next) { events.push(["tab-restricted", next.restricted]); context.tabRef.current = next; },
    report(error) { errors.push(error); },
    setFrame(value) { events.push(["frame", value]); if (value === null) context.frame = null; },
    setReload(update) { events.push(["reload", update(0)]); },
    setFrameReady(value) { events.push(["ready", value]); },
    setLoading(value) { events.push(["loading", value]); },
    setError(value) { events.push(["error", value]); },
    setSpeaking() {},
    setRestrictedBusy(value) { context.restrictedBusy = value; },
  });
  vm.runInContext(compiled, context, { filename: "Reader-fresh-document.tsx" });
  const names = () => events.map((event) => (Array.isArray(event) ? event[0] : event));
  return { context, events, errors, names, finish, fail, session };
}

test("toggling restricted mode retires the document and reopens it in a new frame", async () => {
  for (const turningOn of [true, false]) {
    const r = reader();
    r.context.restricted = !turningOn;
    const done = r.context.toggleRestricted();
    await turn();
    assert.deepEqual(r.events.slice(0, 2), [["native-restricted", "book", turningOn], ["tab-restricted", turningOn]]);
    assert.deepEqual(r.events.find((event) => event[0] === "retire"), ["retire", { restoreOnFailure: false }],
      "the outgoing document leaves through its real pagehide and storage handoff, and is never restored");
    assert.equal(r.context.resolvedKey.current, "", "an interrupted reopen cannot settle on the old document");
    assert.deepEqual(r.events.find((event) => event[0] === "ready"), ["ready", false]);
    assert.equal(r.names().includes("reload"), false, "nothing reopens before the old document is gone");
    assert.equal(r.context.frame, r.session, "the frame keeps its own served policy until it is replaced");
    assert.equal(r.context.restrictedBusy, true);
    r.finish(); await done;
    const names = r.names();
    assert.deepEqual(names.slice(-2), ["frame", "reload"]);
    assert.deepEqual(r.events.at(-2), ["frame", null], "the browsing context is dropped, so the reopen is a new document");
    assert.equal(names.includes("quarantine"), false);
    assert.deepEqual(r.errors, []);
    assert.equal(r.context.restrictedBusy, false);
  }
});

test("Reload article retires the document and reopens it in a new frame", async () => {
  for (const registered of [true, false]) {
    const r = reader({ registered });
    r.context.reloadArticle();
    assert.deepEqual(r.names().slice(0, 3), ["discard-quarantine", "leave", "deactivate"]);
    assert.ok(r.names().includes("retire"));
    assert.equal(r.context.activeFrame.current, null, "the session is deactivated even without registered storage");
    assert.equal(r.names().includes("reload"), false);
    r.finish(); await turn();
    assert.deepEqual(r.events.slice(-2), [["frame", null], ["reload", 1]]);
    assert.deepEqual(r.errors, []);
  }
});

test("an unconfirmed retirement removes the frame instead of presenting the old document as reloaded", async () => {
  for (const action of ["toggleRestricted", "reloadArticle"]) {
    const r = reader();
    const done = r.context[action]();
    await turn();
    r.fail(new Error("The archive could not finish closing its document."));
    await done; await turn();
    const names = r.names();
    assert.equal(names.includes("reload"), false, `${action} does not reopen over an unretired document`);
    assert.deepEqual(r.events.find((event) => event[0] === "quarantine"), ["quarantine", true]);
    assert.ok(names.indexOf("quarantine") < names.lastIndexOf("frame"), "storage is quarantined before the frame is removed");
    assert.deepEqual(r.events.filter((event) => event[0] === "frame"), [["frame", null]]);
    assert.equal(r.context.frame, null, "no frame is left to carry a restricted banner or a stale page");
    assert.deepEqual(r.events.filter((event) => event[0] === "loading").at(-1), ["loading", false]);
    const shown = r.events.find((event) => event[0] === "error")[1];
    assert.match(shown, /could not confirm that this page finished saving/);
    assert.match(shown, /Choose Reload article to open it again/, "the advice matches the button beside it");
    assert.doesNotMatch(shown, /Close and reopen this window/);
    assert.match(shown, /may be lost\.$/, "the writes may in fact have been saved");
    assert.equal(r.errors.length, 1);
    assert.match(r.errors[0].cause.message, /could not finish closing/);
  }
});

test("a reopen that was overtaken does not replace the newer document", async () => {
  for (const overtake of [
    (r) => { r.context.tabRef.current = { ...r.context.tabRef.current, path: "B" }; },
    (r) => { r.context.tabRef.current = { ...r.context.tabRef.current, bookId: "other" }; },
    (r) => { r.context.latest.current.privacyGeneration++; },
    (r) => { r.context.openSequence.current++; },
    (r) => { r.context.activeFrame.current = { sessionId: "newer" }; },
    (r) => { r.context.alive.current = false; },
  ]) {
    const r = reader();
    r.context.reloadArticle();
    overtake(r);
    r.finish(); await turn();
    assert.equal(r.names().includes("reload"), false);
    assert.equal(r.names().includes("frame"), false);
  }
});
