import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import nodeTest from "node:test";
import vm from "node:vm";
import ts from "typescript";

const audioSnapshotsOnly = process.argv.includes("--audio-snapshots");
const test = audioSnapshotsOnly ? () => {} : nodeTest;
const bridge = await readFile(new URL("../src/reader-bridge.js", import.meta.url), "utf8");
const tree = ts.createSourceFile("reader-bridge.js", bridge, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
let capture;
function visit(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === "exportSnapshot") capture = node.getText(tree);
  ts.forEachChild(node, visit);
}
visit(tree);
assert.ok(capture, "production bridge export operation exists");
const escape = (text) => String(text).replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;");
function textNode(value) {
  return {
    nodeType: 3,
    nodeValue: String(value),
    get textContent() { return this.nodeValue; },
    get outerHTML() { return escape(this.nodeValue); },
  };
}
class Element {
  constructor(tag, properties = {}) {
    this.nodeType = 1;
    this.tagName = tag.toUpperCase();
    this.localName = tag.toLowerCase();
    this.attrs = new Map();
    this.childNodes = [];
    this.styles = { display: "block", "background-image": "none" };
    this.style = { setProperty: (name, value) => this.attrs.set("style", `${this.attrs.get("style") || ""}${name}:${value};`) };
    Object.assign(this, properties);
  }
  get textContent() { return this.childNodes.map((node) => node.textContent).join(""); }
  set textContent(value) {
    this.childNodes = [];
    if (String(value)) this.appendChild(textNode(value));
  }
  get attributes() { return Array.from(this.attrs, ([name, value]) => ({ name, value })); }
  setAttribute(name, value) { this.attrs.set(name, String(value)); }
  getAttribute(name) { return this.attrs.get(name) ?? null; }
  hasAttribute(name) { return this.attrs.has(name); }
  removeAttribute(name) { this.attrs.delete(name); }
  toggleAttribute(name, enabled) { if (enabled) this.setAttribute(name, ""); else this.removeAttribute(name); }
  appendChild(child) { this.childNodes.push(child); child.parentElement = this; return child; }
  cloneNode() { const clone = new Element(this.tagName); clone.attrs = new Map(this.attrs); return clone; }
  querySelector(tag) {
    for (const child of this.childNodes) {
      if (child.localName === tag) return child;
      const result = child.querySelector?.(tag);
      if (result) return result;
    }
    return null;
  }
  get src() { return this.getAttribute("src") || ""; }
  set src(value) { this.setAttribute("src", value); }
  get outerHTML() {
    const attrs = Array.from(this.attrs, ([name, value]) => ` ${name}="${escape(value)}"`).join("");
    const content = this.childNodes.map((node) =>
      this.localName === "style" && node.nodeType === 3 ? node.nodeValue : node.outerHTML).join("");
    return `<${this.localName}${attrs}>${content}</${this.localName}>`;
  }
}
class Fragment extends Element {
  constructor() { super("fragment"); this.nodeType = 11; }
  get outerHTML() { return this.childNodes.map((node) => node.outerHTML).join(""); }
}
function reader() {
  const messages = [];
  const fetched = [];
  const root = new Element("html", { clientWidth: 800, scrollHeight: 1800 });
  const head = root.appendChild(new Element("head"));
  const body = root.appendChild(new Element("body", { scrollHeight: 1800 }));
  const document = {
    documentElement: root, head, body, styleSheets: [], baseURI: "http://archive.localhost/article",
    createElement: (tag) => new Element(tag),
    createDocumentFragment: () => new Fragment(),
    createTextNode: textNode,
  };
  const context = vm.createContext({
    document, URL, TextEncoder, Uint8Array, btoa, Node: { ELEMENT_NODE: 1, TEXT_NODE: 3 },
    location: { href: document.baseURI, origin: "http://archive.localhost" }, sessionId: "session-1",
    window: { scrollX: 10, scrollY: 150 },
    post: (type, data) => messages.push({ type, ...data }),
    getComputedStyle(node, pseudo) {
      const values = pseudo ? node.pseudoStyles?.[pseudo] || { content: "none" } : node.styles;
      const names = Object.keys(values);
      return Object.assign({ length: names.length, getPropertyValue: (name) => values[name], content: "none" }, values, names);
    },
    fetch: async (url, options) => {
      assert.equal(new URL(url).origin, "http://archive.localhost");
      assert.equal(options.credentials, "omit");
      assert.equal(options.redirect, "error");
      fetched.push(url);
      return new Response(new Uint8Array([1, 2, 3]), { headers: { "content-type": "image/png" } });
    },
  });
  vm.runInContext(capture, context);
  return { root, head, body, document, context, messages, fetched,
    async capture() {
      await context.exportSnapshot({ requestId: "00000000-0000-4000-8000-000000000001", url: context.location.href });
      return messages.at(-1);
    } };
}

