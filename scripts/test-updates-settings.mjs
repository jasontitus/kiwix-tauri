import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const path = "../src/components/UpdatesSettings.tsx";
const source = await readFile(new URL(path, import.meta.url), "utf8");
const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const compile = (code) => ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
function find(predicate) {
  const found = [];
  (function visit(node) {
    if (predicate(node)) found.push(node);
    ts.forEachChild(node, visit);
  })(tree);
  return found;
}
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const turn = () => new Promise((resolve) => setImmediate(resolve));

// The shipped handlers, run against deferred updater replies.
const handlers = ["installNow", "cancelDownload"].map((name) => {
  const [node] = find((candidate) => ts.isFunctionDeclaration(candidate) && candidate.name?.text === name);
  assert.ok(node, `Missing handler: ${name}`);
  return node.getText(tree);
}).join("\n");
function updater() {
  const state = { installing: false, cancelling: false, confirming: true, errors: [], refreshed: 0, calls: [] };
  const install = deferred();
  const cancel = deferred();
  const context = vm.createContext({
    status: { available: { version: "9.9.9" } }, installing: false, cancelling: false,
    capturePrivacyContext: () => ({ generation: 3 }),
    setInstalling: (value) => { state.installing = value; },
    setCancelling: (value) => { state.cancelling = value; },
    setConfirming: (value) => { state.confirming = value; },
    cancelRequest: { current: null },
    api: (command) => {
      state.calls.push(command);
      return command === "update_install" ? install.promise : cancel.promise;
    },
    onError: (error) => state.errors.push(error instanceof Error ? error.message : String(error)),
    refresh: async () => { state.refreshed++; },
  });
  vm.runInContext(compile(handlers), context);
  return { state, context, install, cancel };
}
const CANCELLED = "The update download was cancelled.";

test("cancelling the update download the user started is not reported as a failure", async () => {
  const ui = updater();
  const installing = ui.context.installNow();
  assert.equal(ui.state.installing, true);
  const cancelling = ui.context.cancelDownload();
  ui.cancel.resolve(true);
  await cancelling;
  ui.install.reject(new Error(CANCELLED));
  await installing;
  assert.deepEqual(ui.state.errors, [], "the user's own cancellation paints no blocking band");
  assert.equal(ui.state.installing, false);
  assert.equal(ui.state.cancelling, false);
  assert.equal(ui.state.refreshed, 1);
});

test("the install rejection can arrive before the updater confirms the cancel", async () => {
  const ui = updater();
  const installing = ui.context.installNow();
  const cancelling = ui.context.cancelDownload();
  ui.install.reject(new Error(CANCELLED));
  await turn();
  assert.equal(ui.state.installing, true, "undecided until the updater says whether the cancel took effect");
  ui.cancel.resolve(true);
  await Promise.all([installing, cancelling]);
  assert.deepEqual(ui.state.errors, []);
  assert.equal(ui.state.installing, false);
});

test("an update that stops for any other reason is still a visible failure", async () => {
  // Nobody pressed Cancel here: another window, or shutdown, stopped it.
  const other = updater();
  const first = other.context.installNow();
  other.install.reject(new Error(CANCELLED));
  await first;
  assert.deepEqual(other.state.errors, [CANCELLED]);

  // Cancel was pressed, but there was no download left to cancel: what failed is the install.
  const late = updater();
  const second = late.context.installNow();
  const cancelling = late.context.cancelDownload();
  late.cancel.resolve(false);
  await cancelling;
  assert.equal(late.state.cancelling, false);
  late.install.reject(new Error("The update signature is not valid."));
  await second;
  assert.deepEqual(late.state.errors, ["The update signature is not valid."]);

  // The cancel request itself failed: both failures are reported.
  const broken = updater();
  const third = broken.context.installNow();
  const failing = broken.context.cancelDownload();
  broken.cancel.reject(new Error("Updates are unavailable"));
  await failing;
  broken.install.reject(new Error("The download failed."));
  await third;
  assert.deepEqual(broken.state.errors, ["Updates are unavailable", "The download failed."]);

  // A cancel from an earlier attempt does not excuse the next one.
  const again = updater();
  again.context.cancelRequest.current = Promise.resolve(true);
  const fourth = again.context.installNow();
  again.install.reject(new Error("The download failed."));
  await fourth;
  assert.deepEqual(again.state.errors, ["The download failed."]);
});

test("a completed install re-enables the button when the app keeps running", async () => {
  const ui = updater();
  const installing = ui.context.installNow();
  assert.equal(ui.state.installing, true);
  ui.install.resolve(null);
  await installing;
  assert.equal(ui.state.installing, false, "Quit can be cancelled after the install; Installing… must not stick");
  assert.deepEqual(ui.state.errors, []);
});

// The listener effect, run with `listen` round trips that resolve late.
const [effect] = find((node) => ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
  node.getText(tree).includes('"kiwix-update-progress"'));
assert.ok(effect, "Missing the update listener effect");
function listeners() {
  const state = { pending: [], unlistened: [], progress: [], refreshed: 0 };
  const context = vm.createContext({
    isDesktop: true,
    listen: (event, handler) => {
      const gate = deferred();
      state.pending.push({ event, handler, gate });
      return gate.promise.then(() => () => { state.unlistened.push(event); });
    },
    setProgress: (value) => state.progress.push(value),
    setCancelling() {},
    refresh: async () => { state.refreshed++; },
  });
  const cleanup = vm.runInContext(compile(`(${effect.arguments[0].getText(tree)})()`), context);
  return { state, cleanup };
}

test("update listeners registered after cleanup are removed, not leaked", async () => {
  const early = listeners();
  assert.deepEqual(early.state.pending.map((entry) => entry.event), ["kiwix-update-progress", "kiwix-update-available"]);
  early.cleanup();
  for (const entry of early.state.pending) entry.gate.resolve();
  await turn();
  assert.deepEqual(early.state.unlistened.sort(), ["kiwix-update-available", "kiwix-update-progress"],
    "each late registration is removed exactly once");
  // A stale event from a listener that is being removed changes nothing.
  early.state.pending[0].handler({ payload: { phase: "downloading", downloaded: 1, total: 2 } });
  early.state.pending[1].handler({});
  assert.deepEqual(early.state.progress, []);
  assert.equal(early.state.refreshed, 0);

  const usual = listeners();
  for (const entry of usual.state.pending) entry.gate.resolve();
  await turn();
  assert.deepEqual(usual.state.unlistened, []);
  usual.state.pending[0].handler({ payload: null });
  assert.deepEqual(usual.state.progress, [null]);
  usual.cleanup();
  assert.deepEqual(usual.state.unlistened.sort(), ["kiwix-update-available", "kiwix-update-progress"]);

  // One listener resolved before cleanup, the other after.
  const mixed = listeners();
  mixed.state.pending[0].gate.resolve();
  await turn();
  mixed.cleanup();
  mixed.state.pending[1].gate.resolve();
  await turn();
  assert.deepEqual(mixed.state.unlistened.sort(), ["kiwix-update-available", "kiwix-update-progress"]);
});
