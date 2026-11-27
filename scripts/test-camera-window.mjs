import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

// Run the shipped capture page script against a minimal window. Only the
// camera, the canvas and the app's IPC are simulated.
const path = "../src/camera.ts";
const code = ts.transpileModule(await readFile(new URL(path, import.meta.url), "utf8"), {
  fileName: path,
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
const turn = () => new Promise((resolve) => setImmediate(resolve));
async function settle() { for (let index = 0; index < 5; index++) await turn(); }

function camera({ invoke, locale = "resolve", query = "?token=session-1&locale=fr" } = {}) {
  const state = { calls: [], closed: 0, cameraStarts: 0, focused: null, warnings: [], listeners: new Map() };
  const element = (id) => ({
    id, hidden: false, disabled: id === "take", textContent: "", srcObject: null, videoWidth: 0, videoHeight: 0,
    classList: { toggle() {} }, attributes: {},
    setAttribute(name, value) { this.attributes[name] = value; },
    addEventListener(type, handler) { state.listeners.set(`${id}:${type}`, handler); },
    focus() { state.focused = id; },
    async play() {},
  });
  const elements = Object.fromEntries(["preview", "message", "take", "cancel", "switch"].map((id) => [id, element(id)]));
  const track = { stop() {}, getSettings: () => ({ deviceId: "camera-1", facingMode: "environment" }) };
  const context = vm.createContext({
    exports: {},
    require: (name) => ({
      "@tauri-apps/api/core": {
        invoke: async (command, payload) => {
          assert.equal(command, "dispatch");
          state.calls.push(payload.command);
          return (invoke ?? (async () => null))(payload.command, payload.args);
        },
      },
      "./i18n": {
        setLocale: (value) => { if (locale === "unsupported") throw new RangeError(`Unsupported interface locale: ${value}`); },
        whenLocaleReady: () => locale === "chunk" ? Promise.reject(new Error("Missing offline locale chunk: fr")) : Promise.resolve(),
        localeDirection: () => "ltr",
        t: (value) => value,
      },
    })[name] ?? assert.fail(`Unexpected import: ${name}`),
    location: { search: query },
    document: {
      documentElement: { lang: "en", dir: "ltr" }, title: "",
      getElementById: (id) => elements[id],
      addEventListener(type, handler) { state.listeners.set(`document:${type}`, handler); },
      createElement: () => ({
        width: 0, height: 0,
        getContext: () => ({ drawImage() {} }),
        toBlob: (done, type) => done(new Blob([Uint8Array.of(0xff, 0xd8, 0xff)], { type })),
      }),
    },
    window: { addEventListener() {}, close() { state.closed++; } },
    navigator: { mediaDevices: {
      async getUserMedia() {
        state.cameraStarts++;
        elements.preview.videoWidth = 640;
        elements.preview.videoHeight = 480;
        return { getTracks: () => [track], getVideoTracks: () => [track] };
      },
      async enumerateDevices() { return [{ kind: "videoinput", deviceId: "camera-1" }]; },
    } },
    console: { warn: (...parts) => state.warnings.push(parts.map(String).join(" ")) },
    URLSearchParams, DOMException, Blob, Uint8Array, Promise, Math, String, btoa,
  });
  vm.runInContext(code, context);
  return {
    state, elements,
    click: (id) => state.listeners.get(`${id}:click`)(),
    key: (key) => state.listeners.get("document:keydown")({ key }),
  };
}

test("a rejected capture leaves a message and a working Cancel instead of a dead window", async () => {
  for (const dismiss of ["cancel", "escape"]) {
    const page = camera({ invoke: async () => { throw "This camera session has ended"; } });
    await settle();
    assert.equal(page.state.cameraStarts, 1);
    assert.equal(page.elements.take.disabled, false);
    page.click("take");
    await settle();
    assert.deepEqual(page.state.calls, ["camera_capture_complete"]);
    assert.equal(page.elements.message.textContent, "The photo could not be sent. Cancel and choose a file instead.");
    assert.equal(page.elements.message.hidden, false);
    assert.equal(page.state.focused, "cancel", "keyboard focus moves to the control that still works");
    assert.match(page.state.warnings.join("\n"), /This camera session has ended/, "the cause is kept for diagnosis");
    // The app no longer answers for this session, so Cancel closes the window itself.
    if (dismiss === "cancel") page.click("cancel"); else page.key("Escape");
    await settle();
    assert.deepEqual(page.state.calls, ["camera_capture_complete", "camera_capture_cancel"]);
    assert.equal(page.state.closed, 1);
  }
});

test("Cancel after a rejected capture reaches the app when the session is still live", async () => {
  const page = camera({ invoke: async (command) => { if (command === "camera_capture_complete") throw new Error("Not available"); return null; } });
  await settle();
  page.click("take");
  await settle();
  page.click("cancel");
  await settle();
  assert.deepEqual(page.state.calls, ["camera_capture_complete", "camera_capture_cancel"]);
  assert.equal(page.state.closed, 0, "the app closes the window after it accepted the cancellation");
  // The session is finished now: a second Cancel is not sent again.
  page.click("cancel");
  await settle();
  assert.equal(page.state.calls.length, 2);
});

test("an accepted capture is sent once and the page waits for the app to close it", async () => {
  const page = camera();
  await settle();
  page.click("take");
  await settle();
  page.click("take");
  page.click("cancel");
  await settle();
  assert.deepEqual(page.state.calls, ["camera_capture_complete"]);
  assert.equal(page.elements.message.textContent, "Sending the photo…");
  assert.equal(page.state.closed, 0);
});

test("the camera starts in English when its locale cannot be loaded or is not supported", async () => {
  for (const locale of ["chunk", "unsupported"]) {
    const page = camera({ locale });
    await settle();
    assert.equal(page.state.cameraStarts, 1, `${locale}: the camera still starts`);
    assert.equal(page.elements.take.disabled, false);
    assert.equal(page.elements.take.textContent, "Take photo");
    assert.equal(page.elements.cancel.textContent, "Cancel");
    assert.equal(page.state.warnings.length, 1);
  }
  const localized = camera();
  await settle();
  assert.equal(localized.state.cameraStarts, 1);
  assert.deepEqual(localized.state.warnings, []);
});
