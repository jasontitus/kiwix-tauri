import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import {
  ProviderDocumentError, isProviderUnavailableFor, parseProviderDocumentError,
  parseSuspendedError, normalizeSeverity, shouldRenderError, isSuspendedError,
} from "../src/error-visibility.ts";
import { DiagnosticLog, reportError } from "../src/diagnostics.ts";

const load = async (path) => ts.createSourceFile(path,
  await readFile(new URL(path, import.meta.url), "utf8"), ts.ScriptTarget.Latest, true,
  path.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
const [api, app, reader] = await Promise.all([load("../src/api.ts"), load("../src/App.tsx"), load("../src/components/Reader.tsx")]);
function find(tree, predicate) {
  const nodes = [];
  function visit(node) { if (predicate(node)) nodes.push(node); ts.forEachChild(node, visit); }
  visit(tree);
  return nodes;
}
const boundary = find(api, (node) => ts.isFunctionDeclaration(node) && node.name?.text === "dispatchOnce")[0];
const reporter = find(app, (node) => ts.isVariableDeclaration(node) && node.name.getText(app) === "onError")[0].initializer.arguments[0];
const readerElement = find(app, (node) => ts.isJsxSelfClosingElement(node) && node.tagName.getText(app) === "Reader")[0];
const readerError = readerElement.attributes.properties.find((node) => ts.isJsxAttribute(node) && node.name.getText(app) === "onError").initializer.expression;
const localReporter = find(reader, (node) => ts.isVariableDeclaration(node) && node.name.getText(reader) === "report")[0].initializer.arguments[0];
const recovery = find(reader, (node) => ts.isCallExpression(node) && node.expression.getText(reader) === "setProviderUnavailable" && ts.isArrowFunction(node.arguments[0]))[0];
const compile = (source) => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
test("a target-independent Android bundle captures both document identities once", async () => {
  const names = ["isNative", "agent", "buildPlatform", "isAndroid", "androidDocumentSource"];
  const declarations = names.map((name) => {
    const declaration = find(api, (node) => ts.isVariableDeclaration(node) && node.name.getText(api) === name)[0];
    return `const ${declaration.getText(api)};`;
  }).join("\n").replaceAll("import.meta.env.TAURI_ENV_PLATFORM", "undefined");
  let activity = "12";
  let captured;
  const page = {
    __KIWIX_ANDROID_WINDOW_INCARNATION__: "first",
    __KIWIX_ANDROID_ACTIVITY_SOURCE__: { generation: () => activity },
  };
  const context = vm.createContext({
    navigator: { userAgent: "Mozilla/5.0 (Linux; Android 15)" }, window: page,
    isTauri: () => true, invoke: async (_, payload) => { captured = payload; },
  });
  vm.runInContext(compile(declarations + "\n" + boundary.getText(api)), context);
  activity = "13";
  page.__KIWIX_ANDROID_WINDOW_INCARNATION__ = "second";
  await context.dispatchOnce("frontend_ready", {});
  assert.equal(captured.androidWindowIncarnation, "first");
  assert.equal(captured.androidActivityGeneration, "12");
});
const book = { id: "book-1", installationId: "installation-1", sourceReference: { reference: "01234567-89ab-4cde-8fab-0123456789ab" } };
const envelope = (overrides = {}) => JSON.stringify({
  code: "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT", retry: true,
  message: "This archive is temporarily unavailable. Reconnect its storage, then reopen or reload the article.",
  bookId: book.id, installationId: book.installationId, sourceReference: book.sourceReference.reference,
  ...overrides,
});
function consumer() {
  const log = new DiagnosticLog();
  const bands = [];
  let localError = "";
  let localProvider = null;
  const currentBook = structuredClone(book);
  const context = vm.createContext({
    Error, isNative: true, androidDocumentSource: {},
    invoke: async () => { throw envelope(); },
    parseProviderDocumentError, parseSuspendedError, isProviderUnavailableFor,
    diagnostics: log, reportError, normalizeSeverity, shouldRenderError, isSuspendedError,
    showTechnicalErrors: { current: true }, setError: (value) => { localError = value; },
    errorText: (error) => error instanceof Error ? error.message : String(error),
    message: (error) => error instanceof Error ? error.message : String(error),
    currentBook, alive: { current: true },
    latest: { current: { books: [currentBook], settings: { showTechnicalErrors: false } } },
    tabRef: { current: { bookId: currentBook.id } },
    session: { location: { bookId: currentBook.id }, installationId: currentBook.installationId },
    isPrivacyTransitionError: () => false, reestablishAuthority() {},
    setProviderUnavailable: (value) => { localProvider = typeof value === "function" ? value(localProvider) : value; },
    setGlobalError: (value) => bands.push(value),
  });
  // Root and reader have separate setError owners, just as React does.
  vm.runInContext(compile(`${boundary.getText(api)}
    const rootOnError = ((setError) => (${reporter.getText(app)}))(setGlobalError);
    latest.current.onError = ((onError) => (${readerError.getText(app)}))(rootOnError);
    globalThis.report = ${localReporter.getText(reader)};
    globalThis.recovered = () => { ${recovery.getText(reader)}; };`), context);
  return { context, log, bands, currentBook, localError: () => localError, localProvider: () => localProvider };
}

test("trusted unavailable acquisition becomes a source-scoped actionable reader notice, then clears only on matching read success", async () => {
  const ui = consumer();
  let failure;
  try { await ui.context.dispatchOnce("open_article", { bookId: book.id }); } catch (error) { failure = error; }
  assert.ok(failure instanceof ProviderDocumentError);
  assert.equal(failure.message, JSON.parse(envelope()).message, "no raw JSON/NativeDocumentError reaches UI");
  ui.context.report(failure);
  assert.equal(ui.localProvider(), failure);
  assert.equal(ui.localError(), "");
  assert.deepEqual(ui.bands, [], "reader-local retry notice never paints generic global failure");
  assert.deepEqual(ui.log.snapshot(), []);
  // A retry does not clear the separate transient state merely by clearing a
  // generic error or obtaining unrelated/new-installation document authority.
  ui.context.setError("");
  assert.equal(ui.localProvider(), failure);
  ui.context.session.installationId = "replacement";
  ui.context.recovered();
  assert.equal(ui.localProvider(), failure);
  ui.context.session.installationId = book.installationId;
  ui.currentBook.sourceReference.reference = "01234567-89ab-4cde-8fab-0123456789ac";
  ui.context.recovered();
  assert.equal(ui.localProvider(), failure);
  ui.currentBook.sourceReference.reference = book.sourceReference.reference;
  ui.context.recovered();
  assert.equal(ui.localProvider(), null, "successful same-source/same-installation ready retires its temporary notice");
});

test("uppercase persisted iOS references retain exact source-scoped recovery authority", async () => {
  const ui = consumer();
  const reference = book.sourceReference.reference.toUpperCase();
  ui.currentBook.sourceReference.reference = reference;
  ui.context.invoke = async () => { throw envelope({ sourceReference: reference }); };
  let failure;
  try { await ui.context.dispatchOnce("open_article", { bookId: book.id }); } catch (error) { failure = error; }
  assert.ok(failure instanceof ProviderDocumentError);
  assert.equal(failure.sourceReference, reference);
  ui.context.report(failure);
  assert.equal(ui.localProvider(), failure);
  assert.deepEqual(ui.bands, []);
  ui.context.recovered();
  assert.equal(ui.localProvider(), null);
});

test("a provider lease conflict is retryable only for its exact book, installation, and source", async () => {
  const ui = consumer();
  const conflict = {
    code: "PROVIDER_LEASE_CONFLICT",
    message: "This archive is temporarily busy. Reopen or reload the article to try again.",
  };
  for (const unscoped of [
    { bookId: "other-book" }, { installationId: "replaced" }, { retry: false },
    { sourceReference: "01234567-89ab-4cde-8fab-0123456789ac" },
  ]) {
    const error = parseProviderDocumentError(envelope({ ...conflict, ...unscoped }));
    assert.ok(error);
    assert.equal(isProviderUnavailableFor(error, ui.currentBook), false);
  }

  ui.context.invoke = async () => { throw envelope(conflict); };
  let error;
  try { await ui.context.dispatchOnce("open_article", { bookId: book.id }); } catch (failure) { error = failure; }
  assert.ok(error instanceof ProviderDocumentError);
  ui.context.report(error);
  assert.equal(ui.localProvider(), error);
  assert.equal(ui.localError(), "");
  assert.deepEqual(ui.bands, []);
  ui.context.recovered();
  assert.equal(ui.localProvider(), null);
});

test("revoked grants and other failures stay visible and are not cleared by a successful unrelated provider read", async () => {
  for (const raw of [envelope({ code: "PROVIDER_REFERENCE_REVOKED", retry: false, message: "Access is no longer authorized. Choose Locate." }), "Deletion was refused"]) {
    const ui = consumer();
    ui.context.invoke = async () => { throw raw; };
    let failure;
    try { await ui.context.dispatchOnce("open_article", {}); } catch (error) { failure = error; }
    ui.context.report(failure);
    assert.equal(ui.localProvider(), null);
    assert.equal(ui.bands.length, 0, "the active reader owns the visible notice");
    assert.equal(ui.log.snapshot().length, 1);
    assert.equal(ui.localError(), failure.message);
    ui.context.recovered();
    assert.equal(ui.bands.length, 0);
    assert.equal(ui.localError(), failure.message, "source recovery cannot erase a genuine error");
  }
});

test("unscoped or mismatched envelopes never acquire local-retry authority", () => {
  for (const overrides of [{ bookId: "other" }, { installationId: "replacement" }, { sourceReference: "01234567-89ab-4cde-8fab-0123456789ac" }, { retry: false }, { bookId: undefined }]) {
    const ui = consumer();
    const failure = parseProviderDocumentError(envelope(overrides));
    assert.ok(failure);
    assert.equal(isProviderUnavailableFor(failure, ui.currentBook), false);
    ui.context.report(failure);
    assert.equal(ui.localProvider(), null);
    assert.equal(ui.bands.length, 0);
    assert.equal(ui.localError(), failure.message);
  }
  assert.equal(parseProviderDocumentError(envelope({ sourceReference: "not-a-reference" })), null);
  assert.equal(parseProviderDocumentError("NativeDocumentError: PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT"), null);
});
