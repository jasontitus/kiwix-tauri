import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

// Static contract between the shell, the archive bridge and the storage host.
// A handler nothing can reach is dead code that a test double can keep alive;
// a command nothing handles is a control that silently does nothing.
const read = (path) => readFile(new URL(path, import.meta.url), "utf8");
const [reader, bridge, storageHost, storageBridge, server] = await Promise.all([
  read("../src/components/Reader.tsx"), read("../src/reader-bridge.js"),
  read("../src/reader-storage.ts"), read("../src/storage-bridge.js"),
  read("../src-tauri/src/server.rs"),
]);

function shellCommands() {
  const tree = ts.createSourceFile("Reader.tsx", reader, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const names = new Set();
  const literals = (type) => {
    if (ts.isUnionTypeNode(type)) type.types.forEach(literals);
    else if (ts.isLiteralTypeNode(type) && ts.isStringLiteral(type.literal)) names.add(type.literal.text);
  };
  const members = (type) => {
    if (ts.isUnionTypeNode(type)) type.types.forEach(members);
    else if (ts.isTypeLiteralNode(type))
      for (const member of type.members)
        if (ts.isPropertySignature(member) && member.name.getText(tree) === "command") literals(member.type);
  };
  for (const statement of tree.statements)
    if (ts.isTypeAliasDeclaration(statement) && statement.name.text === "ReaderCommand") members(statement.type);
  return names;
}

function bridgeCommands() {
  const tree = ts.createSourceFile("reader-bridge.js", bridge, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const names = new Set();
  const visit = (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "command") {
      const cases = (child) => {
        if (ts.isCaseClause(child) && ts.isStringLiteral(child.expression)) names.add(child.expression.text);
        ts.forEachChild(child, cases);
      };
      cases(node);
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return names;
}

test("the shell's reader commands and the bridge's command handlers are the same set", () => {
  const shell = shellCommands();
  const handled = bridgeCommands();
  assert.ok(shell.size > 10 && handled.size > 10, "locate the ReaderCommand union and the bridge command switch");
  assert.deepEqual([...handled].filter((name) => !shell.has(name)), [],
    "every command the bridge handles is one the shell can send");
  assert.deepEqual([...shell].filter((name) => !handled.has(name)), [],
    "every command the shell can send is handled by the bridge");
});

const storageTypes = (text, pattern) => new Set([...text.matchAll(pattern)].map((match) => match[1]));

test("every storage message a side handles is posted by its counterpart", () => {
  const hostHandles = storageTypes(storageHost, /\.type\s*[!=]==\s*"(kiwix-storage-[a-z-]+)"/g);
  const hostPosts = storageTypes(storageHost, /type:\s*"(kiwix-storage-[a-z-]+)"/g);
  const bridgeHandles = storageTypes(storageBridge, /\.type\s*[!=]==\s*"(kiwix-storage-[a-z-]+)"/g);
  const bridgePosts = storageTypes(storageBridge, /type:\s*"(kiwix-storage-[a-z-]+)"/g);
  // The retirement page is served by the archive server; its script is a Rust string.
  const retirementPosts = storageTypes(server, /type:\\"(kiwix-storage-[a-z-]+)\\"/g);
  assert.ok(hostHandles.size >= 5 && bridgeHandles.size >= 5 && retirementPosts.size >= 1,
    "locate the storage message handlers and senders");
  const toHost = new Set([...bridgePosts, ...retirementPosts, ...hostPosts]);
  assert.deepEqual([...hostHandles].filter((type) => !toHost.has(type)), [],
    "the host handles only messages some producer (or its own fence) posts");
  assert.deepEqual([...bridgeHandles].filter((type) => !hostPosts.has(type)), [],
    "the storage bridge handles only messages the host posts");
  assert.deepEqual([...bridgePosts].filter((type) => !hostHandles.has(type)), [],
    "the host handles every message the storage bridge posts");
});
