import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const tree = ts.createSourceFile("reader-bridge.js", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
let factory;
const bridgeFunctions = new Map();
let historyWrappers;
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === "createDocumentHistory") factory = node;
  if (ts.isFunctionDeclaration(node) &&
      ["createDocumentHistory", "resetDocumentHistory", "expectHashNavigation", "traverseDocumentHistory",
        "finishDocumentTraversal", "documentHistoryChanged", "navigationChanged", "stopAnnotationObservation"].includes(node.name?.text))
    bridgeFunctions.set(node.name.text, node);
  if (ts.isForOfStatement(node) && ts.isArrayLiteralExpression(node.expression) &&
      node.expression.elements.length === 2 &&
      node.expression.elements.every((item) => ts.isStringLiteral(item)) &&
      node.expression.elements.map((item) => item.text).join(",") === "pushState,replaceState")
    historyWrappers = node;
  ts.forEachChild(node, visit);
}
visit(tree);
const createHistory = vm.runInNewContext(`${factory.getText(tree)}; createDocumentHistory`, { URL });

test("live history retains native entry distances when consecutive entries share a URL", () => {
  const history = createHistory("article?view=a#first");
  history.push("article?view=a#first");
  history.push("article?view=b#second");
  history.push("article?view=b#second");
  history.push("spa-route?view=c#third");
  assert.equal(history.delta("back", ["article?view=b#second", "article?view=a#first"]), -3);
  assert.equal(history.commit(-3, "article?view=a#first"), true);
  assert.equal(history.url, "article?view=a#first");
  assert.equal(history.delta("forward", ["article?view=b#second", "spa-route?view=c#third"]), 3);
});

test("multi-step traversal distinguishes repeated non-adjacent destinations", () => {
  const history = createHistory("article#a");
  history.push("article#b");
  history.push("article#a");
  history.push("article#c");
  assert.equal(history.delta("back", ["article#a"]), -1);
  assert.equal(history.delta("back", ["article#a", "article#b", "article#a"]), -3);
  assert.equal(history.commit(-3, "article#a"), true);
  assert.equal(history.delta("forward", ["article#b", "article#a", "article#c"]), 3);
});

test("a new branch discards retired forward entries and unknown navigation loses traversal authority", () => {
  const history = createHistory("first");
  history.push("second"); history.push("third");
  assert.equal(history.commit(-1, "second"), true);
  history.push("new-branch");
  assert.equal(history.delta("back", ["second"]), -1);
  assert.equal(history.delta("forward", ["third"]), null);
  history.reset("untracked-route");
  assert.equal(history.delta("back", ["new-branch"]), null);
  assert.equal(history.delta("forward", ["third"]), null);
});

test("replaceState changes the current entry without adding a traversal and a mismatching pop cannot commit", () => {
  const history = createHistory("article?query=one");
  history.push("article?query=two");
  history.replace("article?query=replaced#fragment");
  assert.equal(history.delta("back", ["article?query=one"]), -1);
  assert.equal(history.commit(-1, "foreign-route"), false);
  assert.equal(history.url, "article?query=replaced#fragment");
  assert.equal(history.commit(-1, "article?query=one"), true);
  assert.equal(history.delta("forward", ["article?query=replaced#fragment"]), 1);
});

test("canonical shell punctuation matches the original native URL without rewriting it", () => {
  const base = "http://archive.localhost:8123/";
  const history = createHistory(`${base}index%2Ehtml?view=one`);
  history.push(`${base}other-route.html?view=two#part`);
  assert.equal(history.delta("back", [`${base}index.html?view=one`]), -1);
  assert.equal(history.target(-1), `${base}index%2Ehtml?view=one`);
  assert.equal(history.commit(-1, history.target(-1)), true);
  assert.equal(history.delta("forward", [`${base}other%2Droute%2Ehtml?view=two#part`]), 1);
  assert.equal(history.target(1), `${base}other-route.html?view=two#part`);
});

test("encoded path delimiters, literal percent escapes and query spelling remain distinct", () => {
  const base = "http://archive.localhost:8123/";
  for (const [first, second] of [
    ["page%3Fx=1", "page?x=1"],
    ["page%23part", "page#part"],
    ["literal%252Fentry", "literal%2Fentry"],
    ["page?q=a+b", "page?q=a%20b"],
  ]) {
    const history = createHistory(base + first);
    history.push(base + second);
    assert.equal(history.delta("back", [base + second]), null);
    assert.equal(history.delta("back", [base + first]), -1);
  }
});

