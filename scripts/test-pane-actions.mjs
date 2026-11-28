import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Run the production declarations of the Library, Catalog and storage panes
// against controllable service and timer boundaries.
// `derived` names component-body constants that are re-evaluated on demand,
// as React does on each render: `context.derive.<name>()`.
// `effects` names marker text; each useEffect callback containing one is
// exported as `context.effects[marker]()`.
async function declarations(file, names, bindings = {}, derived = [], effects = []) {
  const source = await readFile(new URL(`../src/components/${file}`, import.meta.url), "utf8");
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found = new Map();
  const expressions = new Map();
  const marked = new Map();
  let effect;
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) found.set(node.name.text, node.getText(tree));
    if (ts.isVariableStatement(node)) for (const declaration of node.declarationList.declarations)
      if (names.includes(declaration.name.getText(tree))) found.set(declaration.name.getText(tree), node.getText(tree));
      else if (derived.includes(declaration.name.getText(tree))) expressions.set(declaration.name.getText(tree), declaration.initializer.getText(tree));
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
        node.arguments[0]?.getText(tree).includes("void loadSuggestions(prefix")) effect = node.arguments[0].getText(tree);
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect")
      for (const marker of effects) if (node.arguments[0]?.getText(tree).includes(marker)) marked.set(marker, node.arguments[0].getText(tree));
    ts.forEachChild(node, visit);
  }
  visit(tree);
  for (const name of names) assert.ok(found.has(name), `${name} exists in ${file}`);
  for (const name of derived) assert.ok(expressions.has(name), `${name} is derived in ${file}`);
  for (const marker of effects) assert.ok(marked.has(marker), `an effect with ${marker} exists in ${file}`);
  const code = ts.transpileModule(`${names.map((name) => found.get(name)).join("\n")}
globalThis.exported = { ${names.join(", ")} };
globalThis.effects = { ${effects.map((marker) => `${JSON.stringify(marker)}: ${marked.get(marker)}`).join(", ")} };
globalThis.derive = { ${derived.map((name) => `${name}: () => (${expressions.get(name)})`).join(", ")} };
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
  id: "book", installationId: "installation", name: "wikipedia_en_all", date: "2026-10-16", language: "eng",
  tags: [], path: "/library/wikipedia_en_all_maxi_2026-10.zim", sourceReference: null, available: true, ...fields,
});
const entry = (fields) => ({
  id: "entry", name: "wikipedia_en_all", date: "2026-10-16T00:00:00Z", language: "eng", tags: ["_flavour:maxi"],
  url: "https://download.kiwix.org/zim/wikipedia/wikipedia_en_all_maxi_2026-10.zim.meta4", ...fields,
});

test("the official catalog's RFC 3339 date matches the installed edition of the same flavour only", async () => {
  const { exported } = await declarations("Catalog.tsx", [
    "archiveName", "FLAVOUR_IN_NAME", "archiveFlavour", "VARIANT_KEYS", "archiveVariants", "variantsCompatible",
    "SERIES_VARIANT_KEYS", "seriesVariants", "lastSegment", "editionDay", "installedArchive", "installedEdition",
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
  // A flavour only one side states is a related edition, never "Open": the
  // renamed copy may be the nopic archive, so Download stays offered.
  const renamed = book({ path: "/library/wikipedia.zim", tags: ["_category:wikipedia", "_pictures:no"] });
  assert.equal(exported.installedEdition(entry({}), index(unnamed)).exact, false);
  assert.equal(exported.installedEdition(entry({}), index(renamed)).exact, false);
  // Content variants both sides state must agree, as in the Rust series match.
  assert.equal(exported.installedEdition(entry({ tags: ["_flavour:maxi", "_pictures:yes"] }), index(renamed)), null);
  assert.equal(exported.installedEdition(entry({ tags: ["_pictures:no"], url: "https://x.invalid/a.zim" }), index(renamed)).exact, true);
  // Exact, which removes Download, follows the Rust series match exactly: every
  // variant (with _ftindex) equal, none stated on one side only, same language.
  // The related badge keeps the looser rule and Download stays offered.
  const plain = (tags) => book({ name: "guide_en_all", tags, path: "/l/guide_en_all_2026-10.zim" });
  const guide = (tags, fields = {}) => entry({ name: "guide_en_all", tags, url: "https://x.invalid/guide_en_all_2026-10.zim", ...fields });
  const guides = (archive) => new Map([["guide_en_all", [exported.installedArchive(archive)]]]);
  for (const [entryTags, bookTags] of [[["_ftindex:yes"], ["_ftindex:no"]], [["_pictures:no"], []], [[], ["_videos:no"]]]) {
    const found = exported.installedEdition(guide(entryTags), guides(plain(bookTags)));
    assert.equal(found?.exact, false, `${entryTags} vs ${bookTags}`);
  }
  assert.equal(exported.installedEdition(guide(["_ftindex:yes", "_pictures:no"]), guides(plain(["_pictures:no", "_ftindex:yes"]))).exact, true);
  assert.equal(exported.installedEdition(guide([], { language: "fra" }), guides(plain([]))).exact, false);
  assert.notEqual(exported.installedEdition(guide(["_pictures:no", "_pictures:yes"]), guides(plain(["_pictures:no"])))?.exact, true,
    "conflicting variants are unmatchable");
  // Provider-backed archives are named by their display name.
  const provided = book({ path: "content://x/1", sourceReference: { reference: "r", displayName: "wikipedia_en_all_nopic_2026-10.zim", persistent: true } });
  assert.equal(exported.installedEdition(entry({}), index(provided)), null);
  assert.equal(exported.editionDay("not a date"), null);
  // Strict RFC 3339 only: without Z or an offset the day depends on the time zone.
  assert.equal(exported.editionDay("2026-10-16T00:30:00"), null);
  assert.equal(exported.editionDay("2026-10-16T00:30:00+09:00"), "2026-10-15");
  assert.equal(exported.editionDay("2026-10-16t00:30:00z"), "2026-10-16");
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
    setSearchSubmitted() {},
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

test("a search that fails does not pop the suggestion list over its error", async () => {
  const timers = new Map();
  let nextTimer = 0;
  const sent = [];
  const errors = [];
  const bindings = {
    search: "animal", selected: ["wiki"], searchOpen: true, searchInputFocused: true, mode: "combined",
    busy: false, searchSubmitted: false, snippetSaving: false, SUGGESTION_DEBOUNCE_MS: 120, SUGGESTION_PAGE_SIZE: 15,
    suggestionId: { current: 0 }, suggestionPrefix: { current: "" }, suggestionScope: { current: [] },
    suggestionMore: { current: false }, searchId: { current: 0 }, searchParameters: { current: null },
    capturePrivacyContext: () => ({ generation: 1 }), isPrivacyContextCurrent: () => true,
    diagnostics: { record() {} }, ERROR_SITES: {},
    setSuggestions() {}, setSuggestionToken() {}, setSuggestionOpen() {}, setSuggestionActive() {}, setSuggestionLoading() {},
    setResults() {}, onError: (error) => errors.push(error), api: async () => null,
    window: {
      setTimeout(callback) { const id = ++nextTimer; timers.set(id, callback); return id; },
      clearTimeout(id) { timers.delete(id); },
    },
    searchRequest: { current: {
      suggest(parameters) { sent.push(parameters); return new Promise(() => {}); },
      run: async () => { throw new Error("Archive unreadable"); },
    } },
  };
  const context = await declarations("Library.tsx", ["closeSuggestions", "loadSuggestions", "performSearch"],
    bindings, ["searchFocused"]);
  context.setBusy = (value) => { context.busy = value; };
  context.setSearchSubmitted = (value) => { context.searchSubmitted = value; };
  await context.exported.performSearch("animal");
  assert.equal(errors.length, 1);
  assert.equal(context.busy, false);
  // React re-runs the debounce effect with the settled value; the field still has focus.
  context.searchFocused = context.derive.searchFocused();
  context.suggestionEffect();
  for (const callback of [...timers.values()]) callback();
  assert.deepEqual(sent, [], "no suggestion list is requested until the next keystroke");
  context.searchSubmitted = false; // what the next keystroke does
  context.searchFocused = context.derive.searchFocused();
  context.suggestionEffect();
  for (const callback of [...timers.values()]) callback();
  assert.equal(sent.length, 1);
});

test("a spelling correction keeps focus on the query on desktop and moves it to the results on a phone", async () => {
  for (const mobile of [false, true]) {
    const focused = [];
    const searched = [];
    const bindings = {
      isMobile: mobile, busy: false, results: null,
      searchInput: { current: { focus: () => focused.push("input") } },
      resultsRegion: { current: { focus: () => focused.push("results") } },
      focusResults: { current: false }, searchId: { current: 0 },
      setSearch() {},
      performSearch: async (query) => { searched.push(query); bindings.searchId.current++; },
    };
    const context = await declarations("Library.tsx", ["searchCorrection"], bindings, [], ["resultsRegion.current?.focus()"]);
    const settle = context.effects["resultsRegion.current?.focus()"];
    context.exported.searchCorrection("animal");
    assert.deepEqual(searched, ["animal"]);
    context.busy = true;
    settle();                       // the search is running: nothing to focus yet
    context.busy = false;
    context.results = { hits: [] };
    settle();                       // its results arrived
    settle();                       // a later render does not move focus again
    assert.deepEqual(focused, mobile ? ["results"] : ["input"], mobile ? "no soft keyboard over the results" : "desktop");
    // A correction whose search did not start (it was refused) leaves no focus
    // move behind for a later search the user did not start from here.
    context.performSearch = async () => {};
    context.exported.searchCorrection("animals");
    context.busy = true;
    settle();
    context.busy = false;
    context.results = { hits: [{}] };
    settle();
    assert.deepEqual(focused, mobile ? ["results"] : ["input", "input"]);
  }
});

// `errorText` as shipped, with the module-level parts it uses, on iOS.
async function iosErrorText() {
  const parts = ["filesystemRoot", "quotedFilesystemPath", "bareFilesystemPath", "NATIVE_CODE", "nativeCodeOnly",
    "nativeCodeForms", "nearbyCodeFamily", "senderVerificationFailure", "nearbyCodeText", "nativeCodeText", "errorText"];
  const path = new URL("../src/components/UI.tsx", import.meta.url);
  const tree = ts.createSourceFile("UI.tsx", await readFile(path, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const found = new Map();
  for (const statement of tree.statements) {
    const names = ts.isFunctionDeclaration(statement) ? [statement.name?.text]
      : ts.isVariableStatement(statement) ? statement.declarationList.declarations.map((item) => item.name.getText(tree)) : [];
    for (const name of names) if (parts.includes(name)) found.set(name, statement.getText(tree).replace(/^export /, ""));
  }
  for (const name of parts) assert.ok(found.has(name), `errorText part ${name} exists`);
  const context = vm.createContext({
    Error, isIOS: true, isSuspendedError: () => false, isUserCancelledError: () => false,
    t: (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key])),
  });
  vm.runInContext(ts.transpileModule(`${[...new Set(found.values())].join("\n")}\nglobalThis.errorText = errorText;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, context);
  return context.errorText;
}

