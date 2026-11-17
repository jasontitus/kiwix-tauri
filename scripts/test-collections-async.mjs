import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Exercise the shipped handlers with deferred I/O. Extracting the functions
// avoids replacing the runtime with a second implementation of their logic.
async function handlers(path, names) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true,
    path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const found = new Map();
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text))
      found.set(node.name.text, node.getText(tree).replace(/^export /, ""));
    ts.forEachChild(node, visit);
  }
  visit(tree);
  for (const name of names) assert.ok(found.has(name), `Missing handler: ${name}`);
  return ts.transpileModule([...found.values()].join("\n"), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
}
const collectionCode = await handlers("../src/components/Collections.tsx", ["save", "clearList", "importList"]);
const pickerCode = await handlers("../src/api.ts", ["readTextFile"]);
const bookmark = () => ({ id: "one", bookId: "book", path: "A/Article", title: "Title", note: "Original", tags: ["saved"] });
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function collection(overrides = {}) {
  const state = { edit: bookmark(), busy: false, refreshed: 0, errors: [], notice: "" };
  const context = vm.createContext({
    edit: state.edit, busy: false, readOnlyRef: { current: false },
    bookmarks: [bookmark(), { ...bookmark(), id: "two" }],
    capturePrivacyContext: () => ({ generation: 7 }),
    isPrivacyContextCurrent: () => true,
    confirmAction: async () => true,
    t: (key) => key,
    api: async () => {},
    setBusy: (value) => { state.busy = value; },
    setEdit: (value) => { state.edit = typeof value === "function" ? value(state.edit) : value; },
    setNotice: (value) => { state.notice = value; },
    onRefresh: () => { state.refreshed++; },
    onError: (error) => { state.errors.push(error); },
    ...overrides,
  });
  vm.runInContext(collectionCode, context);
  return { state, context };
}

for (const scenario of ["same draft", "newer text", "another article", "reopened same article", "dismissed"]) {
  test(`save completion preserves editor ownership: ${scenario}`, async () => {
    const write = deferred();
    let submitted;
    const { state, context } = collection({ api: async (command, args) => {
      assert.equal(command, "save_bookmark");
      submitted = args.bookmark;
      await write.promise;
    } });
    const pending = context.save({ preventDefault() {} });
    if (scenario === "newer text") state.edit = { ...state.edit, note: "Typed after Save" };
    if (scenario === "another article") state.edit = { ...state.edit, id: "two", note: "Another draft" };
    if (scenario === "reopened same article") state.edit = { ...state.edit };
    if (scenario === "dismissed") state.edit = null;
    const latest = state.edit;
    write.resolve();
    await pending;
    assert.equal(submitted.note, "Original");
    assert.equal(state.edit, scenario === "same draft" ? null : latest);
    assert.equal(state.busy, false);
    assert.equal(state.refreshed, 1);
  });
}

test("a failed save preserves the draft and reports the failure", async () => {
  const failure = new Error("Storage is full");
  const { state, context } = collection({ api: async () => { throw failure; } });
  const original = state.edit;
  await context.save({ preventDefault() {} });
  assert.equal(state.edit, original);
  assert.deepEqual(state.errors, [failure]);
  assert.equal(state.busy, false);
  assert.equal(state.refreshed, 0);
});

for (const blocked of ["busy", "readOnly"]) {
  test(`save does not dispatch while ${blocked}`, async () => {
    let writes = 0;
    const { context } = collection({
      busy: blocked === "busy",
      readOnlyRef: { current: blocked === "readOnly" },
      api: async () => { writes++; },
    });
    await context.save({ preventDefault() {} });
    assert.equal(writes, 0);
  });
}

for (const failingId of ["one", "two"]) {
  test(`clear reading list reconciles a failed response for ${failingId}`, async () => {
    const attempted = [], committed = [];
    const failure = new Error("Response lost after committing deletion");
    const { state, context } = collection({ api: async (command, args) => {
      assert.equal(command, "remove_bookmark");
      assert.equal(args.privacyGeneration, 7);
      attempted.push(args.id);
      committed.push(args.id);
      if (args.id === failingId) throw failure;
    } });
    await context.clearList();
    assert.deepEqual(attempted, failingId === "one" ? ["one"] : ["one", "two"]);
    assert.ok(committed.length);
    assert.equal(state.refreshed, 1, "refresh must reflect partial or ambiguous committed writes");
    assert.deepEqual(state.errors, [failure]);
    assert.equal(state.busy, false);
  });
}

test("clear reading list refreshes once after success and respects declined confirmation", async () => {
  const success = collection();
  await success.context.clearList();
  assert.equal(success.state.refreshed, 1);
  let writes = 0;
  const cancelled = collection({ confirmAction: async () => false, api: async () => { writes++; } });
  await cancelled.context.clearList();
  assert.equal(writes, 0);
  assert.equal(cancelled.state.refreshed, 0);
});

test("a privacy cutover stops the remaining batch and suppresses stale callbacks", async () => {
  let current = true, writes = 0;
  const { state, context } = collection({
    isPrivacyContextCurrent: () => current,
    api: async () => { writes++; current = false; },
  });
  await context.clearList();
  assert.equal(writes, 1);
  assert.equal(state.refreshed, 0);
  assert.equal(state.busy, false);
});

function picker() {
  const input = { click() {} };
  const context = vm.createContext({
    isMobile: false, isNative: false,
    document: { createElement: (tag) => { assert.equal(tag, "input"); return input; } },
  });
  vm.runInContext(pickerCode, context);
  return { input, readTextFile: context.readTextFile };
}

test("browser text picker returns selected contents", async () => {
  const { input, readTextFile } = picker();
  const pending = readTextFile();
  input.files = [{ text: async () => '{"bookmarks":[]}' }];
  input.onchange();
  assert.equal(await pending, '{"bookmarks":[]}');
});

for (const event of ["cancel", "empty selection"]) {
  test(`browser text picker settles ${event}`, async () => {
    const { input, readTextFile } = picker();
    const pending = readTextFile();
    if (event === "cancel") input.oncancel();
    else { input.files = []; input.onchange(); }
    assert.equal(await pending, null);
  });
}

test("unreadable browser file rejects through import and releases its busy state", { timeout: 1000 }, async () => {
  const { input, readTextFile } = picker();
  let imported = 0;
  const failure = new Error("Selected file is no longer readable");
  const { state, context } = collection({ readTextFile, api: async () => { imported++; } });
  const pending = context.importList();
  assert.equal(state.busy, true);
  input.files = [{ text: async () => { throw failure; } }];
  input.onchange();
  await pending;
  assert.equal(imported, 0);
  assert.deepEqual(state.errors, [failure]);
  assert.equal(state.busy, false);
  assert.equal(state.refreshed, 0);
});