async function audioSnapshot(visible) {
  const frame = reader();
  frame.body.appendChild(new Element("p", { textContent: "Article before audio" }));
  const audio = frame.body.appendChild(new Element("audio", {
    paused: true, currentTime: 12, duration: 125,
    offsetWidth: 300, offsetHeight: 54,
    getClientRects: () => visible ? [{ width: 300, height: 54 }] : [],
  }));
  audio.styles = { display: visible ? "inline" : "none", visibility: "visible", width: "auto", height: "auto" };
  audio.setAttribute("src", "https://uncaptured.invalid/private-recording.mp3");
  audio.setAttribute("autoplay", "");
  audio.setAttribute("onplay", "malicious()");
  audio.setAttribute("aria-label", "Recorded interview");
  const source = audio.appendChild(new Element("source"));
  source.setAttribute("src", "https://uncaptured.invalid/alternative.ogg");
  audio.appendChild(new Element("span", { textContent: "Unrendered audio fallback" }));
  frame.body.appendChild(new Element("p", { textContent: "Article after audio" }));
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.match(result.snapshot.html, /Article before audio/);
  assert.match(result.snapshot.html, /Article after audio/);
  assert.doesNotMatch(result.snapshot.html, /<audio|<source|uncaptured|onplay|autoplay|Unrendered audio fallback/);
  assert.equal(frame.fetched.length, 0, "audio media is never fetched for a static export");
  if (visible) {
    assert.match(result.snapshot.html, /role="img" aria-label="Recorded interview"/);
    assert.match(result.snapshot.html, /▶ 0:12 \/ 2:05/);
    assert.match(result.snapshot.html, /width:300px;height:54px/);
  } else assert.doesNotMatch(result.snapshot.html, /Recorded interview|0:12/);
  return result.snapshot;
}

test("hidden audio is omitted and a visible player exports only an inert current-state representation", async () => {
  await audioSnapshot(false);
  await audioSnapshot(true);
});

// The native consumer test reads these actual production-bridge captures, not
// hand-written HTML that could drift from the producer's output.
if (audioSnapshotsOnly)
  process.stdout.write(JSON.stringify(await Promise.all([audioSnapshot(false), audioSnapshot(true)])));

test("live export captures current form/details/canvas state and self-contained image/computed styling", async () => {
  const frame = reader();
  const input = frame.body.appendChild(new Element("input", { value: "Edited live", checked: true }));
  input.setAttribute("value", "Original");
  input.setAttribute("onclick", "malicious()");
  frame.body.appendChild(new Element("textarea", { value: "Current note" }));
  const details = frame.body.appendChild(new Element("details", { open: true }));
  details.appendChild(new Element("option", { selected: true }));
  frame.body.appendChild(new Element("canvas", { toDataURL: () => "data:image/png;base64,Y2FudmFz", clientWidth: 100, clientHeight: 80 }));
  const image = frame.body.appendChild(new Element("img", { currentSrc: "http://archive.localhost/image.png" }));
  image.styles["background-image"] = "url('/background.png')";
  frame.head.appendChild(new Element("script", { textContent: "malicious()" }));
  const result = await frame.capture();
  assert.equal(result.snapshot.documentUrl, frame.context.location.href);
  assert.equal(result.snapshot.width, 800);
  assert.equal(result.snapshot.height, 1800);
  const html = result.snapshot.html;
  assert.match(html, /value="Edited live"/);
  assert.match(html, /Current note/);
  assert.match(html, /<details[^>]* open=""/);
  assert.match(html, /<option[^>]* selected=""/);
  assert.match(html, /data:image\/png;base64,Y2FudmFz/);
  assert.match(html, /data:image\/png;base64,AQID/);
  assert.doesNotMatch(html, /<script|onclick|malicious\(\)|http:\/\/archive/);
});

