import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("Reader.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const declarations = tree.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === "readerDocumentUrl" ||
  ts.isVariableStatement(node) && node.declarationList.declarations.some((decl) => decl.name.getText(tree) === "MISS_MARKER"));
const context = vm.createContext({ URL });
vm.runInContext(ts.transpileModule(declarations.map((node) => node.getText(tree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText, context);

test("missing-entry recovery keeps live document identity while removing only the persisted marker", () => {
  const actual = "http://archive.localhost:8123/book/A/Main?view=wide&kiwix-miss=1#Section";
  const identity = context.readerDocumentUrl(actual);
  assert.equal(identity.url, actual, "selection, highlights and link context use the real location.href");
  assert.equal(identity.persistedUrl, "http://archive.localhost:8123/book/A/Main?view=wide#Section");
  assert.equal(identity.missed, true);
});

test("ordinary archive URL query spelling remains byte-identical", () => {
  const actual = "http://archive.localhost:8123/book/A/Main?query=a%20b&encoded=%2f#Section";
  const identity = context.readerDocumentUrl(actual);
  assert.equal(identity.url, actual);
  assert.equal(identity.persistedUrl, actual);
  assert.equal(identity.missed, false);
});