test("on iOS a kept map file's path is hidden without swallowing the reason beside it", async () => {
  const errorText = await iosErrorText();
  const root = "/private/var/mobile/Containers/Data/Application/0A1B/Documents/maps";
  const reasons = [
    // Current backend sentences (maps/partials.rs).
    "Its folder is not available right now. Reconnect the drive that holds it, then press Delete again",
    "It could not be removed just now; another program may be using it. Close anything that has it open, then press Delete again",
    "Its folder was moved or replaced, so Kiwix left the file alone",
    // Features round 4 wording.
    "Its folder cannot be opened right now. If it is on a drive that is not connected, reconnect it and press Delete again. If the folder is gone, dismiss this warning.",
    "It could not be removed just now. Press Delete again; if it keeps failing, free some space or close programs using that folder.",
  ];
  for (const reason of reasons) {
    const warning = `Some earlier map downloads could not be cleaned up. The files were kept here: ${root}/.streetzim-a.part (${reason}), ${root}/.streetzim-b.part`;
    assert.equal(errorText(warning),
      `Some earlier map downloads could not be cleaned up. The files were kept here: [File] (${reason}), [File]`);
    const kept = `Kiwix could not remove a partial map download, so it was kept at ${root}/.streetzim-cleanup-1/.streetzim-a.part. ${reason}`;
    assert.equal(errorText(kept), `Kiwix could not remove a partial map download, so it was kept at [File]. ${reason}`);
  }
  // A full stop or bracket inside a folder name is part of the path: the whole
  // path is hidden, not cut at "Dr." and shown from there on.
  for (const folder of ["Dr. Who", "A. Smith", "Vol. II", "St. Louis Guide (Draft)", "Books (Old)",
    "U.S. History", "J.R.R. Tolkien", "Notes.v2 (Draft)", "Report.Final (Draft)"]) {
    assert.equal(errorText(`Could not open ${root}/${folder}/wikipedia.zim right now`), "Could not open [File]", folder);
    assert.equal(errorText(`The archive ${root}/${folder}/notes.zim is not available`), "The archive [File] is not available", folder);
    assert.equal(errorText(`Import failed for ${root}/${folder}/b.zim`), "Import failed for [File]", folder);
  }
  assert.equal(errorText(`Could not open ${root}/a.zim. Try again.`), "Could not open [File]. Try again.");
  assert.equal(errorText(`Could not open ${root}/a.zimaa. Try again.`), "Could not open [File]. Try again.");
  assert.equal(errorText(`Some earlier map downloads could not be cleaned up. The files were kept here: ${root}/U.S. History/.streetzim-a.part (Its folder was moved or replaced, so Kiwix left the file alone)`),
    "Some earlier map downloads could not be cleaned up. The files were kept here: [File] (Its folder was moved or replaced, so Kiwix left the file alone)");
  assert.equal(errorText(`Some earlier map downloads could not be cleaned up. The files were kept here: ${root}/Vol. II/.streetzim-a.part (Its folder was moved or replaced, so Kiwix left the file alone)`),
    "Some earlier map downloads could not be cleaned up. The files were kept here: [File] (Its folder was moved or replaced, so Kiwix left the file alone)");
  assert.equal(errorText(`(see ${root}/a.zim)`), "(see [File])", "a closing bracket after a path still ends it");
  // The forms redaction already handled are unchanged.
  assert.equal(errorText(`Could not open ${root}/a.zim: permission denied`), "Could not open [File]: permission denied");
  assert.equal(errorText(`${root}/My Maps/a b.zim is missing`), "[File] is missing");
  assert.equal(errorText(`Archive at ${root}/a.zim.`), "Archive at [File].");
  assert.equal(errorText(`Saved to "${root}/a (1).zim" earlier`), "Saved to [File] earlier");
});
