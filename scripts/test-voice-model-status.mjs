import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/components/useVoiceModelStatus.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});
const settle = () => new Promise((resolve) => setImmediate(resolve));

function mount() {
  const slots = [];
  const refs = [];
  const requests = [];
  let stateIndex = 0;
  let refIndex = 0;
  let listener;
  let dispose;
  const react = {
    useRef(current) {
      const index = refIndex++;
      return refs[index] ??= { current };
    },
    useState(initial) {
      const index = stateIndex++;
      const slot = slots[index] ??= { value: initial };
      return [slot.value, (value) => { slot.value = value; }];
    },
    useCallback: (fn) => fn,
    useEffect: (start) => { dispose = start(); },
  };
  const bridge = {
    isNative: true,
    api(command) {
      assert.equal(command, "kokoro_status");
      const deferred = Promise.withResolvers();
      requests.push(deferred);
      return deferred.promise;
    },
  };
  const events = { listen: async (_event, fn) => { listener = fn; return () => {}; } };
  const scope = { exports: {}, window: { setTimeout, clearTimeout }, require: (id) => {
    return { react, "../api": bridge, "@tauri-apps/api/event": events }[id];
  } };
  vm.runInNewContext(outputText, scope);
  function render(enabled) {
    dispose?.();
    stateIndex = refIndex = 0;
    return scope.exports.useVoiceModelStatus(enabled);
  }
  render(true);
  return { slots, requests, render, notify: (payload) => listener({ payload }), dispose: () => dispose() };
}

test("a stale installation event, an old status reply and an old error cannot undo removal", async () => {
  const view = mount();
  await settle();
  const initial = view.requests.slice();
  view.notify({ state: "installed" });
  view.notify({ state: "error", error: "Retired download" });
  const current = view.requests.at(-1);
  current.resolve({ available: false, removalPending: true, downloaded: 0 });
  await settle();
  assert.equal(view.slots[0].value.removalPending, true);
  initial[0].resolve({ available: true });
  for (const request of view.requests.slice(1, -1)) request.reject(new Error("Earlier operation"));
  await settle();
  assert.equal(view.slots[0].value.available, false);
  assert.equal(view.slots[0].value.removalPending, true);
  assert.equal(view.slots[1].value, "");
  view.dispose();
});

test("disabling the surface discards outstanding status and queued listener notifications", async () => {
  const view = mount();
  await settle();
  const count = view.requests.length;
  view.dispose();
  view.notify({ state: "installed" });
  for (const request of view.requests) request.resolve({ available: true });
  await settle();
  assert.equal(view.requests.length, count);
  assert.equal(view.slots[0].value, null);
});

test("reopening a disabled surface starts without the prior installation or lookup error", async () => {
  const view = mount();
  await settle();
  view.requests.at(-1).resolve({ available: false, bundled: false });
  await settle();
  view.notify({ state: "error" });
  view.requests.at(-1).reject(new Error("Earlier lookup failed"));
  await settle();
  assert.equal(view.slots[0].value.bundled, false);
  assert.match(view.slots[1].value, /Earlier lookup failed/);
  view.render(false);
  const reopened = view.render(true);
  assert.equal(reopened.status, null);
  assert.equal(reopened.readError, "");
  await settle();
  view.requests.at(-1).resolve({ available: true, bundled: true });
  await settle();
  assert.equal(view.slots[0].value.bundled, true);
  view.dispose();
});
