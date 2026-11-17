import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";

// Exercise the actual component branches without a browser or native bridge.
async function component(file, name, bindings = {}) {
  const source = await readFile(new URL(`../src/components/${file}`, import.meta.url), "utf8");
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let declaration;
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) declaration = node;
    if (ts.isVariableDeclaration(node) && node.name.getText(tree) === name) declaration = node;
    ts.forEachChild(node, visit);
  }
  visit(tree);
  assert.ok(declaration, `${name} exists in ${file}`);
  const declarationSource = ts.isVariableDeclaration(declaration)
    ? `const ${declaration.getText(tree)};`
    : declaration.getText(tree);
  const code = ts.transpileModule(declarationSource, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const context = {
    React: { createElement: (type, props, ...children) => ({ type, props: props || {}, children }) },
    t: (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key])),
    errorText: String,
    ...bindings,
  };
  vm.runInNewContext(`${code}\nthis.render = ${name};`, context);
  return context.render;
}
function text(node) {
  if (node == null || typeof node === "boolean") return "";
  if (Array.isArray(node)) return node.map(text).join(" ");
  if (typeof node !== "object") return String(node);
  return text(node.children);
}
function elements(node, type) {
  if (!node || typeof node !== "object") return [];
  if (Array.isArray(node)) return node.flatMap((child) => elements(child, type));
  return [...(node.type === type ? [node] : []), ...elements(node.children, type)];
}

test("catalog freshness describes only the displayed page and offers offline retry", async () => {
  const render = await component("Catalog.tsx", "CatalogCacheNotice");
  let retries = 0;
  const base = { temporary: false, onRetry: () => retries++ };
  const network = render({ ...base, cache: { status: "network", stale: false, stored: true, error: null } });
  assert.match(text(network), /This catalog page was refreshed/);
  const validated = render({ ...base, cache: { status: "revalidated", stale: false, stored: true, error: null } });
  assert.match(text(validated), /This catalog page was revalidated/);
  const offline = render({ ...base, cache: { status: "offline", stale: true, stored: true, error: "Network unavailable" } });
  assert.match(text(offline), /cached page that may be out of date/);
  assert.match(text(offline), /Network unavailable/);
  const retry = elements(offline, "button");
  assert.equal(retry.length, 1);
  retry[0].props.onClick();
  assert.equal(retries, 1);
});

test("private and read-only catalog metadata does not promise offline persistence", async () => {
  const render = await component("Catalog.tsx", "CatalogCacheNotice");
  const result = render({
    cache: { status: "network", stale: false, stored: false, error: null },
    temporary: true,
    onRetry() {},
  });
  assert.match(text(result), /not saved for offline browsing/);
  assert.match(text(result), /temporary in private or read-only mode/);
});

test("verified Ready requires review and then a separate explicit Replace action", async () => {
  const render = await component("Downloads.tsx", "EditionDownloadActions", { formatBytes: (value) => `${value} bytes` });
  let reviewed = 0;
  let replaced = 0;
  const props = {
    intent: {
      status: "ready", verified: { id: "new" }, needsAuthorization: true,
      target: { title: "Installed", date: "2025-01" },
      selection: { entry: { title: "Candidate", date: "2026-01", size: 4096 } },
    },
    authorized: false, blocked: false, busy: false, targetCurrent: true,
    onReview: () => reviewed++, onReplace: () => replaced++, onCancel() {},
  };
  const ready = render(props);
  assert.match(text(ready), /Ready.*verified/);
  assert.match(text(ready), /Full download: 4096 bytes/);
  assert.match(text(ready), /Original archive files are retained/);
  assert.equal(replaced, 0, "rendering Ready never commits");
  assert.equal(elements(ready, "button").some((button) => text(button) === "Replace"), false);
  elements(ready, "button").find((button) => text(button) === "Review replacement").props.onClick();
  assert.equal(reviewed, 1);
  assert.equal(replaced, 0, "review alone never commits");
  const reviewedReady = render({ ...props, intent: { ...props.intent, needsAuthorization: false }, authorized: true });
  elements(reviewedReady, "button").find((button) => text(button) === "Replace").props.onClick();
  assert.equal(replaced, 1);
  const retired = render({ ...props, authorized: true });
  assert.equal(elements(retired, "button").some((button) => text(button) === "Replace"), false,
    "native retirement overrides a stale local token");
  const reviewAgain = elements(retired, "button").find((button) => text(button) === "Review replacement");
  assert.equal(reviewAgain.props.disabled, false);
  reviewAgain.props.onClick();
  assert.equal(reviewed, 2);
});