function bridge() {
  const events = [];
  const frames = [];
  const deltas = [];
  const origin = "http://archive.localhost:8123";
  const location = { href: `${origin}/first`, origin };
  const native = {
    state: null,
    pushState(state, _unused, url) { this.state = state; if (url != null) location.href = new URL(url, location.href).href; },
    replaceState(state, _unused, url) { this.state = state; if (url != null) location.href = new URL(url, location.href).href; },
    go(delta) { deltas.push(delta); },
  };
  const context = vm.createContext({
    URL, location, history: native, sessionId: "active-session",
    uuid: /^[0-9a-f-]{36}$/, queueMicrotask,
    window: { scrollTo() {} }, document: { title: "Fixture" },
    requestAnimationFrame(callback) { frames.push(callback); },
    post(type, value) { events.push({ type, ...value }); },
    stopSpeech() {}, resetReadingPan() {}, resetReadAloud() {}, paintAnnotations() {}, selectionReport() {},
    headingFragment: () => false, toc: () => [],
    annotationItems: [], annotationReceiving: false, annotationUrl: "", annotationRanges: [],
    annotationObserver: undefined, annotationResize: undefined,
  });
  vm.runInContext(`"use strict";
    ${[...bridgeFunctions.values()].map((node) => node.getText(tree)).join("\n")}
    const documentHistory = createDocumentHistory(location.href);
    let expectedHashNavigation = null;
    let documentTraversal = null;
    ${historyWrappers.getText(tree)}
  `, context);
  return { context, events, frames, deltas, origin,
    push(path) { native.pushState({ path }, "", path); },
    flush: () => new Promise((resolve) => setImmediate(resolve)),
    begin(destinations) {
      context.traverseDocumentHistory({ requestId: "11111111-1111-4111-8111-111111111111",
        url: location.href, direction: "back", destinations, y: 400 });
    },
    arrive(path) { location.href = new URL(path, location.href).href; context.documentHistoryChanged("pop"); },
  };
}

test("synchronous History API mutations report their captured locations in native order", async () => {
  const r = bridge();
  r.push("/second?view=one");
  r.push("/third?view=two");
  await r.flush();
  assert.deepEqual(r.events.map((event) => event.url),
    [`${r.origin}/second?view=one`, `${r.origin}/third?view=two`]);
  r.begin([`${r.origin}/second?view=one`]);
  assert.equal(r.deltas.at(-1), -1);
});

test("popstate replaceState handlers preserve the owned acknowledgment and native state", async () => {
  const r = bridge();
  r.push("/second"); r.push("/third");
  await r.flush(); r.events.length = 0;
  r.begin([`${r.origin}/second`]);
  r.arrive("/second");
  r.context.history.replaceState({ normalized: true }, "", "/second?normalized=1");
  r.context.documentHistoryChanged("hash");
  await r.flush();
  assert.equal(r.events.length, 0);
  r.frames.shift()();
  assert.equal(r.events.length, 1);
  assert.equal(r.events[0].navigationKind, "pop");
  assert.equal(r.events[0].url, `${r.origin}/second?normalized=1`);
  assert.equal(r.events[0].historyRequestId, "11111111-1111-4111-8111-111111111111");
  assert.deepEqual(r.context.history.state, { normalized: true });
});

test("a router push during popstate follows the owned history move rather than replacing it", async () => {
  const r = bridge();
  r.push("/second"); r.push("/third");
  await r.flush(); r.events.length = 0;
  r.begin([`${r.origin}/second`]);
  r.arrive("/second");
  r.push("/redirected");
  r.frames.shift()();
  await r.flush();
  assert.deepEqual(r.events.map((event) => [event.navigationKind, event.url]),
    [["pop", `${r.origin}/second`], ["push", `${r.origin}/redirected`]]);
  assert.equal(r.events[0].historyRequestId, "11111111-1111-4111-8111-111111111111");
  assert.equal(r.events[1].historyRequestId, undefined);
});

test("a known hash push with both native events creates one navigable entry", async () => {
  const r = bridge();
  r.context.expectHashNavigation(`${r.origin}/first#section`);
  r.arrive("/first#section");
  r.context.documentHistoryChanged("hash");
  await r.flush();
  assert.equal(r.events.length, 1);
  assert.equal(r.events[0].navigationKind, "push");
  r.begin([`${r.origin}/first`]);
  assert.equal(r.deltas.at(-1), -1);
});
