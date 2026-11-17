import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import {
  ERROR_SITES,
  SUSPENDED_CODE,
  SuspendedError,
  isSuspendedError,
  normalizeSeverity,
  parseSuspendedError,
  shouldRenderError,
} from "../src/error-visibility.ts";
import {
  DIAGNOSTIC_LIMIT,
  DiagnosticLog,
  reportError,
} from "../src/diagnostics.ts";

test("only the sensor decision write is blocking; the rest stay recoverable", () => {
  for (const [site, severity] of Object.entries(ERROR_SITES)) {
    assert.equal(
      severity,
      site === "reader.sensor-revoke" ? "blocking" : "recoverable",
      site,
    );
  }
});

test("the reporting path defaults to blocking", () => {
  assert.equal(normalizeSeverity(undefined), "blocking");
  assert.equal(normalizeSeverity("blocking"), "blocking");
  assert.equal(normalizeSeverity("recoverable"), "recoverable");
});

test("recoverable errors render only when technical details are on", () => {
  assert.equal(shouldRenderError("recoverable", false), false);
  assert.equal(shouldRenderError("recoverable", true), true);
});

test("blocking errors render regardless of the setting", () => {
  assert.equal(shouldRenderError("blocking", false), true);
  assert.equal(shouldRenderError("blocking", true), true);
  assert.equal(shouldRenderError(undefined, false), true);
});

test("a recoverable error is recorded but not rendered with the setting off", () => {
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new Error("A stale article link was not found"),
    "recoverable",
    false,
  );
  assert.equal(render, false);
  assert.equal(log.snapshot().length, 1);
  assert.equal(log.snapshot()[0].severity, "recoverable");
  assert.match(log.snapshot()[0].message, /stale article link/);
});

test("a blocking error is recorded and rendered with the setting off", () => {
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new Error("The archive could not be opened"),
    "blocking",
    false,
  );
  assert.equal(render, true);
  assert.equal(log.snapshot().length, 1);
  assert.equal(log.snapshot()[0].severity, "blocking");
});

test("the in-memory log keeps only the newest bounded entries", () => {
  const log = new DiagnosticLog(3);
  for (let index = 0; index < 5; index++)
    log.record(new Error(`failure ${index}`), "recoverable");
  assert.deepEqual(
    log.snapshot().map((entry) => entry.message),
    ["failure 2", "failure 3", "failure 4"],
  );
  assert.equal(DIAGNOSTIC_LIMIT, 50);
});

test("clearing the log notifies subscribers and empties the copy text", () => {
  const log = new DiagnosticLog();
  let notifications = 0;
  const unsubscribe = log.subscribe(() => notifications++);
  log.record(new Error("recorded"), "recoverable");
  log.clear();
  unsubscribe();
  assert.equal(notifications, 2);
  assert.deepEqual(log.snapshot(), []);
  assert.equal(log.text(), "[]");
});

test("the Rust lifecycle gate becomes a typed error, never a text match", () => {
  const signal = JSON.stringify({
    code: SUSPENDED_CODE,
    message: "The application is not interactive",
  });
  const error = parseSuspendedError(signal);
  assert.ok(error instanceof SuspendedError);
  assert.equal(error.code, SUSPENDED_CODE);
  assert.equal(isSuspendedError(error), true);
  assert.equal(isSuspendedError(new Error(signal)), false);
  assert.equal(
    parseSuspendedError("The application is not interactive"),
    null,
  );
  assert.equal(parseSuspendedError('{"code":"OTHER","message":"no"}'), null);
});

test("a suspended command is neither recorded nor rendered", () => {
  assert.equal(shouldRenderError("suspended", false), false);
  assert.equal(shouldRenderError("suspended", true), false);
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new SuspendedError("The application is not interactive"),
    "blocking",
    true,
  );
  assert.equal(render, false);
  assert.deepEqual(log.snapshot(), []);
});

test("selected-storage operations preserve terminal SuspendedError through Library and App reporting", async () => {
  const loadTree = async (path) => {
    const source = await readFile(new URL(path, import.meta.url), "utf8");
    return ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  };
  const [selected, library, app, ui] = await Promise.all([
    loadTree("../src/components/SelectedStorageActions.tsx"),
    loadTree("../src/components/Library.tsx"),
    loadTree("../src/App.tsx"),
    loadTree("../src/components/UI.tsx"),
  ]);
  const find = (tree, predicate) => {
    const found = [];
    function visit(node) {
      if (predicate(node)) found.push(node);
      ts.forEachChild(node, visit);
    }
    visit(tree);
    return found;
  };
  const operations = find(selected, (node) => ts.isFunctionDeclaration(node) &&
    ["chooseLocation", "relocate"].includes(node.name?.text));
  const forwards = find(library, (node) => ts.isJsxSelfClosingElement(node) &&
    node.tagName.getText(library) === "SelectedStorageActions").map((node) =>
    node.attributes.properties.find((attribute) =>
      ts.isJsxAttribute(attribute) && attribute.name.getText(library) === "onError").initializer.expression);
  const reporter = find(app, (node) => ts.isVariableDeclaration(node) &&
    node.name.getText(app) === "onError")[0].initializer.arguments[0];
  const formatter = find(ui, (node) => ts.isFunctionDeclaration(node) && node.name?.text === "errorText")[0];
  assert.equal(operations.length, 2);
  assert.equal(forwards.length, 2, "test bulk and detail storage consumers");
  assert.ok(reporter);
  assert.ok(formatter);
  const compile = (source) => ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  for (const operation of operations) for (const forward of forwards) {
    for (const failure of [
      new SuspendedError("The originating foreground was retired", false),
      new Error("Storage permission was revoked"),
    ]) {
      const log = new DiagnosticLog();
      const bands = [];
      const context = vm.createContext({
        Error, reportError, diagnostics: log, normalizeSeverity, isSuspendedError,
        showTechnicalErrors: { current: true }, setError: (value) => bands.push(value),
        isIOS: false, t: (value) => value,
        capturePrivacyContext: () => ({ generation: 7, privateMode: false, readOnly: false }),
        isPrivacyContextCurrent: () => true,
        unavailable: false, target: "/managed", removeOld: false, books: [{ id: "book" }],
        api: async () => { throw failure; },
        onChanged: () => { throw new Error("failed operation must not refresh"); },
        setOpen() {}, setBusy() {}, setLocations() {}, setTarget() {}, setRemoveOld() {}, setNotice() {},
      });
      vm.runInContext(compile(`${formatter.getText(ui).replace(/^export /, "")}
        const rootOnError = ${reporter.getText(app)};
        const onError = ((onError) => (${forward.getText(library)}))(rootOnError);
        ${operation.getText(selected)}`), context);
      await context[operation.name.text]();
      if (isSuspendedError(failure)) {
        assert.deepEqual(log.snapshot(), [], operation.name.text);
        assert.deepEqual(bands, [], "lifecycle cancellation must not paint the global error band");
      } else {
        assert.equal(log.snapshot().length, 1);
        assert.equal(log.snapshot()[0].message, failure.message);
        assert.equal(bands.length, 1);
        assert.equal(bands[0].message, failure.message, "genuine storage failure remains visible");
      }
    }
  }
});