test("live export refuses inaccessible external resources and frame documents without original-content fallback", async () => {
  for (const tag of ["iframe", "object", "img"]) {
    const frame = reader();
    frame.body.appendChild(new Element(tag, { currentSrc: "https://external.invalid/image.png" }));
    const result = await frame.capture();
    assert.equal(typeof result.error, "string");
    assert.equal(result.snapshot, undefined);
  }
});

test("live export rejects obsolete request identity and drops results after navigation", async () => {
  const frame = reader();
  await frame.context.exportSnapshot({ requestId: "invalid", url: frame.context.location.href });
  await frame.context.exportSnapshot({ requestId: "00000000-0000-4000-8000-000000000001", url: "http://archive.localhost/old" });
  assert.equal(frame.messages.length, 0);
  frame.body.appendChild(new Element("img", { currentSrc: "http://archive.localhost/image.png" }));
  const originalFetch = frame.context.fetch;
  frame.context.fetch = async (...args) => {
    frame.context.location.href = "http://archive.localhost/new";
    return originalFetch(...args);
  };
  await frame.capture();
  assert.equal(frame.messages.length, 0);
});

test("live export cancels a resource stream before it exceeds the capture budget", async () => {
  const frame = reader();
  frame.body.appendChild(new Element("img", { currentSrc: "http://archive.localhost/huge.png" }));
  let cancelled = false;
  let reads = 0;
  frame.context.fetch = async () => ({ ok: true, headers: { get: () => "image/png" },
    body: { getReader: () => ({
      async read() { reads++; return { done: false, value: new Uint8Array(17 * 1024 * 1024) }; },
      async cancel() { cancelled = true; },
      releaseLock() {},
    }) } });
  const result = await frame.capture();
  assert.equal(cancelled, true);
  assert.equal(reads, 1);
  assert.equal(result.snapshot, undefined);
  assert.equal(typeof result.error, "string");
});

test("long articles retain every paragraph within the unchanged capture budget when computed declarations repeat", async () => {
  const frame = reader();
  const paragraphs = Array.from({ length: 1000 }, (_, index) => `Paragraph ${index}`);
  for (const textContent of paragraphs) {
    const paragraph = frame.body.appendChild(new Element("p", { textContent }));
    paragraph.styles["--archive-state"] = `"${"x".repeat(24 * 1024)}"`;
    paragraph.pseudoStyles = {
      "::before": { content: '"•"', "--archive-state": paragraph.styles["--archive-state"] },
    };
  }
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.deepEqual(result.snapshot.html.match(/Paragraph \d+/g), paragraphs);
  assert.ok(new TextEncoder().encode(result.snapshot.html).length <= 16 * 1024 * 1024);
});

test("unique computed content still cannot bypass the 16 MiB capture limit", async () => {
  const frame = reader();
  const paragraph = frame.body.appendChild(new Element("p", { textContent: "Oversized live state" }));
  paragraph.styles["--archive-state"] = `"${"x".repeat(17 * 1024 * 1024)}"`;
  const result = await frame.capture();
  assert.equal(result.snapshot, undefined);
  assert.equal(typeof result.error, "string");
});

function embedded(frame, parent = frame.body, path = "/nested/player.html") {
  const root = new Element("html", { clientWidth: 320, scrollHeight: 900 });
  const head = root.appendChild(new Element("head"));
  const body = root.appendChild(new Element("body", { scrollHeight: 900 }));
  const location = new URL(path, "http://archive.localhost");
  const document = { documentElement: root, head, body, styleSheets: [], baseURI: location.href,
    location, defaultView: { location, innerWidth: 320, innerHeight: 180, scrollX: 3, scrollY: 40,
      getComputedStyle: frame.context.getComputedStyle } };
  const element = parent.appendChild(new Element("iframe", { contentDocument: document }));
  element.styles = { display: "inline", width: "320px", height: "180px" };
  return { root, head, body, document, element };
}

