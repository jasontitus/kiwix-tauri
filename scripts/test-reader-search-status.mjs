import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { searchStatusMessage } from "../src/reader-search-status.ts";

const english = JSON.parse(
  readFileSync(
    new URL("../src/locales/messages/en.json", import.meta.url),
    "utf8",
  ),
);

test("an active search announces that it is searching", () => {
  assert.deepEqual(
    searchStatusMessage({
      busy: true,
      resultCount: 0,
      total: 0,
      error: "Search failed",
      validation: true,
      hasMore: false,
    }),
    { kind: "searching", key: "Searching…" },
  );
});

test("a completed search announces its total without page-local counts", () => {
  assert.deepEqual(
    searchStatusMessage({
      busy: false,
      resultCount: 3,
      total: 3,
      error: "",
      hasMore: false,
    }),
    {
      kind: "results",
      key: "About {count} results",
      values: { count: 3 },
      hasMore: false,
    },
  );
});

test("a search with no matches announces no results", () => {
  assert.deepEqual(searchStatusMessage({ busy: false, resultCount: 0, total: 0 }), {
    kind: "no-results",
    key: "No results",
  });
  assert.deepEqual(
    searchStatusMessage({ busy: false, resultCount: 0, total: null }),
    { kind: "no-results", key: "No results" },
  );
});

test("an empty query announces validation rather than a request failure", () => {
  assert.deepEqual(
    searchStatusMessage({
      busy: false,
      resultCount: null,
      total: null,
      error: "Enter a word or phrase to search your books.",
      validation: true,
    }),
    {
      kind: "validation",
      key: "Enter a word or phrase to search your books.",
    },
  );
});

test("a failed search announces the failure instead of stale results", () => {
  assert.deepEqual(
    searchStatusMessage({
      busy: false,
      resultCount: 4,
      total: 4,
      error: "The search index is unavailable",
      validation: false,
    }),
    { kind: "failed", key: "Search failed" },
  );
});

test("pagination announces the visible row range, and the range changes per page", () => {
  const firstPage = searchStatusMessage({
    busy: false,
    resultCount: 20,
    total: 127,
    offset: 0,
    hasMore: true,
  });
  const secondPage = searchStatusMessage({
    busy: false,
    resultCount: 50,
    total: 127,
    offset: 50,
    hasMore: true,
  });
  assert.deepEqual(firstPage, {
    kind: "page",
    key: "{start}–{end} of {total}",
    values: { start: 1, end: 20, total: 127 },
    hasMore: true,
  });
  assert.deepEqual(secondPage, {
    kind: "page",
    key: "{start}–{end} of {total}",
    values: { start: 51, end: 100, total: 127 },
    hasMore: true,
  });
  assert.notDeepEqual(firstPage, secondPage);
});

test("the result count alone changes the announcement while the total is stable", () => {
  const fewerRows = searchStatusMessage({
    busy: false,
    resultCount: 10,
    total: 200,
  });
  const moreRows = searchStatusMessage({
    busy: false,
    resultCount: 50,
    total: 200,
  });
  assert.deepEqual(fewerRows, {
    kind: "page",
    key: "{start}–{end} of {total}",
    values: { start: 1, end: 10, total: 200 },
    hasMore: false,
  });
  assert.deepEqual(moreRows, {
    kind: "page",
    key: "{start}–{end} of {total}",
    values: { start: 1, end: 50, total: 200 },
    hasMore: false,
  });
  assert.notDeepEqual(fewerRows, moreRows);
});

test("another result page adds a more-results hint", () => {
  assert.deepEqual(
    searchStatusMessage({
      busy: false,
      resultCount: 50,
      total: 200,
      hasMore: true,
    }),
    {
      kind: "page",
      key: "{start}–{end} of {total}",
      values: { start: 1, end: 50, total: 200 },
      hasMore: true,
    },
  );
});

test("a page with no rendered rows keeps the known total announceable", () => {
  assert.deepEqual(
    searchStatusMessage({ busy: false, resultCount: 0, total: 12 }),
    {
      kind: "results",
      key: "About {count} results",
      values: { count: 12 },
      hasMore: false,
    },
  );
});

test("an untouched search panel has nothing to announce", () => {
  assert.equal(
    searchStatusMessage({ busy: false, resultCount: null, total: null }),
    null,
  );
});

test("every announced key exists in the English catalog", () => {
  const statuses = [
    searchStatusMessage({ busy: true, resultCount: null, total: null }),
    searchStatusMessage({
      busy: false,
      resultCount: null,
      total: null,
      error: "Enter a word or phrase to search your books.",
      validation: true,
    }),
    searchStatusMessage({
      busy: false,
      resultCount: null,
      total: null,
      error: "The search index is unavailable",
    }),
    searchStatusMessage({ busy: false, resultCount: 0, total: 0 }),
    searchStatusMessage({ busy: false, resultCount: 20, total: 127 }),
    searchStatusMessage({ busy: false, resultCount: 3, total: 3 }),
  ];
  for (const status of statuses) {
    assert.ok(status);
    assert.ok(
      Object.hasOwn(english, status.key),
      `missing English catalog key: ${status.key}`,
    );
  }
});
