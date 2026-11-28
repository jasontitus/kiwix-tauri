import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";
import {
  READER_OVERLAYS,
  SHELL_POPOVERS,
  backTarget,
  escapeTarget,
  openDialogs,
  openPopover,
} from "../src/shell-dismiss.ts";
import { BACK_LISTENER_KEY, createBackRegistration } from "../src/android-back.ts";

const library = { dialog: false, tabSheet: false, popover: false, search: false, pane: "library", readerHistory: false };

test("Android Back dismisses the top-most surface before it navigates, and leaves only from the library", () => {
  for (const [name, surfaces, expected] of [
    ["library with nothing open", {}, "leave"],
    ["a modal dialog", { dialog: true }, "dialog"],
    ["a dialog over every other surface", { dialog: true, tabSheet: true, popover: true, search: true, pane: "reader", readerHistory: true }, "dialog"],
    ["the reader-tab sheet", { tabSheet: true }, "tab-sheet"],
    ["the tab sheet over a picker and the reader", { tabSheet: true, popover: true, pane: "reader", readerHistory: true }, "tab-sheet"],
    ["a picker on the library", { popover: true }, "popover"],
    ["a picker before search and history", { popover: true, search: true, pane: "reader", readerHistory: true }, "popover"],
    ["global search still loading", { search: true }, "search"],
    ["global search over the reader", { search: true, pane: "reader", readerHistory: true }, "search"],
    ["an earlier article in the reader", { pane: "reader", readerHistory: true }, "reader-history"],
    ["the first article in the reader", { pane: "reader" }, "library"],
    ["Settings", { pane: "settings" }, "library"],
    ["Downloads", { pane: "downloads" }, "library"],
    ["Discover", { pane: "catalog" }, "library"],
    // Reader history only counts while the reader is the pane on screen.
    ["stale reader history on another pane", { pane: "maps", readerHistory: true }, "library"],
    ["stale reader history on the library", { readerHistory: true }, "leave"],
  ]) assert.equal(backTarget({ ...library, ...surfaces }), expected, name);
});

test("every surface eventually leads Back to the system, one step at a time", () => {
  // Closing what Back acted on must never loop: each step strictly reduces the state.
  let surfaces = { dialog: true, tabSheet: true, popover: true, search: true, pane: "reader", readerHistory: true };
  const steps = [];
  for (let guard = 0; guard < 10; guard++) {
    const target = backTarget(surfaces);
    steps.push(target);
    if (target === "leave") break;
    surfaces = {
      ...surfaces,
      ...(target === "dialog" && { dialog: false }),
      ...(target === "tab-sheet" && { tabSheet: false }),
      ...(target === "popover" && { popover: false }),
      ...(target === "search" && { search: false }),
      ...(target === "reader-history" && { readerHistory: false }),
      ...(target === "library" && { pane: "library" }),
    };
  }
  assert.deepEqual(steps, ["dialog", "tab-sheet", "popover", "search", "reader-history", "library", "leave"]);
});

test("a hardware Escape closes a dialog or the tab sheet, otherwise goes to the focused shell control", () => {
  assert.equal(escapeTarget({ dialog: true, tabSheet: true }), "dialog");
  assert.equal(escapeTarget({ dialog: true, tabSheet: false }), "dialog");
  assert.equal(escapeTarget({ dialog: false, tabSheet: true }), "tab-sheet");
  assert.equal(escapeTarget({ dialog: false, tabSheet: false }), "focused-control");
});

function fakeDocument(elements) {
  const select = (selectors) => elements.filter((element) =>
    selectors.split(",").some((selector) => element.selectors.includes(selector.trim())));
  return {
    querySelector: (selectors) => select(selectors)[0] ?? null,
    querySelectorAll: (selectors) => select(selectors),
  };
}
const element = (name, ...selectors) => ({ name, selectors, matches: (selector) => selectors.includes(selector) });