test("same-origin nested documents are flattened with live state, viewport geometry and local resource bases", async () => {
  const frame = reader();
  const child = embedded(frame);
  child.body.appendChild(new Element("input", { value: "Live child input", checked: true }));
  child.body.appendChild(new Element("img", { currentSrc: "./child-image.png" }));
  child.body.appendChild(new Element("canvas", { toDataURL: () => "data:image/png;base64,Y2hpbGQ=",
    clientWidth: 40, clientHeight: 30 }));
  const grandchild = embedded(frame, child.body, "/nested/deeper/index.html");
  grandchild.body.appendChild(new Element("textarea", { value: "Live grandchild note" }));
  grandchild.body.appendChild(new Element("img", { currentSrc: "./grandchild.png" }));
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  const html = result.snapshot.html;
  assert.match(html, /Live child input/);
  assert.match(html, /Live grandchild note/);
  assert.match(html, /data:image\/png;base64,Y2hpbGQ=/);
  assert.match(html, /overflow:hidden;isolation:isolate;transform:translate\(0px,0px\);width:320px;height:180px/);
  assert.match(html, /left:-3px;top:-40px;width:320px/);
  assert.deepEqual(frame.fetched, [
    "http://archive.localhost/nested/child-image.png",
    "http://archive.localhost/nested/deeper/grandchild.png",
  ]);
  assert.doesNotMatch(html, /<iframe|<frame|<canvas|<video|<object|<embed|srcdoc=/);
  assert.equal((html.match(/<html\b/g) || []).length, 1, "nested documents use valid div containers");
  assert.equal((html.match(/<body\b/g) || []).length, 1);
});

