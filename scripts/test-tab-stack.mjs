import assert from "node:assert/strict";
import test from "node:test";
import { insertTab, removeReopened } from "../src/tab-stack.ts";

const tab = (id) => ({ id, bookId: null, url: "" });

test("a foreground tab is appended and a background tab follows the active one", () => {
  const tabs = [tab("a"), tab("b"), tab("c")];
  assert.deepEqual(insertTab(tabs, tab("x"), "a", false).map((t) => t.id), ["a", "b", "c", "x"]);
  assert.deepEqual(insertTab(tabs, tab("x"), "a", true).map((t) => t.id), ["a", "x", "b", "c"]);
});

test("reopening a closed tab twice before a re-render cannot duplicate its identity", () => {
  // Undo-close reads the closed stack from its render closure; two shortcut
  // events handled before React re-renders both see the same top entry.
  const closed = [tab("older"), tab("x")];
  let tabs = [tab("a")];
  let stack = closed;
  for (let press = 0; press < 2; press++) {
    const top = closed[closed.length - 1];
    const next = insertTab(tabs, top, "a", false);
    if (next) {
      tabs = next;
      stack = removeReopened(stack, top.id);
    }
  }
  assert.deepEqual(tabs.map((t) => t.id), ["a", "x"]);
  // The unrelated, older closed tab is still reopenable.
  assert.deepEqual(stack.map((t) => t.id), ["older"]);
});

test("removing a reopened tab only removes that tab", () => {
  const stack = [tab("a"), tab("b"), tab("c")];
  assert.deepEqual(removeReopened(stack, "b").map((t) => t.id), ["a", "c"]);
  assert.deepEqual(removeReopened(stack, "missing").map((t) => t.id), ["a", "b", "c"]);
});
