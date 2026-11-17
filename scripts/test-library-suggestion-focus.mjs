import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/components/Library.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Library.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let suggestionEffect;
const functions = new Map();
const handlers = new Map();
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name) functions.set(node.name.text, node);
  if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
      node.arguments[0]?.getText(tree).includes("void loadSuggestions(prefix")) suggestionEffect = node;
  if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(tree) === "input" &&
      node.attributes.properties.some((attribute) => ts.isJsxAttribute(attribute) &&
        attribute.name.getText(tree) === "aria-label" && attribute.initializer?.getText(tree).includes('"Search articles"'))) {
    for (const attribute of node.attributes.properties) {
      if (ts.isJsxAttribute(attribute) && ["onFocus", "onBlur"].includes(attribute.name.getText(tree))) {
        handlers.set(attribute.name.getText(tree), attribute.initializer.expression.getText(tree));
      }
    }
  }
  ts.forEachChild(node, visit);
}
visit(tree);
const effectCode = ts.transpileModule(`globalThis.effect = ${suggestionEffect.arguments[0].getText(tree)};
globalThis.dependencies = () => ${suggestionEffect.arguments[1].getText(tree)};
${["closeSuggestions", "loadSuggestions", "loadMoreSuggestions"].map((name) => functions.get(name).getText(tree)).join("\n")}
globalThis.focus = ${handlers.get("onFocus") || "() => {}"};
globalThis.blur = ${handlers.get("onBlur")};`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

// Run the production debounce effect, focus handlers and asynchronous result
// admission. The controllable service/timer boundaries expose late completions
// without browser timing or a new component-test framework.
function shell() {
  let nextTimer = 0;
  const timers = new Map();
  const pending = [];
  const context = vm.createContext({
    search: "animal", selected: ["wiki"], searchOpen: true, searchFocused: false,
    suggestionId: { current: 0 }, suggestionPrefix: { current: "" },
    suggestionScope: { current: [] }, suggestionMore: { current: false },
    suggestions: [], suggestionToken: null, suggestionOpen: false,
    suggestionActive: -1, suggestionLoading: false,
    SUGGESTION_DEBOUNCE_MS: 120, SUGGESTION_PAGE_SIZE: 15,
    capturePrivacyContext: () => ({ generation: 1 }),
    diagnostics: { record(error) { throw error; } },
    ERROR_SITES: { "library.suggestion-refresh": "suggest" },
    window: {
      setTimeout(callback) { const id = ++nextTimer; timers.set(id, callback); return id; },
      clearTimeout(id) { timers.delete(id); },
    },
    searchRequest: { current: { suggest(parameters) {
      return new Promise((resolve) => pending.push({ parameters, resolve }));
    } } },
  });
  for (const field of ["SearchFocused", "Suggestions", "SuggestionToken", "SuggestionOpen", "SuggestionActive", "SuggestionLoading"]) {
    const key = field[0].toLowerCase() + field.slice(1);
    context[`set${field}`] = (value) => { context[key] = typeof value === "function" ? value(context[key]) : value; };
  }
  vm.runInContext(effectCode, context);
  let previous;
  let cleanup;
  function render() {
    const dependencies = context.dependencies();
    if (previous && dependencies.every((value, index) => Object.is(value, previous[index]))) return;
    cleanup?.();
    previous = dependencies;
    cleanup = context.effect();
  }
  render();
  return {
    context, pending,
    focus() { context.focus(); render(); },
    blur() { context.blur(); render(); },
    timers() { return timers.size; },
    tick() { const callbacks = [...timers.values()]; timers.clear(); callbacks.forEach((callback) => callback()); },
    async finish(index, page) { pending[index].resolve(page); await Promise.resolve(); await Promise.resolve(); },
  };
}
const page = (path, nextToken = null) => ({ suggestions: [{ bookId: "wiki", path, title: path }], nextToken });

test("leaving search before debounce prevents a popup over the newly focused control", () => {
  const app = shell();
  app.focus();
  assert.equal(app.timers(), 1);
  app.blur();
  app.tick();
  assert.equal(app.pending.length, 0);
  assert.equal(app.context.suggestionOpen, false);
});

test("blur retires in-flight suggestions; refocus admits only the new result", async () => {
  const app = shell();
  app.focus();
  app.tick();
  app.blur();
  await app.finish(0, page("stale"));
  assert.equal(app.context.suggestionOpen, false);
  assert.deepEqual(Array.from(app.context.suggestions), []);
  app.focus();
  app.tick();
  await app.finish(1, page("Animal"));
  assert.equal(app.context.suggestionOpen, true);
  assert.equal(app.context.suggestions[0].path, "Animal");
});

test("a dismissed continuation cannot append results or reopen the popup", async () => {
  const app = shell();
  app.focus();
  app.tick();
  await app.finish(0, page("Animal", "continuation"));
  const continuation = vm.runInContext("loadMoreSuggestions()", app.context);
  app.blur();
  await app.finish(1, page("Animals as food"));
  await continuation;
  assert.equal(app.context.suggestionOpen, false);
  assert.deepEqual(Array.from(app.context.suggestions), []);
  assert.equal(app.context.suggestionToken, null);
});