test("open popovers are found in the shell first, and a docked reader panel is not one", () => {
  const shellSelectors = SHELL_POPOVERS.split(",").map((selector) => selector.trim());
  const readerSelectors = READER_OVERLAYS.split(",").map((selector) => selector.trim());
  assert.deepEqual(shellSelectors,
    ["details.facet-picker[open]", "details.rw-history-menu[open]", '[role="combobox"][aria-expanded="true"]']);
  assert.deepEqual(readerSelectors, [".rw-findbar", ".rw-article-preview", ".rw-panel"]);
  const picker = element("picker", "details.facet-picker[open]");
  const suggestions = element("suggestions", '[role="combobox"][aria-expanded="true"]');
  const find = element("find", ".rw-findbar");
  const panel = element("panel", ".rw-panel");
  const overlaid = (target) => target.overlaid === true;
  assert.equal(openPopover(fakeDocument([]), overlaid), null);
  assert.deepEqual(openPopover(fakeDocument([find, picker]), overlaid), { kind: "shell", element: picker });
  assert.deepEqual(openPopover(fakeDocument([suggestions]), overlaid), { kind: "shell", element: suggestions });
  assert.deepEqual(openPopover(fakeDocument([find]), overlaid), { kind: "reader", element: find });
  assert.equal(openPopover(fakeDocument([panel]), overlaid), null, "a panel docked beside the article is layout");
  const sheet = { ...panel, overlaid: true };
  assert.deepEqual(openPopover(fakeDocument([sheet]), overlaid), { kind: "reader", element: sheet });
  assert.deepEqual(openPopover(fakeDocument([panel, find]), overlaid), { kind: "reader", element: find });
  const dialogs = [element("first", "dialog[open]"), element("second", "dialog[open]")];
  assert.deepEqual(openDialogs(fakeDocument([picker, ...dialogs])), dialogs);
});

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function bridge({ stale } = {}) {
  const store = new Map(stale === undefined ? [] : [[BACK_LISTENER_KEY, String(stale)]]);
  const state = { listening: [], removed: [], forgotten: [], pending: [], next: 100, errors: [], presses: 0, store };
  return {
    state,
    bridge: {
      listen(handler) {
        const gate = deferred();
        state.pending.push(gate);
        return gate.promise.then(() => {
          const listener = {
            channelId: state.next++,
            handler,
            unregister: async () => {
              if (state.failUnregister) throw new Error("remove refused");
              state.removed.push(listener.channelId);
              state.listening = state.listening.filter((entry) => entry !== listener);
            },
          };
          state.listening.push(listener);
          return listener;
        });
      },
      async forget(channelId) { state.forgotten.push(channelId); },
      storage: {
        getItem: (key) => store.get(key) ?? null,
        setItem: (key, value) => { store.set(key, value); },
        removeItem: (key) => { store.delete(key); },
      },
    },
  };
}
const release = (state) => { for (const gate of state.pending.splice(0)) gate.resolve(); };
async function settle(registration, state) {
  // Registration awaits a round trip; answer each one as it is requested.
  for (let turn = 0; turn < 20; turn++) {
    release(state);
    await new Promise((resolve) => setImmediate(resolve));
  }
  await registration.settled();
}

test("Back belongs to the system until the shell has something to dismiss", async () => {
  const { bridge: native, state } = bridge();
  const registration = createBackRegistration(native, () => state.presses++, (error) => state.errors.push(error));
  await settle(registration, state);
  assert.equal(state.listening.length, 0, "no listener at the library root: the platform default leaves the app");
  assert.equal(state.store.has(BACK_LISTENER_KEY), false);

  registration.want(true);
  await settle(registration, state);
  assert.equal(state.listening.length, 1);
  assert.equal(state.store.get(BACK_LISTENER_KEY), String(state.listening[0].channelId),
    "the live channel is remembered for the next document");
  state.listening[0].handler();
  assert.equal(state.presses, 1);

  registration.want(false);
  await settle(registration, state);
  assert.equal(state.listening.length, 0, "withdrawn again once nothing is open");
  assert.deepEqual(state.removed, [100]);
  assert.equal(state.store.has(BACK_LISTENER_KEY), false);
  assert.deepEqual(state.errors, []);
});

test("a burst of changes and an effect replay never leave two listeners or a stale one", async () => {
  const { bridge: native, state } = bridge();
  const registration = createBackRegistration(native, () => {}, (error) => state.errors.push(error));
  // StrictMode replays the effect: want, cleanup, want - before any round trip resolves.
  registration.want(true);
  registration.want(false);
  registration.want(true);
  await settle(registration, state);
  assert.equal(state.listening.length, 1);
  assert.deepEqual(state.removed, []);
  // Dialog opened and closed before the registration resolved.
  registration.want(false);
  registration.want(true);
  registration.want(false);
  await settle(registration, state);
  assert.equal(state.listening.length, 0);
  assert.equal(state.store.has(BACK_LISTENER_KEY), false);
  // Wanted while the first registration is still in flight, then cleaned up.
  registration.want(true);
  await new Promise((resolve) => setImmediate(resolve));
  registration.want(false);
  await settle(registration, state);
  assert.equal(state.listening.length, 0, "a listener that resolves after cleanup is removed, not leaked");
  assert.deepEqual(state.errors, []);
});

