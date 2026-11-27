import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Printing has three real paths: the desktop system print panel, Android's
// system print dialog (its PDF export), and window.print() in the browser-only
// preview. A mobile webview does not implement window.print() for the archive
// frame, so the frame command must never be the mobile path.
const source = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Reader.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let printArticle;
let printRow;
function visit(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(tree) === "printArticle") printArticle = node.initializer;
  if (ts.isJsxElement(node) && node.openingElement.tagName.getText(tree) === "button" &&
      node.getText(tree).includes('t("Choose a printer or PDF in the system dialog.")')) printRow = node;
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(printArticle && printRow, "exercise the production print action and its row");

function reader(platform) {
  const sent = [];
  const exports = [];
  const notices = [];
  const context = vm.createContext({
    isNative: true, isAndroid: false, canPrintArticlePdf: false, ...platform,
    activeFrame: { current: { location: { mime: "text/html; charset=utf-8" } } },
    latest: { current: { settings: { readOnly: false } } },
    isHtml: (mime) => mime.startsWith("text/html"),
    t: (text) => text,
    send: (command) => sent.push(command.command),
    exportArticle: async (...args) => { exports.push(args); },
    setNotice: (value) => notices.push(value),
  });
  vm.runInContext(ts.transpileModule(`globalThis.printArticle = ${printArticle.getText(tree)};`,
    { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return { context, sent, exports, notices };
}

test("Android prints through its system print dialog, never the frame's window.print()", () => {
  const r = reader({ isAndroid: true });
  r.context.printArticle();
  assert.deepEqual(r.sent, [], "the dead frame print command is not sent");
  assert.deepEqual(r.exports, [["pdf"]], "the PDF export is the Android print dialog");
  assert.deepEqual(r.notices, []);
});

test("iOS says that printing is unavailable instead of doing nothing", () => {
  const r = reader({});
  r.context.printArticle();
  assert.deepEqual(r.sent, []);
  assert.deepEqual(r.exports, []);
  assert.deepEqual(r.notices, ["Printing is not available on this device. Save or share the article as a PDF instead."]);
});

test("desktop keeps the native print panel and the browser preview keeps window.print()", () => {
  const desktop = reader({ canPrintArticlePdf: true });
  desktop.context.printArticle();
  assert.deepEqual(desktop.exports, [["pdf", false, true]]);
  assert.deepEqual(desktop.sent, []);
  const preview = reader({ isNative: false });
  preview.context.printArticle();
  assert.deepEqual(preview.sent, ["print"]);
  assert.deepEqual(preview.exports, []);
});

test("native printing still needs an exportable article and write permission", () => {
  for (const platform of [{ isAndroid: true }, { canPrintArticlePdf: true }, {}]) {
    const readOnly = reader(platform);
    readOnly.context.latest.current.settings.readOnly = true;
    readOnly.context.printArticle();
    const media = reader(platform);
    media.context.activeFrame.current.location.mime = "image/png";
    media.context.printArticle();
    const absent = reader(platform);
    absent.context.activeFrame.current = null;
    absent.context.printArticle();
    for (const r of [readOnly, media, absent])
      assert.deepEqual([r.sent, r.exports, r.notices], [[], [], []]);
  }
});

test("the separate print row is not offered on mobile", () => {
  // Android's working print dialog is the PDF row below it; iOS has no print path.
  let guarded = false;
  for (let node = printRow.parent; node; node = node.parent)
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken &&
        node.left.getText(tree) === "!isMobile") guarded = true;
  assert.equal(guarded, true);
  assert.match(source, /\.\.\.\(isIOS \? \[\] : \[\[t\("Print \/ save as PDF"\), t\("⌘ \/ Ctrl P"\)\]\]\)/,
    "the shortcut list does not advertise printing where there is none");
});