test("nested documents cannot collide with the parent's font faces or local SVG references", async () => {
  const frame = reader();
  const child = embedded(frame);
  function fontSheet(href) {
    const values = { "font-family": '"Shared Font"', src: "url('./font.woff2')" };
    const names = Object.keys(values);
    return { href, cssRules: [{ type: 5,
      style: Object.assign({ length: names.length, getPropertyValue: (name) => values[name] }, names) }] };
  }
  frame.document.styleSheets.push(fontSheet("http://archive.localhost/root/main.css"));
  child.document.styleSheets.push(fontSheet("http://archive.localhost/nested/child.css"));
  for (const body of [frame.body, child.body]) {
    const surface = body.appendChild(new Element("div"));
    surface.setAttribute("id", "shared-clip");
    surface.styles = { "font-family": '"Shared Font", serif', "clip-path": "url('#shared-clip')" };
    const reference = body.appendChild(new Element("use"));
    reference.setAttribute("href", "#shared-clip");
  }
  const result = await frame.capture();
  const html = result.snapshot.html;
  assert.match(html, /kiwix-export-1-0/);
  assert.match(html, /kiwix-export-2-0/);
  assert.match(html, /id=\"kiwix-document-0-shared-clip\"/);
  assert.match(html, /id=\"kiwix-document-1-shared-clip\"/);
  assert.match(html, /href=\"#kiwix-document-0-shared-clip\"/);
  assert.match(html, /href=\"#kiwix-document-1-shared-clip\"/);
  assert.deepEqual(frame.fetched, [
    "http://archive.localhost/root/font.woff2", "http://archive.localhost/nested/font.woff2",
  ]);
});

function unreadableSheet(href) {
  const error = new Error("Failed to read the 'cssRules' property from 'CSSStyleSheet': Cannot access rules");
  error.name = "SecurityError";
  return { href, get cssRules() { throw error; } };
}

test("a same-origin stylesheet the engine refused to load is skipped, not fatal", async () => {
  // Windows WebView2 refuses archive CSS served as application/octet-stream;
  // the sheet then has no readable rules and styles nothing.
  const frame = reader();
  const values = { "font-family": '"Kept Font"', src: "url('./kept.woff2')" };
  const names = Object.keys(values);
  frame.document.styleSheets.push(unreadableSheet("http://archive.localhost/assets/index.css"));
  frame.document.styleSheets.push({ href: "http://archive.localhost/application.css", cssRules: [{ type: 5,
    style: Object.assign({ length: names.length, getPropertyValue: (name) => values[name] }, names) }] });
  frame.body.appendChild(new Element("p", { textContent: "Readable article" }));
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.match(result.snapshot.html, /Readable article/);
  assert.match(result.snapshot.html, /kiwix-export-1-0/, "fonts from readable sheets are still exported");
  assert.deepEqual(frame.fetched, ["http://archive.localhost/kept.woff2"]);
});

test("an unreadable cross-origin or non-security stylesheet still fails the export", async () => {
  for (const sheet of [unreadableSheet("https://external.invalid/style.css"),
    { href: "http://archive.localhost/broken.css", get cssRules() { throw new TypeError("broken"); } }]) {
    const frame = reader();
    frame.document.styleSheets.push(sheet);
    const result = await frame.capture();
    assert.equal(result.snapshot, undefined);
    assert.equal(typeof result.error, "string");
  }
});

test("nested document navigation during asynchronous capture invalidates the entire snapshot", async () => {
  const frame = reader();
  const child = embedded(frame);
  child.body.appendChild(new Element("img", { currentSrc: "./image.png" }));
  const originalFetch = frame.context.fetch;
  frame.context.fetch = async (...args) => {
    child.document.defaultView.location = new URL("http://archive.localhost/replaced");
    return originalFetch(...args);
  };
  const result = await frame.capture();
  assert.equal(result.snapshot, undefined);
  assert.equal(typeof result.error, "string");
});

test("fixed and bottom-anchored elements are captured once at document coordinates, not the export viewport", async () => {
  const frame = reader();
  const toolbar = frame.body.appendChild(new Element("div", {
    offsetWidth: 200, offsetHeight: 50, textContent: "Fixed toolbar",
    getBoundingClientRect: () => ({ left: 100, top: 600, width: 200, height: 50 }),
  }));
  toolbar.styles = { position: "fixed", bottom: "20px", right: "30px", transform: "none" };
  const result = await frame.capture();
  const html = result.snapshot.html;
  assert.match(html, /position:absolute;left:110px;top:750px;right:auto;bottom:auto/);
  assert.match(html, /width:200px;height:50px/);
  assert.equal((html.match(/Fixed toolbar/g) || []).length, 1);
});

test("sticky content keeps its layout slot and is frozen relative to its scrolled containing block", async () => {
  const frame = reader();
  const container = frame.body.appendChild(new Element("section", {
    clientLeft: 2, clientTop: 3, scrollLeft: 5, scrollTop: 100,
    getBoundingClientRect: () => ({ left: 40, top: 20, width: 500, height: 600 }),
  }));
  container.styles = { position: "relative" };
  const sticky = container.appendChild(new Element("div", {
    offsetWidth: 120, offsetHeight: 30, textContent: "Sticky heading",
    getBoundingClientRect: () => ({ left: 50, top: 60, width: 120, height: 30 }),
  }));
  sticky.styles = { position: "sticky", top: "0px", transform: "none" };
  const result = await frame.capture();
  const html = result.snapshot.html;
  assert.match(html, /position:static;inset:auto;transform:none;opacity:0/);
  assert.match(html, /position:absolute;left:13px;top:137px;right:auto;bottom:auto/);
  assert.equal((html.match(/Sticky heading/g) || []).length, 1);
});

test("centered fixed transforms are not applied twice when freezing current geometry", async () => {
  const frame = reader();
  frame.context.window.DOMMatrixReadOnly = class {
    a = 1; b = 0; c = 0; d = 1; e = -100; f = 0;
  };
  const toolbar = frame.body.appendChild(new Element("div", {
    offsetWidth: 200, offsetHeight: 50,
    getBoundingClientRect: () => ({ left: 200, top: 600, width: 200, height: 50 }),
  }));
  toolbar.styles = { position: "fixed", transform: "matrix(1, 0, 0, 1, -100, 0)", transformOrigin: "100px 25px" };
  const result = await frame.capture();
  assert.match(result.snapshot.html, /left:310px;top:750px/);
  assert.match(result.snapshot.html, /transform:matrix\(1, 0, 0, 1, -100, 0\)/);
});

test("fixed content inside a child document freezes in child scroll coordinates before viewport clipping", async () => {
  const frame = reader();
  const child = embedded(frame);
  const toolbar = child.body.appendChild(new Element("div", {
    offsetWidth: 50, offsetHeight: 20, textContent: "Child fixed control",
    getBoundingClientRect: () => ({ left: 10, top: 140, width: 50, height: 20 }),
  }));
  toolbar.styles = { position: "fixed", bottom: "20px", transform: "none" };
  const result = await frame.capture();
  assert.match(result.snapshot.html, /left:13px;top:180px;right:auto;bottom:auto/);
  assert.match(result.snapshot.html, /top:-40px/);
  assert.equal((result.snapshot.html.match(/Child fixed control/g) || []).length, 1);
});

test("frozen viewport content retains local coordinates inside scaled containing blocks", async () => {
  const frame = reader();
  const container = frame.body.appendChild(new Element("section", {
    offsetWidth: 250, offsetHeight: 300, clientLeft: 2, clientTop: 3,
    scrollLeft: 5, scrollTop: 100,
    getBoundingClientRect: () => ({ left: 40, top: 20, width: 500, height: 600 }),
  }));
  container.styles = { position: "relative", transform: "scale(2)" };
  const sticky = container.appendChild(new Element("div", {
    offsetWidth: 120, offsetHeight: 30,
    getBoundingClientRect: () => ({ left: 140, top: 100, width: 240, height: 60 }),
  }));
  sticky.styles = { position: "sticky", transform: "none" };
  const result = await frame.capture();
  assert.match(result.snapshot.html, /position:absolute;left:53px;top:137px;right:auto;bottom:auto/);
  assert.match(result.snapshot.html, /width:120px;height:30px/);
});

function shadowHost(frame) {
  const host = frame.body.appendChild(new Element("archive-player"));
  const shadow = new Fragment();
  shadow.styleSheets = [];
  shadow.adoptedStyleSheets = [];
  shadow.host = host;
  host.shadowRoot = shadow;
  return { host, shadow };
}

test("open shadow DOM exports its composed live content, assigned slots and fallback slots, not unslotted light DOM", async () => {
  const frame = reader();
  const { host, shadow } = shadowHost(frame);
  const selected = host.appendChild(new Element("input", { value: "Assigned live field", checked: true }));
  host.appendChild(new Element("img", { currentSrc: "https://unrendered.invalid/unslotted.png" }));
  shadow.appendChild(new Element("canvas", { toDataURL: () => "data:image/png;base64,c2hhZG93",
    clientWidth: 80, clientHeight: 60 }));
  const slot = shadow.appendChild(new Element("slot", { assignedNodes: () => [selected] }));
  slot.styles = { display: "contents" };
  slot.appendChild(new Element("div", { textContent: "Not rendered slot fallback" }));
  const fallback = shadow.appendChild(new Element("slot", { assignedNodes: () => [] }));
  fallback.styles = { display: "contents" };
  fallback.appendChild(new Element("div", { textContent: "Rendered slot fallback" }));
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.match(result.snapshot.html, /Assigned live field/);
  assert.match(result.snapshot.html, /data:image\/png;base64,c2hhZG93/);
  assert.match(result.snapshot.html, /Rendered slot fallback/);
  assert.doesNotMatch(result.snapshot.html, /Not rendered slot fallback|unslotted|<archive-player|<slot|<canvas/);
  assert.equal((result.snapshot.html.match(/Assigned live field/g) || []).length, 1);
  assert.equal(frame.fetched.length, 0);
});

test("open shadow adopted font sheets are self-contained and scoped separately from document fonts", async () => {
  const frame = reader();
  const { shadow } = shadowHost(frame);
  const values = { "font-family": '"Shadow Font"', src: "url('./shadow.woff2')" };
  const names = Object.keys(values);
  shadow.adoptedStyleSheets.push({ href: null, cssRules: [{ type: 5,
    style: Object.assign({ length: names.length, getPropertyValue: (name) => values[name] }, names) }] });
  const text = shadow.appendChild(new Element("div", { textContent: "Shadow type" }));
  text.styles = { "font-family": '"Shadow Font"' };
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.match(result.snapshot.html, /kiwix-export-2-0/);
  assert.match(result.snapshot.html, /data:image\/png;base64,AQID/);
  assert.deepEqual(frame.fetched, ["http://archive.localhost/shadow.woff2"]);
});

test("custom surfaces without readable shadow DOM fail explicitly rather than claiming empty live-page completeness", async () => {
  const frame = reader();
  const host = frame.body.appendChild(new Element("archive-player"));
  host.appendChild(new Element("div", { textContent: "Possibly unslotted content" }));
  const result = await frame.capture();
  assert.equal(result.snapshot, undefined);
  assert.equal(typeof result.error, "string");
});

test("a never-defined, empty, unrendered custom element is left out instead of failing the export", async () => {
  // DevDocs archives carry an empty <devdocs-navbar> whose script may never
  // register it; Chromium gives it an empty 0x0 inline box.
  const state = (defined) => ({ matches: (selector) => selector === ":defined" ? defined : false });
  const zero = () => ({ width: 0, height: 0 });
  const inline = { display: "inline", "background-image": "none" };
  const frame = reader();
  frame.body.appendChild(new Element("p", { textContent: "Article text" }));
  frame.body.appendChild(new Element("devdocs-navbar", {
    ...state(false), getBoundingClientRect: zero, styles: inline,
  }));
  const result = await frame.capture();
  assert.equal(result.error, undefined);
  assert.match(result.snapshot.html, /Article text/);
  assert.doesNotMatch(result.snapshot.html, /devdocs-navbar/);

  // Still fails closed when the element is defined (it may hold a closed
  // shadow root), has light DOM, renders a box in either dimension, is not
  // inline (a sized block or display:contents host can render shadow content
  // from a 0x0 box), or its defined state cannot be read.
  for (const properties of [
    { ...state(true), getBoundingClientRect: zero, styles: inline },
    { ...state(false), getBoundingClientRect: () => ({ width: 120, height: 40 }), styles: inline },
    { ...state(false), getBoundingClientRect: () => ({ width: 0, height: 21 }), styles: inline },
    { ...state(false), getBoundingClientRect: zero, styles: { ...inline, display: "contents" } },
    { ...state(false), getBoundingClientRect: zero, styles: { ...inline, display: "block" } },
    { getBoundingClientRect: zero, styles: inline },
  ]) {
    const refused = reader();
    refused.body.appendChild(new Element("archive-player", properties));
    const outcome = await refused.capture();
    assert.equal(outcome.snapshot, undefined);
    assert.equal(typeof outcome.error, "string");
  }
  const withChildren = reader();
  const host = withChildren.body.appendChild(new Element("archive-player", {
    ...state(false), getBoundingClientRect: zero, styles: inline,
  }));
  host.appendChild(new Element("div", { textContent: "Possibly hidden" }));
  const outcome = await withChildren.capture();
  assert.equal(outcome.snapshot, undefined);
  assert.equal(typeof outcome.error, "string");
});

test("replaced shadow roots invalidate a capture rather than mixing rendered surface identities", async () => {
  const frame = reader();
  const { host, shadow } = shadowHost(frame);
  shadow.appendChild(new Element("img", { currentSrc: "./shadow-image.png" }));
  const originalFetch = frame.context.fetch;
  frame.context.fetch = async (...args) => {
    host.shadowRoot = new Fragment();
    return originalFetch(...args);
  };
  const result = await frame.capture();
  assert.equal(result.snapshot, undefined);
  assert.equal(typeof result.error, "string");
});