test("a listener left by a replaced document is removed before Back can become a dead key", async () => {
  const { bridge: native, state } = bridge({ stale: 4242 });
  const registration = createBackRegistration(native, () => {}, (error) => state.errors.push(error));
  await settle(registration, state);
  assert.deepEqual(state.forgotten, [4242], "removed even though this document wants no listener");
  assert.equal(state.store.has(BACK_LISTENER_KEY), false);
  registration.want(true);
  await settle(registration, state);
  assert.deepEqual(state.forgotten, [4242], "recovery runs once");
  assert.equal(state.store.get(BACK_LISTENER_KEY), "100");
  assert.deepEqual(state.errors, []);

  for (const junk of ["", "not-a-number", "-1", "1.5"]) {
    const other = bridge({ stale: junk });
    const second = createBackRegistration(other.bridge, () => {}, (error) => other.state.errors.push(error));
    await settle(second, other.state);
    assert.deepEqual(other.state.forgotten, [], `an unusable record (${JSON.stringify(junk)}) is only discarded`);
    assert.equal(other.state.store.has(BACK_LISTENER_KEY), false);
  }
});

test("a recovery that fails is reported and does not cost this document its listener", async () => {
  const { bridge: native, state } = bridge({ stale: 7 });
  const registration = createBackRegistration({ ...native, forget: async () => { throw new Error("forget refused"); } },
    () => {}, (error) => state.errors.push(error.message));
  registration.want(true);
  await settle(registration, state);
  assert.deepEqual(state.errors, ["forget refused"]);
  assert.equal(state.listening.length, 1);
});

test("a failed registration or removal is reported and retried, never assumed", async () => {
  const { bridge: native, state } = bridge();
  const failing = { ...native, listen: async () => { throw new Error("listener refused"); } };
  const refused = createBackRegistration(failing, () => {}, (error) => state.errors.push(error.message));
  refused.want(true);
  await settle(refused, state);
  assert.deepEqual(state.errors, ["listener refused"]);
  assert.equal(state.store.has(BACK_LISTENER_KEY), false);

  const other = bridge();
  const registration = createBackRegistration(other.bridge, () => {}, (error) => other.state.errors.push(error.message));
  registration.want(true);
  await settle(registration, other.state);
  other.state.failUnregister = true;
  registration.want(false);
  await settle(registration, other.state);
  assert.deepEqual(other.state.errors, ["remove refused"]);
  assert.equal(other.state.listening.length, 1);
  assert.equal(other.state.store.get(BACK_LISTENER_KEY), "100", "a listener that may still be registered stays recorded");
  other.state.failUnregister = false;
  registration.want(false);
  await settle(registration, other.state);
  assert.equal(other.state.listening.length, 0, "the next change retries the removal");
  assert.equal(other.state.store.has(BACK_LISTENER_KEY), false);
});

test("a registration works without storage, and a broken one cannot stop it", async () => {
  for (const storage of [null, { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); }, removeItem() { throw new Error("blocked"); } }]) {
    const { bridge: native, state } = bridge();
    const registration = createBackRegistration({ ...native, storage }, () => state.presses++, (error) => state.errors.push(error));
    registration.want(true);
    await settle(registration, state);
    assert.equal(state.listening.length, 1);
    registration.want(false);
    await settle(registration, state);
    assert.equal(state.listening.length, 0);
    assert.deepEqual(state.errors, []);
  }
});

test("the shell registers Back only on Android and never asks Tauri to exit", async () => {
  const [app, back, capability] = await Promise.all([
    readFile(new URL("../src/App.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/android-back.ts", import.meta.url), "utf8"),
    readFile(new URL("../src-tauri/capabilities/main.json", import.meta.url), "utf8"),
  ]);
  const tree = ts.createSourceFile("App.tsx", app, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const effects = [];
  (function visit(node) {
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
        /createBackRegistration|readAndroidSurfaces\(\);\s*\}\);\s*\}\);/.test(node.getText(tree)))
      effects.push(node.arguments[0].getText(tree));
    ts.forEachChild(node, visit);
  })(tree);
  assert.equal(effects.length, 2, "the surface observer and the registration");
  for (const effect of effects)
    assert.match(effect, /^\(\) => \{\s*if \(!isAndroid\) return;/, "nothing is observed or registered on another platform");
  assert.match(app, /const androidBackWanted = isAndroid && !quitting && !mobileSuspended &&\s*\(androidSurfaceOpen \|\| mobileTabsOpen \|\| globalSearch \|\| pane !== "library"\);/);
  // Leaving is the platform default (no listener). The plugin's exit command
  // finishes the task and is not granted to this window.
  for (const source of [app, back]) {
    const code = ts.transpileModule(source, { compilerOptions: { removeComments: true, jsx: ts.JsxEmit.Preserve, target: ts.ScriptTarget.ES2022 } }).outputText;
    assert.doesNotMatch(code, /plugin:app\|exit|\bexit\s*\(/);
  }
  const permissions = JSON.parse(capability).permissions;
  assert.ok(permissions.includes("core:default"), "core:default carries core:app:default (register/remove listener)");
  assert.ok(!permissions.some((permission) => /app:allow-exit/.test(JSON.stringify(permission))));
});