test("downloading never offers Replace and stale or blocked installations cannot cut over", async () => {
  const render = await component("Downloads.tsx", "EditionDownloadActions", { formatBytes: String });
  const props = {
    intent: { status: "downloading", target: { title: "Old" }, selection: { entry: { title: "New", size: 0 } } },
    authorized: true, blocked: false, busy: false, targetCurrent: true,
    onReview() {}, onReplace() {}, onCancel() {},
  };
  assert.equal(elements(render(props), "button").some((button) => text(button) === "Replace"), false);
  for (const override of [{ blocked: true }, { busy: true }, { targetCurrent: false }]) {
    const result = render({ ...props, ...override, intent: { ...props.intent, status: "ready", verified: {} } });
    assert.equal(elements(result, "button").find((button) => text(button) === "Replace").props.disabled, true);
  }
  const cancelled = render({ ...props, intent: { ...props.intent, status: "cancelled" } });
  assert.match(text(cancelled), /original archive is unchanged/);
  assert.equal(elements(cancelled, "button").length, 0);
});

test("association requires an explicit selection and prepares the exact installed principal", async () => {
  const privacy = { generation: 7, epoch: 2, privateMode: false, readOnly: false };
  const update = {
    bookId: "book", installationId: "installation", associationRequired: true,
    selection: { sourceId: "source", entryId: "entry", catalogGeneration: 4 },
  };
  const calls = [];
  const decisions = [];
  const associations = {};
  const prepare = await component("AdvancedSettings.tsx", "prepareEdition", {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
    settings: { privateMode: false, readOnly: false },
    books: [{ id: "book", installationId: "installation" }],
    associations, editionRequest: { current: 1 }, editionPending: { current: false }, editionAlive: { current: true },
    setBusy() {}, setEditionError() {}, setEditionDecision: (decision) => decisions.push(decision),
    api: async (command, args) => { calls.push({ command, args }); return { token: "native-decision" }; },
  });
  await prepare(update);
  assert.equal(calls.length, 0);
  associations["book:source:entry"] = true;
  await prepare(update);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "edition_prepare");
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].args)), {
    bookId: "book", installationId: "installation", sourceId: "source", entryId: "entry",
    catalogGeneration: 4, associationConfirmed: true, privacyGeneration: 7,
  });
  assert.equal(decisions[0].preparation.token, "native-decision");
  await prepare({ ...update, installationId: "retired" });
  assert.equal(calls.length, 1, "stale installation cannot prepare");
});

test("late preparation cannot resurrect a decision after source or privacy retirement", async () => {
  let finish;
  let current = true;
  const request = { current: 1 };
  const decisions = [];
  const update = {
    bookId: "book", installationId: "installation", associationRequired: false,
    selection: { sourceId: "source", entryId: "entry", catalogGeneration: 4 },
  };
  const prepare = await component("AdvancedSettings.tsx", "prepareEdition", {
    capturePrivacyContext: () => ({ generation: 7, epoch: 2, privateMode: false, readOnly: false }),
    isPrivacyContextCurrent: () => current,
    settings: { privateMode: false, readOnly: false }, books: [{ id: "book", installationId: "installation" }],
    associations: {}, editionRequest: request, editionPending: { current: false }, editionAlive: { current: true },
    setBusy() {}, setEditionError() {}, setEditionDecision: (decision) => decisions.push(decision),
    api: () => new Promise((resolve) => { finish = resolve; }),
  });
  const pending = prepare(update);
  current = false;
  request.current++;
  finish({ token: "retired-token" });
  await pending;
  assert.equal(decisions.length, 0);
});

