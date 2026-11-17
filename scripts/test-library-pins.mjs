import assert from "node:assert/strict";
import test from "node:test";
import {
  orderLibraryBooks,
  usablePinnedBooks,
} from "../src/library-pins.ts";

const book = (id) => ({ id, title: id.toUpperCase() });
const titles = (books) => books.map((entry) => entry.id);

test("pinned archives lead in pin order and unpinning restores the previous sort", () => {
  const sorted = ["a", "b", "c", "d"].map(book);
  assert.deepEqual(titles(orderLibraryBooks(sorted, ["d", "b"])), ["d", "b", "a", "c"]);
  assert.deepEqual(titles(orderLibraryBooks(sorted, ["b"])), ["b", "a", "c", "d"]);
  assert.deepEqual(titles(orderLibraryBooks(sorted, [])), ["a", "b", "c", "d"]);
});

test("pin order survives a settings round trip", () => {
  const saved = JSON.stringify({
    pinnedBookIds: ["wiktionary", "wikipedia"],
  });
  const loaded = JSON.parse(saved).pinnedBookIds;
  const books = ["wikipedia", "wiktionary", "wikisource"].map(book);
  assert.deepEqual(titles(orderLibraryBooks(books, loaded)), [
    "wiktionary",
    "wikipedia",
    "wikisource",
  ]);
});

test("removed archives and duplicates never consume a pin slot", () => {
  const books = ["a", "b"].map(book);
  assert.deepEqual(usablePinnedBooks(["missing", "b", "b", "a"], books), ["b", "a"]);
  assert.deepEqual(usablePinnedBooks([], books), []);
  assert.deepEqual(titles(orderLibraryBooks(books, ["missing", "b"])), ["b", "a"]);
});
