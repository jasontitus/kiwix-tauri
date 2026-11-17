import assert from "node:assert/strict";
import test from "node:test";
import { displayPath } from "../src/display-path.ts";

test("Windows verbatim drive and UNC paths are shown in their ordinary form", () => {
  assert.equal(displayPath("\\\\?\\C:\\Users\\me\\Downloads\\a.zim"), "C:\\Users\\me\\Downloads\\a.zim");
  assert.equal(displayPath("\\\\?\\UNC\\server\\share\\a.zim"), "\\\\server\\share\\a.zim");
});

test("other paths are unchanged", () => {
  for (const path of [
    "C:\\Users\\me\\a.zim",
    "\\\\server\\share\\a.zim",
    "\\\\?\\Volume{0000}\\a.zim",
    "\\\\?\\C:",
    "/Users/me/a.zim",
    "",
  ]) assert.equal(displayPath(path), path);
});
