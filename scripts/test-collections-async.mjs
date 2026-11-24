import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import React, { useId, useRef, useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";

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
    fileName: path,
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
}
const collectionCode = await handlers("../src/components/Collections.tsx", ["save", "clearList", "importList", "exportCollection"]);
const pagedCode = await handlers("../src/components/UI.tsx", ["PagedResults"]);
const binaryCode = await handlers("../src/api.ts", ["saveBytes", "saveResource", "saveMhtml"]);
const pickerCode = await handlers("../src/api.ts", ["readTextFile"]);
const bookmark = () => ({ id: "one", bookId: "book", path: "A/Article", title: "Title", note: "Original", tags: ["saved"], tagsText: "saved" });
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function collection(overrides = {}) {
  const state = { edit: bookmark(), busy: false, refreshed: 0, errors: [], notice: "", importError: "" };
  const context = vm.createContext({
    edit: state.edit, busy: false, readOnlyRef: { current: false },
    bookmarks: [bookmark(), { ...bookmark(), id: "two" }],
    capturePrivacyContext: () => ({ generation: 7 }),
    isPrivacyContextCurrent: () => true,
    isPrivacyPolicyContextCurrent: () => true,
    confirmAction: async () => true,
    t: (key) => key,
    api: async () => {},
    setBusy: (value) => { state.busy = value; },
    setEdit: (value) => { state.edit = typeof value === "function" ? value(state.edit) : value; },
    setNotice: (value) => { state.notice = value; },
    setImportError: (value) => { state.importError = value; },
    reportError: () => true,
    diagnostics: {},
    errorText: (error) => error instanceof Error ? error.message : String(error),
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

function picker(overrides = {}) {
  const input = { click() {} };
  const context = vm.createContext({
    isMobile: false, isNative: false,
    isPrivacyContextCurrent: () => true,
    document: { createElement: (tag) => { assert.equal(tag, "input"); return input; } },
    ...overrides,
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
  assert.deepEqual(state.errors, []);
  assert.match(state.importError, /Selected file is no longer readable/);
  assert.equal(state.busy, false);
  assert.equal(state.refreshed, 0);
});

test("browser import enforces the byte limit before reading the selected file", async () => {
  for (const size of [16 * 1024 * 1024, 16 * 1024 * 1024 + 1]) {
    const { input, readTextFile } = picker();
    let reads = 0;
    const pending = readTextFile({ generation: 7 });
    input.files = [{ size, text: async () => { reads++; return "allowed"; } }];
    input.onchange();
    if (size === 16 * 1024 * 1024) assert.equal(await pending, "allowed");
    else await assert.rejects(pending, /16 MiB/);
    assert.equal(reads, Number(size === 16 * 1024 * 1024));
  }
});

test("mobile text handoff waits for native foreground generation and retains its initiating policy", async () => {
  for (const changed of [false, true]) {
    const calls = [];
    const snapshot = deferred();
    let policyCurrent = true;
    const requested = deferred();
    const { readTextFile } = picker({
      isMobile: true, privacyGeneration: 7,
      isPrivacyPolicyContextCurrent: () => policyCurrent,
      api: async (command) => {
        calls.push(command);
        if (command === "snapshot") { requested.resolve(); return snapshot.promise; }
        return "selected document";
      },
    });
    const pending = readTextFile({ generation: 7 });
    await requested.promise;
    assert.deepEqual(calls, ["mobile_read_text", "snapshot"]);
    policyCurrent = !changed;
    snapshot.resolve({});
    assert.equal(await pending, changed ? null : "selected document");
  }
});

test("import retains picker policy while using the active foreground generation", async () => {
  for (const changedPolicy of [true, false]) {
    const calls = [];
    let generation = 7;
    const { context, state } = collection({
      capturePrivacyContext: () => ({ generation }),
      isPrivacyPolicyContextCurrent: (privacy) => !changedPolicy || privacy.generation === generation,
      readTextFile: async () => { generation++; return "contents"; },
      api: async (_command, args) => { calls.push(args.privacyGeneration); return 1; },
    });
    await context.importList();
    assert.deepEqual(calls, changedPolicy ? [] : [8]);
    assert.equal(state.busy, false);
    assert.equal(state.refreshed, Number(!changedPolicy));
  }
});

for (const kind of ["bookmarks", "history"]) {
  test(`a late ${kind} export cannot open a file picker after a privacy change`, async () => {
    const result = deferred();
    let current = true;
    const saved = [];
    const { context, state } = collection({
      kind, api: () => result.promise, saveText: (...args) => saved.push(args),
      isPrivacyContextCurrent: () => current,
    });
    const pending = context.exportCollection("json");
    current = false;
    result.resolve("old browsing data");
    await pending;
    assert.deepEqual(saved, []);
    assert.equal(state.busy, false);
  });
}


test("resource exports retain the reader installation through each platform handoff", async () => {
  const privacy = { generation: 7 };
  for (const platform of ["mobile", "desktop", "browser"]) {
    const requests = [];
    const context = vm.createContext({
      isMobile: platform === "mobile", isNative: platform !== "browser",
      capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
      desktopSave: (_name, write) => write("/chosen", privacy), t: (key) => key,
      window: { prompt: () => "/chosen" }, browserSave() {}, atob: () => "", Blob,
      api: async (name, args) => { requests.push([name, args]); return { bytes: "", fileName: "fixture" }; },
    });
    vm.runInContext(binaryCode, context);
    await context.saveResource("book", "first-install", "A/file", "fixture");
    await context.saveMhtml("book", "first-install", "A/page", "page.mhtml");
    assert.deepEqual(requests.map(([name]) => name), platform === "mobile"
      ? ["mobile_save_resource", "mobile_save_mhtml"] : platform === "desktop"
        ? ["save_resource", "save_mhtml"] : ["export_article", "save_mhtml"]);
    for (const [, args] of requests) {
      assert.equal(args.installationId, "first-install");
      assert.equal(args.privacyGeneration, 7);
    }
  }
});

test("retired binary exports never reach a native picker or browser download", async () => {
  for (const platform of ["mobile", "desktop", "browser"]) {
    const context = vm.createContext({
      isMobile: platform === "mobile", isNative: platform !== "browser",
      isPrivacyContextCurrent: () => false,
      api: assert.fail, desktopSave: assert.fail, browserSave: assert.fail,
    });
    vm.runInContext(binaryCode, context);
    await context.saveBytes("fixture.torrent", "", "application/x-bittorrent", { generation: 7 });
  }
});


test("a 10,000-result group mounts a single accessible page", () => {
  const context = vm.createContext({
    React, useId, useState, useRef,
    t: (key, fields) => fields ? key.replace(/\{(\w+)\}/g, (_, field) => fields[field]) : key,
  });
  vm.runInContext(pagedCode, context);
  const rows = [];
  const html = renderToStaticMarkup(React.createElement(context.PagedResults, {
    items: Array.from({ length: 10000 }, (_, i) => i), criteria: "", label: "Recently read",
    children: (item) => {
      rows.push(item);
      return React.createElement("article", { key: item }, String(item));
    },
  }));
  assert.deepEqual(rows, Array.from({ length: 50 }, (_, i) => i));
  assert.match(html, /role="group" aria-label="Recently read"/);
  assert.match(html, /role="status">1–50 of 10000/);
  assert.match(html, /<button[^>]+disabled[^>]+>Previous<\/button>/);
  assert.match(html, /<button[^>]+aria-controls=[^>]+>Next<\/button>/);
});
