import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Run the production declarations of the Library, Catalog and storage panes
// against controllable service and timer boundaries.
async function declarations(file, names, bindings = {}) {
  const source = await readFile(new URL(`../src/components/${file}`, import.meta.url), "utf8");
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found = new Map();
  let effect;
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) found.set(node.name.text, node.getText(tree));
    if (ts.isVariableStatement(node)) for (const declaration of node.declarationList.declarations)
      if (names.includes(declaration.name.getText(tree))) found.set(declaration.name.getText(tree), node.getText(tree));
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
        node.arguments[0]?.getText(tree).includes("void loadSuggestions(prefix")) effect = node.arguments[0].getText(tree);
    ts.forEachChild(node, visit);
  }
  visit(tree);
  for (const name of names) assert.ok(found.has(name), `${name} exists in ${file}`);
  const code = ts.transpileModule(`${names.map((name) => found.get(name)).join("\n")}
globalThis.exported = { ${names.join(", ")} };
globalThis.suggestionEffect = ${effect ?? "undefined"};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const context = vm.createContext({
    t: (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key])),
    errorText: (value) => value instanceof Error ? value.message : String(value),
    Date, Number, ...bindings,
  });
  vm.runInContext(code, context);
  return context;
}

const book = (fields) => ({
  id: "book", installationId: "installation", name: "wikipedia_en_all", date: "2026-10-16",
  tags: [], path: "/library/wikipedia_en_all_maxi_2026-10.zim", sourceReference: null, available: true, ...fields,
});
const entry = (fields) => ({
  id: "entry", name: "wikipedia_en_all", date: "2026-10-16T00:00:00Z", tags: ["_flavour:maxi"],
  url: "https://download.kiwix.org/zim/wikipedia/wikipedia_en_all_maxi_2026-10.zim.meta4", ...fields,
});

test("the official catalog's RFC 3339 date matches the installed edition of the same flavour only", async () => {
  const { exported } = await declarations("Catalog.tsx", [
    "archiveName", "FLAVOUR_IN_NAME", "archiveFlavour", "lastSegment", "editionDay", "installedArchive", "installedEdition",
  ]);
  const index = (...books) => new Map([["wikipedia_en_all", books.map(exported.installedArchive)]]);
  const installed = book({});
  assert.deepEqual(JSON.parse(JSON.stringify(exported.installedEdition(entry({}), index(installed)))),
    JSON.parse(JSON.stringify({ book: installed, exact: true })));
  // A timestamp is taken in UTC, as the Rust catalog index does.
  assert.equal(exported.installedEdition(entry({ date: "2026-10-15T22:00:00-02:00" }), index(installed)).exact, true);
  assert.equal(exported.installedEdition(entry({ date: "2026-11-01T00:00:00Z" }), index(installed)).exact, false);
  // Only nopic is installed: the maxi card is a different archive, by tag or by file name.
  const nopic = book({ path: "/library/wikipedia_en_all_nopic_2026-10.zim" });
  assert.equal(exported.installedEdition(entry({}), index(nopic)), null);
  assert.equal(exported.installedEdition(entry({}), index(book({ tags: ["_flavour:nopic"], path: "/x/a.zim" }))), null);
  assert.equal(exported.installedEdition(entry({ tags: [] }), index(nopic)), null, "the entry's file name states maxi");
  assert.equal(exported.installedEdition(entry({ tags: ["_flavour:nopic"] }), index(installed, nopic)).book, nopic);
  // A flavour neither side states does not separate them.
  const unnamed = book({ path: "/library/archive.zim" });
  assert.equal(exported.installedEdition(entry({ tags: [], url: "https://x.invalid/a.zim" }), index(unnamed)).exact, true);
  assert.equal(exported.installedEdition(entry({}), index(unnamed)).exact, true);
  // Provider-backed archives are named by their display name.
  const provided = book({ path: "content://x/1", sourceReference: { reference: "r", displayName: "wikipedia_en_all_nopic_2026-10.zim", persistent: true } });
  assert.equal(exported.installedEdition(entry({}), index(provided)), null);
  assert.equal(exported.editionDay("not a date"), null);
  assert.equal(exported.editionDay("2026-10"), null);
});

function torrentShare(jobs) {
  const state = { error: "", sharing: null, busy: null, errors: [], polls: 0 };
  const privacy = { generation: 1, privateMode: false, readOnly: false };
  const bindings = {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
    settings: { readOnly: false }, shareBusy: false,
    shareRequest: { current: 0 }, alive: { current: true },
    setShareBusy: (value) => { state.busy = value; }, setShareError: (value) => { state.error = value; },
    setShareCopied() {}, setSharing: (value) => { state.sharing = value; }, setDetail() {}, onRefresh() {},
    onError: (...args) => state.errors.push(args),
    window: { setTimeout: (resolve) => { resolve(); return 0; } },
    api: async (command) => {
      if (command === "share_torrent") return jobs(0);
      state.polls++;
      return jobs(state.polls);
    },
  };
  return { state, bindings };
}

test("sharing a large archive waits for its torrent instead of failing after three minutes", async () => {
  const creating = { id: "job", status: "creating", magnet: null, error: null };
  const { state, bindings } = torrentShare((poll) => poll < 300 ? creating : { ...creating, status: "seeding", magnet: "magnet:?xt=urn:btih:x" });
  const { exported } = await declarations("Library.tsx", ["shareBook", "shareSettled"], bindings);
  await exported.shareBook({ id: "book" });
  assert.equal(state.polls, 300);
  assert.equal(state.error, "");
  assert.equal(state.sharing.magnet, "magnet:?xt=urn:btih:x");
  assert.deepEqual(state.errors, []);
  assert.equal(exported.shareSettled(creating), false);
});

test("a torrent that stops without a magnet link says so and the dialog stops waiting", async () => {
  const { state, bindings } = torrentShare((poll) => poll < 3
    ? { id: "job", status: "creating", magnet: null, error: null }
    : { id: "job", status: "error", magnet: null, error: "Disk read failed" });
  const { exported } = await declarations("Library.tsx", ["shareBook", "shareSettled"], bindings);
  await exported.shareBook({ id: "book" });
  assert.equal(state.error, "Disk read failed");
  assert.equal(exported.shareSettled(state.sharing), true, "no spinner remains beside the error");
  assert.deepEqual(state.errors, [], "the share dialog is the one owner");
});

test("a failed status read inside the share dialog is shown once, in the dialog", async () => {
  const { state, bindings } = torrentShare((poll) => {
    if (poll === 2) throw new Error("Torrent status unavailable");
    return { id: "job", status: "creating", magnet: null, error: null };
  });
  const { exported } = await declarations("Library.tsx", ["shareBook", "shareSettled"], bindings);
  await exported.shareBook({ id: "book" });
  assert.equal(state.error, "Torrent status unavailable");
  assert.equal(state.errors.length, 1);
  assert.deepEqual(state.errors[0].slice(1), ["blocking", false], "recorded, not painted a second time");
});

test("Enter before the suggestion debounce elapses keeps the submitted search running", async () => {
  const timers = new Map();
  let nextTimer = 0;
  const sent = [];
  const pendingSearch = [];
  const state = { busy: false, results: null, errors: [] };
  const bindings = {
    search: "animal", selected: ["wiki"], searchOpen: true, searchFocused: true, mode: "combined",
    snippetSaving: false, SUGGESTION_DEBOUNCE_MS: 120, SUGGESTION_PAGE_SIZE: 15,
    suggestionId: { current: 0 }, suggestionPrefix: { current: "" }, suggestionScope: { current: [] },
    suggestionMore: { current: false }, searchId: { current: 0 }, searchParameters: { current: null },
    capturePrivacyContext: () => ({ generation: 1 }), isPrivacyContextCurrent: () => true,
    diagnostics: { record() {} }, ERROR_SITES: {},
    setSuggestions() {}, setSuggestionToken() {}, setSuggestionOpen() {}, setSuggestionActive() {}, setSuggestionLoading() {},
    setBusy: (value) => { state.busy = value; }, setResults: (value) => { state.results = value; },
    onError: (error) => state.errors.push(error), api: async () => null,
    window: {
      setTimeout(callback) { const id = ++nextTimer; timers.set(id, callback); return id; },
      clearTimeout(id) { timers.delete(id); },
    },
    searchRequest: { current: {
      suggest(parameters) { sent.push(["suggest", parameters]); return new Promise(() => {}); },
      run(parameters) {
        sent.push(["search", parameters]);
        return new Promise((resolve) => pendingSearch.push(resolve));
      },
    } },
  };
  const context = await declarations("Library.tsx", ["closeSuggestions", "loadSuggestions", "performSearch"], bindings);
  context.suggestionEffect();           // the last keystroke arms the debounce
  const search = context.exported.performSearch("animal");
  for (const callback of [...timers.values()]) callback(); // the debounce elapses
  assert.deepEqual(sent.map(([command]) => command), ["search"], "no suggestion supersedes the search");
  pendingSearch[0]({ hits: [], total: 0, hasMore: false, errors: [] });
  await search;
  assert.deepEqual(state.errors, []);
  assert.equal(state.results.total, 0);
});

test("a provider-backed selection is copied, never moved, even if Move was chosen", async () => {
  const calls = [];
  const privacy = { generation: 3, privateMode: false, readOnly: false };
  const bindings = {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
    unavailable: false, target: "/managed", removeOld: true,
    books: [{ id: "managed", sourceReference: null }, { id: "provided", sourceReference: { reference: "r" } }],
    api: async (command, args) => { calls.push({ command, args }); return { cleanupPending: null }; },
    onChanged: async () => {}, onError: (error) => { throw error; },
    setOpen() {}, setBusy() {}, setNotice: (value) => calls.push({ notice: value }),
  };
  const context = await declarations("SelectedStorageActions.tsx", ["relocate"], bindings);
  await context.exported.relocate();
  assert.equal(calls[0].command, "library_files_relocate");
  assert.equal(calls[0].args.removeOld, false);
  assert.equal(calls[1].notice, "Selected archives copied. Originals were kept.");
  context.books = [{ id: "managed", sourceReference: null }];
  calls.length = 0;
  await context.exported.relocate();
  assert.equal(calls[0].args.removeOld, true, "managed archives can still be moved");
});