test("Downloads reviews a retained intent after catalog refresh and loads its refreshed native selection", async () => {
  const privacy = { generation: 7, epoch: 2, privateMode: false, readOnly: false };
  const intent = {
    id: "intent", status: "ready", verified: { id: "new" }, downloadId: "download", needsAuthorization: true,
    target: { id: "old", installationId: "original-installation" },
    selection: { sourceId: "source", entryId: "entry", catalogGeneration: 4 },
  };
  const currentSelection = { ...intent.selection, catalogGeneration: 5 };
  const calls = [];
  const authorizations = {};
  let refreshed = 0;
  let downloaded = 0;
  let durableIntent = intent;
  let displayedStatus;
  const native = async (command, args) => {
    if (command === "edition_prepare") {
      calls.push({ command, args });
      assert.deepEqual(JSON.parse(JSON.stringify(args)), {
        intentId: "intent", bookId: "old", installationId: "original-installation", privacyGeneration: 7,
      }, "retained review asks native to resolve the intent, not its obsolete catalog generation");
      durableIntent = { ...durableIntent, selection: currentSelection, needsAuthorization: false };
      return { token: "new-native-decision", intentId: "intent", selection: currentSelection };
    }
    if (command === "edition_commit") {
      calls.push({ command, args });
      durableIntent = { ...durableIntent, status: "committed" };
      return {};
    }
    if (command === "edition_status") return { intents: [durableIntent], transactions: [] };
    if (command === "downloads") return [];
    if (command === "map_status" || command === "transfer_history_notice") return null;
    assert.fail(`Unexpected native command: ${command}`);
  };
  const onRefresh = () => refreshed++;
  const load = await component("Downloads.tsx", "load", {
    useCallback: (callback) => callback, request: { current: 0 }, alive: { current: true },
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true, api: native,
    setHistoryNotice() {}, setDownloadNotice() {},
    setEditions: (status) => { displayedStatus = status; },
    previousEditionRevision: { current: "" }, onRefresh, setEditionError() {},
    previousMap: { current: "" }, setMapJob() {}, setMapError() {},
    previousPayload: { current: { jobs: "", map: "", editions: "" } },
    document: { visibilityState: "visible" },
    previous: { current: new Map() }, setJobs() {}, setError() {}, setLoading() {},
  });
  const run = await component("Downloads.tsx", "editionAction", {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
    editionPending: { current: false }, localBlockedRef: { current: false }, alive: { current: true },
    booksRef: { current: [intent.target] }, editionAuthorizations: authorizations,
    setEditionWorking() {}, setEditionActionError() {},
    setEditionAuthorizations: (update) => {
      const next = typeof update === "function" ? update(authorizations) : update;
      for (const id of Object.keys(authorizations)) delete authorizations[id];
      Object.assign(authorizations, next);
    },
    api: native,
    editionDownload: async () => { downloaded++; },
    onDownload() {}, onRefresh, load,
  });
  await run(intent, "review");
  assert.equal(calls.length, 1);
  assert.equal(downloaded, 0, "Ready output is reused");
  assert.equal(refreshed, 0, "review does not replace the library");
  const reviewed = displayedStatus.intents[0];
  assert.deepEqual(reviewed.selection, currentSelection, "load shows the refreshed durable selection");
  assert.equal(reviewed.needsAuthorization, false);
  assert.equal(reviewed.status, "ready");
  assert.equal(reviewed.verified, intent.verified, "native retains the already verified archive");
  await run(reviewed, "replace");
  assert.equal(calls.length, 2);
  assert.equal(calls[1].command, "edition_commit");
  assert.deepEqual(JSON.parse(JSON.stringify(calls[1].args)), {
    intentId: "intent", decisionToken: "new-native-decision",
    installationId: "original-installation", privacyGeneration: 7,
  });
  assert.equal(refreshed, 1);
  assert.equal(displayedStatus.intents[0].status, "committed", "load reflects durable completion");
});

test("reviewing sequential Ready rows retires sibling tokens and permits reviewing the first again", async () => {
  const privacy = { generation: 7, epoch: 2, privateMode: false, readOnly: false };
  let privacyCurrent = true;
  const intents = ["first", "second"].map((id) => ({
    id, status: "ready", downloadId: `download-${id}`, verified: { id: `new-${id}` },
    needsAuthorization: true,
    target: { id: `old-${id}`, installationId: `installation-${id}`, title: `Installed ${id}` },
    selection: { sourceId: "source", entryId: id, catalogGeneration: 4, entry: { title: id, size: 4096 } },
  }));
  const authorizations = {};
  const nativeDecisions = new Map();
  const reviews = [];
  let commits = 0;
  let downloaded = 0;
  let run;
  const render = await component("Downloads.tsx", "EditionDownloadActions", { formatBytes: String });
  const row = (intent) => render({
    intent, authorized: !!authorizations[intent.id] && privacyCurrent,
    blocked: false, busy: false, targetCurrent: true,
    onReview: () => run(intent, "review"), onReplace: () => run(intent, "replace"), onCancel() {},
  });
  const button = (intent, label) => elements(row(intent), "button").find((item) => text(item) === label);
  run = await component("Downloads.tsx", "editionAction", {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => privacyCurrent,
    editionPending: { current: false }, localBlockedRef: { current: false }, alive: { current: true },
    booksRef: { current: intents.map((intent) => intent.target) }, editionAuthorizations: authorizations,
    setEditionWorking() {}, setEditionActionError() {},
    setEditionAuthorizations: (update) => {
      const next = typeof update === "function" ? update(authorizations) : update;
      for (const id of Object.keys(authorizations)) delete authorizations[id];
      Object.assign(authorizations, next);
    },
    api: async (command, args) => {
      if (command === "edition_prepare") {
        reviews.push(args.intentId);
        nativeDecisions.clear();
        const token = `decision-${reviews.length}`;
        nativeDecisions.set(args.intentId, token);
        for (const intent of intents) intent.needsAuthorization = intent.id !== args.intentId;
        return { token, intentId: args.intentId };
      }
      assert.equal(command, "edition_commit");
      assert.equal(args.decisionToken, nativeDecisions.get(args.intentId));
      commits++;
      return {};
    },
    editionDownload: async () => { downloaded++; },
    onDownload() {}, onRefresh() {}, load: async () => {},
  });
  await button(intents[0], "Review replacement").props.onClick();
  assert.ok(button(intents[0], "Replace"));
  await button(intents[1], "Review replacement").props.onClick();
  assert.deepEqual(Object.keys(authorizations), ["second"], "the second native review replaces all local tokens");
  assert.equal(button(intents[0], "Replace"), undefined);
  assert.equal(button(intents[0], "Review replacement").props.disabled, false);
  assert.ok(button(intents[1], "Replace"));
  await button(intents[0], "Review replacement").props.onClick();
  assert.deepEqual(reviews, ["first", "second", "first"]);
  assert.deepEqual(Object.keys(authorizations), ["first"]);
  assert.equal(button(intents[1], "Replace"), undefined);
  assert.equal(commits, 0, "reviewing Ready rows never commits or downloads");
  assert.equal(downloaded, 0);
  privacyCurrent = false;
  assert.equal(button(intents[0], "Replace"), undefined, "retired privacy context cannot authorize Replace");
  privacyCurrent = true;
  intents[0].needsAuthorization = true;
  assert.equal(button(intents[0], "Replace"), undefined, "native retirement wins even with a local token");
  assert.equal(button(intents[0], "Review replacement").props.disabled, false);
  await run(intents[0], "replace");
  assert.equal(commits, 0, "a stale event cannot submit a retired native decision");
  await button(intents[0], "Review replacement").props.onClick();
  await button(intents[0], "Replace").props.onClick();
  assert.equal(commits, 1, "the first row can be re-reviewed and explicitly replaced");
});

