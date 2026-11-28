import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { isRustTestSource, portablePath, rustInventory } from "./error-inventory.mjs";
import { describeDifference, parseArguments, renderDocument } from "./generate-error-docs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const script = path.join(root, "scripts/generate-error-docs.mjs");
const committed = path.join(root, "docs/ERRORS.md");
const run = (...args) => spawnSync(process.execPath, [script, ...args], { cwd: root, encoding: "utf8" });

test("docs/ERRORS.md matches the error copy in the current sources", () => {
  const before = readFileSync(committed);
  const result = run("--check");
  assert.deepEqual(readFileSync(committed), before, "--check must never write the document");
  assert.equal(result.status, 0, result.stderr);
});

test("only a bare invocation writes; help and unrecognized arguments leave the document alone", () => {
  const before = readFileSync(committed);
  const help = run("--help");
  assert.equal(help.status, 0);
  assert.match(help.stdout, /^Usage: /);
  for (const args of [["--chek"], ["--check", "extra"], ["docs/ERRORS.md"]]) {
    const result = run(...args);
    assert.equal(result.status, 2, args.join(" "));
    assert.match(result.stderr, /Unrecognized arguments: /);
    assert.match(result.stderr, /Usage: /);
  }
  assert.deepEqual(readFileSync(committed), before);
  assert.equal(parseArguments([]), "write");
  assert.equal(parseArguments(["--check"]), "check");
  assert.equal(parseArguments(["-h"]), "help");
});

test("the document omits line numbers and does not depend on scan order", () => {
  const rust = [
    { file: "src-tauri/src/b.rs", line: 40, text: "Second file | piped" },
    { file: "src-tauri/src/a.rs", line: 900, text: "Zebra was not found" },
    { file: "src-tauri/src/a.rs", line: 12, text: "Archive was not found" },
    { file: "src-tauri/src/b.rs", line: 7, text: "Archive was not found" },
  ];
  const document = renderDocument({ rust, frontendErrors: [], native: [] });
  const rows = document.split("\n").filter((line) => line.startsWith("| src-tauri/"));
  assert.deepEqual(rows, [
    "| src-tauri/src/a.rs | Archive was not found |  |",
    "| src-tauri/src/a.rs | Zebra was not found |  |",
    "| src-tauri/src/b.rs | Second file \\| piped |  |",
  ]);
  // Moving a literal within its file, or listing the files in another order, changes nothing.
  const moved = rust.map((entry) => ({ ...entry, line: entry.line + 500 })).reverse();
  assert.equal(renderDocument({ rust: moved, frontendErrors: [], native: [] }), document);
});

test("the check names added and removed literals instead of only failing", () => {
  const entry = (text) => ({ file: "src-tauri/src/a.rs", line: 1, text });
  const render = (...texts) => renderDocument({ rust: texts.map(entry), frontendErrors: [], native: [] });
  const current = render("Kept message here", "New message here");
  assert.equal(describeDifference(current, current), null);
  const difference = describeDifference(render("Kept message here", "Old message here"), current);
  assert.match(difference, /Rows the current sources produce that the file lacks: 1\n {2}\| src-tauri\/src\/a\.rs \| New message here \|/);
  assert.match(difference, /Rows in the file that the current sources no longer produce: 1\n {2}\| src-tauri\/src\/a\.rs \| Old message here \|/);
  assert.doesNotMatch(difference, /Kept message here/);
  assert.match(difference, /npm run docs:errors/);
  assert.match(describeDifference(current.replace("# Error copy inventory", "# Edited"), current), /The rows match/);
});

test("test-only Rust sources are excluded whatever the host's path separator", async () => {
  const windows = portablePath("src-tauri\\src\\localswarm\\tests.rs", "\\");
  assert.equal(windows, "src-tauri/src/localswarm/tests.rs");
  for (const file of [windows, "src-tauri/src/store_tests.rs", portablePath("src-tauri\\src\\store_tests.rs", "\\"), "tests.rs"])
    assert.equal(isRustTestSource(file), true, file);
  for (const file of ["src-tauri/src/store.rs", "src-tauri/src/contests.rs", "src-tauri/src/tests/helper.rs"])
    assert.equal(isRustTestSource(file), false, file);
  // The inventory stores the portable form and contains no test-only source.
  const files = new Set((await rustInventory()).map((entry) => entry.file));
  assert.ok(files.size > 0);
  for (const file of files) {
    assert.ok(file.startsWith("src-tauri/src/") && !file.includes("\\"), file);
    assert.equal(isRustTestSource(file), false, file);
  }
});
