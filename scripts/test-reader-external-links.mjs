import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Reader.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let openExternal;
let blocked;
let dialog;
let validExternal;
function visit(node) {
  if (ts.isVariableDeclaration(node)) {
    if (node.name.getText(tree) === "openExternal") openExternal = node.initializer;
    if (node.name.getText(tree) === "externalLinksBlocked") blocked = node.initializer;
  }
  if (ts.isFunctionDeclaration(node) && node.name?.text === "validExternal") validExternal = node;
  if (ts.isJsxElement(node) && node.openingElement.tagName.getText(tree) === "Dialog" &&
      node.openingElement.attributes.getText(tree).includes('t("External links are blocked")')) dialog = node;
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(openExternal && blocked && dialog && validExternal, "exercise the production handoff and confirmation dialog");
const compiled = ts.transpileModule(`
  ${validExternal.getText(tree)}
  globalThis.openExternal = ${openExternal.getText(tree)};
  globalThis.renderDialog = () => {
    const externalLinksBlocked = ${blocked.getText(tree)};
    return (${dialog.getText(tree)});
  };
`, { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;

function reader() {
  const settings = { externalLinks: "ask" };
  const calls = [];
  const errors = [];
  const context = vm.createContext({
    URL, settings, latest: { current: { settings } }, alive: { current: true },
    external: { url: "https://example.org/article" }, externalBusy: false,
    React: { createElement: (tag, props, ...children) => ({ tag, props, children }) },
    Dialog: "Dialog", ExternalLink: "ExternalLink", ArrowUpRight: "ArrowUpRight",
    canRestoreDialogFocus: () => true, t: (value) => value,
    setExternal(value) { context.external = typeof value === "function" ? value(context.external) : value; },
    setExternalBusy(value) { context.externalBusy = value; },
    report: (value) => errors.push(value),
    async api(command, args) { calls.push({ command, args }); },
  });
  vm.runInContext(compiled, context);
  return { context, settings, calls, errors };
}
function descendants(node) {
  if (!node || typeof node !== "object") return [];
  return [node, ...(node.children ?? []).flat(Infinity).flatMap(descendants)];
}

test("an open confirmation reacts to another window blocking external links", async () => {
  const r = reader();
  assert.equal(r.context.renderDialog().props.title, "Open outside your offline library?");
  r.settings.externalLinks = "deny";
  const view = r.context.renderDialog();
  assert.equal(view.props.title, "External links are blocked");
  assert.equal(descendants(view).filter((node) => node.tag === "button").length, 1);
  await r.context.openExternal();
  assert.deepEqual(r.calls, []);
  assert.ok(r.context.external, "keep the blocked address and explanation visible");
});

test("confirmed current-policy links still reach the native boundary", async () => {
  const r = reader();
  await r.context.openExternal();
  assert.equal(r.calls.length, 1);
  assert.equal(r.calls[0].command, "open_external");
  assert.equal(r.calls[0].args.url, "https://example.org/article");
  assert.equal(r.calls[0].args.confirmed, true);
  assert.equal(r.context.external, null);
  assert.equal(r.context.externalBusy, false);
});

test("busy and unsafe handoffs cannot issue another native request", async () => {
  const r = reader();
  r.context.externalBusy = true;
  await r.context.openExternal();
  r.context.externalBusy = false;
  r.context.external.url = "file:///tmp/untrusted";
  await r.context.openExternal();
  assert.deepEqual(r.calls, []);
});