test("committed-but-uncertain errors refresh native state and do not retry a cutover", async () => {
  const privacy = { generation: 7, epoch: 2, privateMode: false, readOnly: false };
  const intent = {
    id: "intent", target: { id: "old", installationId: "old-installation" },
    selection: { sourceId: "source", entryId: "entry", catalogGeneration: 4 },
  };
  const authorizations = { intent: { token: "decision", privacy } };
  let committedCalls = 0;
  let refreshed = 0;
  let loaded = 0;
  let error = "";
  let nextAuthorizations;
  const run = await component("Downloads.tsx", "editionAction", {
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: () => true,
    editionPending: { current: false }, localBlockedRef: { current: false }, alive: { current: true },
    booksRef: { current: [intent.target] }, editionAuthorizations: authorizations,
    setEditionWorking() {}, setEditionActionError: (value) => { error = value; },
    setEditionAuthorizations: (update) => { nextAuthorizations = update(authorizations); },
    api: async () => { committedCalls++; throw new Error("Store committed; directory sync failed"); },
    onRefresh: () => refreshed++, load: async () => { loaded++; },
  });
  await run(intent, "replace");
  assert.equal(committedCalls, 1);
  assert.equal(refreshed, 1);
  assert.equal(loaded, 1);
  assert.match(error, /Store committed; directory sync failed/);
  assert.equal(nextAuthorizations.intent, undefined, "a failed decision must be reviewed afresh");
});

test("rollback confirms and binds the currently installed replacement principal", async () => {
  const transaction = {
    id: "transaction", original: { id: "original" },
    replacement: { id: "replacement", installationId: "replacement-installation" },
  };
  const calls = [];
  let confirmed = false;
  const booksRef = { current: [transaction.replacement] };
  const run = await component("Downloads.tsx", "rollbackEdition", {
    capturePrivacyContext: () => ({ generation: 7, privateMode: false, readOnly: false }),
    isPrivacyContextCurrent: () => true,
    editionPending: { current: false }, localBlockedRef: { current: false }, alive: { current: true },
    booksRef, setEditionWorking() {}, setEditionActionError() {},
    confirmAction: async () => confirmed,
    api: async (command, args) => { calls.push({ command, args }); return transaction; },
    onRefresh() {}, load: async () => {},
  });
  await run(transaction);
  assert.equal(calls.length, 0);
  confirmed = true;
  await run(transaction);
  assert.equal(calls[0].command, "edition_rollback");
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].args)), {
    transactionId: "transaction", expectedInstallationId: "replacement-installation", privacyGeneration: 7,
  });
  booksRef.current = [{ id: "replacement", installationId: "different-installation" }];
  await run(transaction);
  assert.equal(calls.length, 1);
});
