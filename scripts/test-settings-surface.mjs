import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const require = createRequire(import.meta.url);

// Run the shipped autosave module with its real edit state machine. Only its
// imports are replaced: timers, the settings write and the React subscription.
async function autosave() {
  const path = "../src/components/SettingsAutosave.tsx";
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const code = ts.transpileModule(source, {
    fileName: path,
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const saved = { downloadDirectory: "/home/reader/Downloads", catalogUrl: "https://library.example/catalog", zoom: 1, opdsSources: [] };
  const state = { now: 0, timers: new Map(), nextTimer: 1, writes: [], pendingWrites: [], confirmed: { ...saved } };
  const modules = {
    react: {
      useEffect() {}, useState: (value) => [value, () => {}], useSyncExternalStore: (_subscribe, snapshot) => snapshot(),
      // The prompt records whether it can be shown in a layout effect.
      useLayoutEffect: (run) => { run(); },
    },
    "react/jsx-runtime": require("react/jsx-runtime"),
    "../api": {
      api: (command, args) => new Promise((resolve, reject) => {
        state.writes.push({ command, args });
        state.pendingWrites.push({ resolve, reject });
      }),
      getConfirmedSettings: () => state.confirmed,
      isIOS: false,
      registerSettingsDraftFlusher: () => () => {},
      subscribeReaderPreferences: () => () => {},
    },
    "../i18n": { t: (value) => value },
    "./UI": { errorText: String, Modal: "Modal" },
  };
  const exports = {};
  const context = vm.createContext({
    exports,
    require: (name) => modules[name] ?? assert.fail(`Unexpected import: ${name}`),
    window: {
      setTimeout: (run, delay) => {
        const id = state.nextTimer++;
        state.timers.set(id, { run, due: state.now + delay });
        return id;
      },
      addEventListener() {}, removeEventListener() {},
    },
    clearTimeout: (id) => { state.timers.delete(id); },
    document: { addEventListener() {}, removeEventListener() {}, visibilityState: "visible" },
    Error, Map, Set, Number, String, Promise, Object, URL,
  });
  vm.runInContext(code, context);
  const handlers = { onRefresh() {}, onError() {} };
  // App mounts the prompt for the life of the shell.
  exports.SettingsPolicyConfirmation({ available: true });
  return {
    state,
    exports,
    field: (key) => exports.useSettingsAutosave(saved, handlers).input(key),
    advance(milliseconds) {
      state.now += milliseconds;
      for (const [id, timer] of [...state.timers])
        if (timer.due <= state.now) { state.timers.delete(id); timer.run(); }
    },
    // The App-level prompt: null while nothing asks for confirmation.
    prompt: (available = true) => exports.SettingsPolicyConfirmation({ available }),
    // The status line under the Settings heading.
    status() {
      const [line] = [].concat(exports.SettingsSaveStatus().props.children);
      return { state: line.props["data-state"], text: line.props.children };
    },
    key: (field, key, extra = {}) => {
      let prevented = false;
      field.onKeyDown({ key, preventDefault() { prevented = true; }, ...extra });
      return prevented;
    },
  };
}
const type = (field, value) => field.onChange({ target: { value } });

test("a pause while typing a download directory neither saves nor opens the confirmation", async () => {
  const settings = await autosave();
  type(settings.field("downloadDirectory"), "/media/re");
  assert.equal(settings.state.timers.size, 0, "no debounce is armed for a change that needs confirmation");
  settings.advance(600);
  settings.advance(5000);
  assert.equal(settings.prompt(), null, "the prompt must not open over a half-typed path");
  assert.deepEqual(settings.state.writes, []);
  assert.equal(settings.field("downloadDirectory").value, "/media/re", "the typed text is kept in the field");

  // The rest of the path arrives after the pause.
  type(settings.field("downloadDirectory"), "/media/reader/zim");
  settings.advance(600);
  assert.equal(settings.prompt(), null);

  // Leaving the field is the commit; it asks before it saves.
  settings.field("downloadDirectory").onBlur();
  const prompt = settings.prompt();
  assert.ok(prompt, "leaving the field asks for confirmation");
  assert.match(JSON.stringify(prompt.props.children), /\/media\/reader\/zim/, "with the finished path");
  assert.deepEqual(settings.state.writes, [], "nothing is saved before consent");
});

test("a typed download directory is shown as not saved, never as being saved", async () => {
  const settings = await autosave();
  assert.deepEqual(settings.status(), { state: "auto", text: "Changes are saved automatically." });
  type(settings.field("downloadDirectory"), "/media/reader/zim");
  settings.advance(5000);
  assert.deepEqual(settings.status(), {
    state: "confirm", text: "This change is not saved yet. Leave the field or press Enter to review it.",
  });
  // Another preference saving at the same time is still reported as saving.
  type(settings.field("catalogUrl"), "https://mirror.example/catalog");
  assert.equal(settings.status().text, "Saving…");
  settings.advance(600);
  settings.state.pendingWrites.shift().resolve({ ...settings.state.confirmed, catalogUrl: "https://mirror.example/catalog" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settings.status().text, "This change is not saved yet. Leave the field or press Enter to review it.");
  // Once the prompt is open the line says so.
  settings.field("downloadDirectory").onBlur();
  assert.deepEqual(settings.status(), { state: "confirm", text: "Review the preference change to save it." });
});

test("Enter in the download directory field opens the review, and only there", async () => {
  const settings = await autosave();
  type(settings.field("downloadDirectory"), "/srv/zim");
  // A key that composes text, or any other key, is not a commit.
  assert.equal(settings.key(settings.field("downloadDirectory"), "Enter", { nativeEvent: { isComposing: true } }), false);
  assert.equal(settings.key(settings.field("downloadDirectory"), "a"), false);
  assert.equal(settings.prompt(), null);
  assert.equal(settings.key(settings.field("downloadDirectory"), "Enter"), true);
  assert.match(JSON.stringify(settings.prompt().props.children), /\/srv\/zim/);
  assert.deepEqual(settings.state.writes, [], "nothing is saved before consent");
  // Fields that save on their own keep the browser's Enter (a newline in the source list).
  type(settings.field("opdsSources"), "https://one.example/catalog");
  assert.equal(settings.key(settings.field("opdsSources"), "Enter"), false);
});

test("Quit or close with an unreviewed download directory neither applies nor discards it", async () => {
  const settings = await autosave();
  type(settings.field("downloadDirectory"), "/media/reader/zim");
  // The shell is hidden for Quit/close before the final flush: no prompt can be shown.
  assert.equal(settings.prompt(false), null);
  settings.exports.useSettingsAutosave(settings.state.confirmed, { onRefresh() {} }).flush();
  settings.advance(5000);
  assert.deepEqual(settings.state.writes, [], "an unconfirmed directory is never applied by the flush");
  assert.equal(settings.field("downloadDirectory").value, "/media/reader/zim", "nor thrown away behind the person's back");
  assert.equal(settings.status().text, "This change is not saved yet. Leave the field or press Enter to review it.");
  // If the app keeps running (Quit cancelled), the change can still be reviewed.
  assert.equal(settings.prompt(true), null);
  settings.field("downloadDirectory").onBlur();
  assert.ok(settings.prompt(true));
  // Preferences that need no confirmation are still flushed on the way out.
  const other = await autosave();
  type(other.field("catalogUrl"), "https://mirror.example/catalog");
  other.prompt(false);
  other.exports.useSettingsAutosave(other.state.confirmed, { onRefresh() {} }).flush();
  assert.equal(other.state.writes.length, 1);
});

test("submitting the form commits a typed download directory through the same confirmation", async () => {
  const settings = await autosave();
  type(settings.field("downloadDirectory"), "/srv/zim");
  settings.exports.useSettingsAutosave(settings.state.confirmed, { onRefresh() {} }).flush();
  assert.ok(settings.prompt());
  assert.deepEqual(settings.state.writes, []);
});

test("fields that need no confirmation still save after the typing pause", async () => {
  const settings = await autosave();
  type(settings.field("catalogUrl"), "https://mirror.example/catalog");
  assert.equal(settings.state.timers.size, 1);
  settings.advance(599);
  assert.deepEqual(settings.state.writes, []);
  settings.advance(1);
  assert.equal(settings.prompt(), null);
  assert.deepEqual(JSON.parse(JSON.stringify(settings.state.writes)),
    [{ command: "save_settings", args: { settings: { catalogUrl: "https://mirror.example/catalog" } } }]);
});

test("Settings and Help describe windows, the File menu and New window only on desktop", async () => {
  const path = "../src/components/Settings.tsx";
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  const tree = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const literals = [];
  (function visit(node) {
    if (ts.isStringLiteralLike(node)) literals.push(node);
    ts.forEachChild(node, visit);
  })(tree);
  const desktopOnly = (node) => {
    for (let child = node, parent = node.parent; parent; child = parent, parent = parent.parent) {
      if (ts.isBinaryExpression(parent) && parent.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken &&
          parent.left.getText(tree) === "isDesktop" && parent.right === child) return true;
      if (ts.isConditionalExpression(parent) && parent.condition.getText(tree) === "isDesktop" &&
          parent.whenTrue === child) return true;
    }
    return false;
  };
  const mentions = literals.filter((node) => /File →|^Windows, reader tabs|Shift \+ N|^New window$/.test(node.text));
  assert.equal(mentions.length, 3, "the session paragraph and the New window shortcut row");
  for (const node of mentions) assert.ok(desktopOnly(node), `desktop-only copy is not gated: ${node.text}`);
  const mobile = literals.filter((node) => node.text === "Reader tabs and reading positions reopen at launch. Private and read-only profiles are never restored.");
  assert.equal(mobile.length, 1, "phones and tablets get a paragraph without windows or a File menu");
  assert.ok(!desktopOnly(mobile[0]));
  assert.match(source, /import \{[^}]*\bisDesktop\b[^}]*\} from "\.\.\/api";/);
});
