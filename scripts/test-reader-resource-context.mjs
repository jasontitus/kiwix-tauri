import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const tree = ts.createSourceFile("reader-bridge.js", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
let listener;
let linkHref;
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === "linkHref")
    linkHref = node.getText(tree);
  if (ts.isCallExpression(node) && node.expression.getText(tree) === "document.addEventListener" &&
      node.arguments[0]?.text === "contextmenu") listener = node.arguments[1].getText(tree);
  ts.forEachChild(node, visit);
}
visit(tree);
function bridge() {
  const messages = [];
  const location = { href: "http://archive.localhost:8123/index.html", origin: "http://archive.localhost:8123" };
  const receive = vm.runInNewContext(`${linkHref}; (${listener})`, { URL, location, post(type, value) { messages.push({ type, ...value }); } });
  return { messages, location,
    context(resource, anchor = null, trusted = true) {
      let prevented = false;
      receive({ isTrusted: trusted, defaultPrevented: false,
        target: { closest(selector) {
          const tags = selector.split(",").map((item) => item.trim().split("[")[0]);
          if (resource && tags.includes(resource.tagName.toLowerCase())) return resource;
          if (anchor && tags.some((tag) => tag === "a" || tag === "area")) return anchor;
          return null;
        } },
        preventDefault() { prevented = true; },
      });
      return prevented;
    },
  };
}

test("the selected video, audio or image resource wins over an enclosing article link", () => {
  for (const tagName of ["VIDEO", "AUDIO", "IMG"]) {
    const r = bridge();
    const resource = { tagName, currentSrc: `${r.location.origin}/media/chosen.bin`, src: `${r.location.origin}/media/fallback.bin` };
    assert.equal(r.context(resource, { href: `${r.location.origin}/other-article.html` }), true);
    assert.deepEqual(r.messages.map((item) => [item.type, item.url, item.documentUrl]),
      [["resource-context", resource.currentSrc, r.location.href]]);
  }
});

test("resource context cannot authorize an untrusted or external resource", () => {
  const r = bridge();
  r.context({ tagName: "VIDEO", currentSrc: `${r.location.origin}/movie.mp4` }, null, false);
  r.context({ tagName: "IMG", currentSrc: "https://example.org/picture.png" });
  r.context({ tagName: "AUDIO", currentSrc: "blob:http://archive.localhost:8123/id" });
  r.context({ tagName: "VIDEO", currentSrc: "", src: "", getAttribute: () => null });
  assert.deepEqual(r.messages, []);
});

test("an empty source must not save the containing article through reflected src", () => {
  const r = bridge();
  r.context({ tagName: "IMG", currentSrc: "", src: r.location.href, getAttribute: () => "" });
  assert.deepEqual(r.messages, []);
});
