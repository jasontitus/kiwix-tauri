import assert from "node:assert/strict";
import test from "node:test";
import { readFile, readdir } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  CANCELLED_CODE,
  ERROR_SITES,
  SUSPENDED_CODE,
  SuspendedError,
  UserCancelledError,
  errorSurface,
  isSuspendedError,
  isUserCancelledError,
  normalizeSeverity,
  parseSuspendedError,
  shouldRenderError,
} from "../src/error-visibility.ts";
import { openDialogs } from "../src/shell-dismiss.ts";
import {
  DIAGNOSTIC_LIMIT,
  DiagnosticLog,
  reportError,
} from "../src/diagnostics.ts";

test("only the sensor decision write is blocking; the rest stay recoverable", () => {
  for (const [site, severity] of Object.entries(ERROR_SITES)) {
    assert.equal(
      severity,
      site === "reader.sensor-revoke" ? "blocking" : "recoverable",
      site,
    );
  }
});

test("the reporting path defaults to blocking", () => {
  assert.equal(normalizeSeverity(undefined), "blocking");
  assert.equal(normalizeSeverity("blocking"), "blocking");
  assert.equal(normalizeSeverity("recoverable"), "recoverable");
});

test("recoverable errors render only when technical details are on", () => {
  assert.equal(shouldRenderError("recoverable", false), false);
  assert.equal(shouldRenderError("recoverable", true), true);
});

test("blocking errors render regardless of the setting", () => {
  assert.equal(shouldRenderError("blocking", false), true);
  assert.equal(shouldRenderError("blocking", true), true);
  assert.equal(shouldRenderError(undefined, false), true);
});

test("a recoverable error is recorded but not rendered with the setting off", () => {
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new Error("A stale article link was not found"),
    "recoverable",
    false,
  );
  assert.equal(render, false);
  assert.equal(log.snapshot().length, 1);
  assert.equal(log.snapshot()[0].severity, "recoverable");
  assert.match(log.snapshot()[0].message, /stale article link/);
});

test("a blocking error is recorded and rendered with the setting off", () => {
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new Error("The archive could not be opened"),
    "blocking",
    false,
  );
  assert.equal(render, true);
  assert.equal(log.snapshot().length, 1);
  assert.equal(log.snapshot()[0].severity, "blocking");
});

test("the in-memory log keeps only the newest bounded entries", () => {
  const log = new DiagnosticLog(3);
  for (let index = 0; index < 5; index++)
    log.record(new Error(`failure ${index}`), "recoverable");
  assert.deepEqual(
    log.snapshot().map((entry) => entry.message),
    ["failure 2", "failure 3", "failure 4"],
  );
  assert.equal(DIAGNOSTIC_LIMIT, 50);
});

test("clearing the log notifies subscribers and empties the copy text", () => {
  const log = new DiagnosticLog();
  let notifications = 0;
  const unsubscribe = log.subscribe(() => notifications++);
  log.record(new Error("recorded"), "recoverable");
  log.clear();
  unsubscribe();
  assert.equal(notifications, 2);
  assert.deepEqual(log.snapshot(), []);
  assert.equal(log.text(), "[]");
});

test("the Rust lifecycle gate becomes a typed error, never a text match", () => {
  const signal = JSON.stringify({
    code: SUSPENDED_CODE,
    message: "The application is not interactive",
  });
  const error = parseSuspendedError(signal);
  assert.ok(error instanceof SuspendedError);
  assert.equal(error.code, SUSPENDED_CODE);
  assert.equal(isSuspendedError(error), true);
  assert.equal(isSuspendedError(new Error(signal)), false);
  assert.equal(
    parseSuspendedError("The application is not interactive"),
    null,
  );
  assert.equal(parseSuspendedError('{"code":"OTHER","message":"no"}'), null);
});

test("a suspended command is neither recorded nor rendered", () => {
  assert.equal(shouldRenderError("suspended", false), false);
  assert.equal(shouldRenderError("suspended", true), false);
  const log = new DiagnosticLog();
  const render = reportError(
    log,
    new SuspendedError("The application is not interactive"),
    "blocking",
    true,
  );
  assert.equal(render, false);
  assert.deepEqual(log.snapshot(), []);
});

// `errorText` with the module-level patterns and helpers it uses, as shipped.
const ERROR_TEXT_PARTS = [
  "filesystemRoot", "quotedFilesystemPath", "bareFilesystemPath", "NATIVE_CODE", "nativeCodeOnly",
  "nativeCodeForms", "nearbyCodeFamily", "senderVerificationFailure", "nearbyCodeText", "nativeCodeText", "errorText",
];
function errorTextSource(tree) {
  const found = new Map();
  for (const statement of tree.statements) {
    const names = ts.isFunctionDeclaration(statement) ? [statement.name?.text]
      : ts.isVariableStatement(statement) ? statement.declarationList.declarations.map((item) => item.name.getText(tree))
        : [];
    for (const name of names)
      if (ERROR_TEXT_PARTS.includes(name)) found.set(name, statement.getText(tree).replace(/^export /, ""));
  }
  for (const name of ERROR_TEXT_PARTS) assert.ok(found.has(name), `Missing errorText part: ${name}`);
  return [...new Set(found.values())].join("\n");
}
const interpolate = (source, values = {}) => source.replace(/\{(\w+)\}/g, (whole, key) => key in values ? String(values[key]) : whole);
async function loadErrorText(overrides = {}) {
  const path = "../src/components/UI.tsx";
  const tree = ts.createSourceFile(path, await readFile(new URL(path, import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const context = vm.createContext({ Error, isSuspendedError, isUserCancelledError, isIOS: false, t: interpolate, ...overrides });
  vm.runInContext(ts.transpileModule(`${errorTextSource(tree)}\nglobalThis.errorText = errorText;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, context);
  return context.errorText;
}
async function sourceFiles(directory, extension, skip = () => false) {
  const root = new URL(directory, import.meta.url);
  const files = [];
  for (const entry of await readdir(root, { recursive: true, withFileTypes: true })) {
    const path = `${entry.parentPath ?? entry.path}/${entry.name}`;
    if (entry.isFile() && entry.name.endsWith(extension) && !/\/(?:build|target|\.gradle)\//.test(path) && !skip(path))
      files.push({ path, text: await readFile(path, "utf8") });
  }
  assert.ok(files.length, `No ${extension} sources under ${directory}`);
  return files;
}
const SENDER_SENTENCE = "The sender's archive failed verification. Stop sharing and start again, or download it again from the catalog.";
const VERIFICATION_WORDS = /checksum|corrupt|source changed|integrity|verification/i;
// The receiver was given bytes that do not match the sender's manifest, or the
// sender's own shared archive changed or failed its check.
const SENDER_VERIFICATION = [
  "LocalSwarm chunk checksum mismatch",
  "LocalSwarm source changed or returned corrupt bytes",
  "Verified LocalSwarm source changed",
  "LocalSwarm source parts must exactly cover the ZIM checksum boundary",
];
// Nearby messages that use the same words for something that is not the
// sender's archive: a worker that stopped, this device's own staged or
// published files, a cancelled check. They keep their own text.
const NEARBY_LOCAL = [
  "LocalSwarm chunk verification stopped",
  "Seal LocalSwarm writers before native verification",
  "LocalSwarm verification handle no longer identifies the staged bytes",
  "LocalSwarm staged bytes changed after chunk verification",
  "Starting LocalSwarm verification: {e}",
  "LocalSwarm verification stopped: {e}",
  "Archive publication verification was cancelled",
  "Nearby publication changed during verification; retry the import",
];
const rustLiterals = (text) => [...text.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)].map((match) => match[1]);
// Unit tests quote the same messages; only shipped code decides the contract.
const shippedRust = (text) => text.split(/^#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))? )?mod \w+/m)[0];

test("only Nearby sender verification failures become the sender sentence", async () => {
  const errorText = await loadErrorText();
  for (const message of SENDER_VERIFICATION) {
    assert.equal(errorText(message), SENDER_SENTENCE, message);
    assert.equal(errorText(new Error(message)), SENDER_SENTENCE, message);
    assert.equal(errorText(`LocalSwarm source reconnect limit reached after 3 attempts; retained verified data can be resumed (last error: ${message})`),
      SENDER_SENTENCE, "a wrapped sender failure is still a sender failure");
  }
  // Real messages from downloads, imports, integrity checks, maps, voices and
  // library moves that the unscoped pattern used to replace.
  for (const message of [
    "Verification was interrupted by application shutdown; resume when ready",
    "Reading download checksum: Input/output error (os error 5)",
    "Download ownership changed during verification; retry the import",
    "Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download",
    "An archive integrity check is already running",
    "A selected archive failed verification; no files were moved",
    "Catalog source changed; select this edition again",
    "Edition intent changed during verification",
    "Archive changed during verification",
    "Saved map prefix failed its checksum; delete the retained download or get the map again. No bytes were modified",
    "Map checkpoint moved or changed identity during verification; no bytes were modified",
    "Map file length changed during ownership verification",
    "Archive verification queue is full",
    "Archive verification was cancelled",
    "Archive changed after verification; retry the import",
    "Malformed archive: Zim file(s) is of bad size or corrupted",
    "Map import failed: Saved map prefix failed its checksum; delete the retained download or get the map again. No bytes were modified",
    ...NEARBY_LOCAL,
  ]) {
    assert.equal(errorText(message), message, message);
    assert.equal(errorText(new Error(message)), message, message);
  }
});

test("the sender-verification list covers every Nearby producer, and no other Rust message is rewritten", async () => {
  const errorText = await loadErrorText();
  const nearby = (path) => /\/localswarm\/|\/nearby[^/]*$/.test(path);
  const tests = (path) => /(?:^|[/_])tests?\.rs$|_tests\.rs$|\/tests\//.test(path);
  const rust = await sourceFiles("../src-tauri/src", ".rs", tests);
  const classified = new Set([...SENDER_VERIFICATION, ...NEARBY_LOCAL]);
  const produced = new Set();
  let others = 0;
  for (const { path, text } of rust) {
    for (const literal of rustLiterals(shippedRust(text))) {
      // A sentence, not a thread name, file name or log tag.
      if (!VERIFICATION_WORDS.test(literal) || !/^[A-Z][^"]* [^"]*$/.test(literal)) continue;
      if (nearby(path)) {
        produced.add(literal);
        assert.ok(classified.has(literal),
          `${path}: classify this Nearby message as a sender verification failure or a local one: ${literal}`);
      } else if (!classified.has(literal)) {
        others++;
        assert.equal(errorText(literal), literal, `${path}: not a Nearby sender failure, so its own text stays`);
      }
    }
  }
  for (const message of SENDER_VERIFICATION)
    assert.ok(produced.has(message), `No Nearby source produces this any more; remove it from errorText: ${message}`);
  assert.ok(others >= 15, `expected the non-Nearby verification messages to be scanned, found ${others}`);
});

const NEARBY_CODE = /"((?:NEARBY|BLE|JOIN|HOTSPOT|LOCAL_NETWORK|MOBILE_NETWORK|MOBILE_LOCAL_NETWORK|VISIBLE_ACTIVITY)_[A-Z0-9_]+)"/g;
const ANY_CODE = /"([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)"/g;
async function nativeCodes() {
  const plugin = "../src-tauri/plugins/tauri-plugin-kiwix-mobile";
  const kotlin = await sourceFiles(`${plugin}/android/src/main`, ".kt");
  const swift = await sourceFiles(`${plugin}/ios/Sources`, ".swift");
  const rust = [...await sourceFiles("../src-tauri/src", ".rs"), ...await sourceFiles(`${plugin}/src`, ".rs")];
  const collect = (files, pattern) => new Set(files.flatMap(({ text }) => [...text.matchAll(pattern)].map((match) => match[1])));
  return {
    nearby: collect([...kotlin, ...swift, ...rust], NEARBY_CODE),
    // Every code-shaped literal of the two native bridges, whatever its family.
    native: collect([...kotlin, ...swift], ANY_CODE),
    perPlatform: [collect(kotlin, NEARBY_CODE), collect(swift, NEARBY_CODE), collect(rust, NEARBY_CODE)],
  };
}
const identifierOnly = /^\[?[A-Z][A-Z0-9_]*\]?(?:\s+-\s+[A-Z][A-Z0-9_]*)?$/;

test("no native status code produced by Kotlin, Swift or Rust is ever shown as the message", async () => {
  const errorText = await loadErrorText();
  const { nearby, native, perPlatform } = await nativeCodes();
  // Guard the collection itself: an empty or moved source tree must not pass.
  assert.ok(perPlatform[0].size >= 60, `Kotlin codes found: ${perPlatform[0].size}`);
  assert.ok(perPlatform[1].size >= 40, `Swift codes found: ${perPlatform[1].size}`);
  assert.ok(perPlatform[2].size >= 5, `Rust codes found: ${perPlatform[2].size}`);
  for (const code of ["BLE_POWERED_OFF", "NEARBY_JOIN_TIMEOUT", "HOTSPOT_TETHERING_DISALLOWED", "MOBILE_LOCAL_NETWORK_BLOCKED", "NEARBY_LAN_REQUIRED"])
    assert.ok(nearby.has(code), `expected ${code} among the produced codes`);
  const sentence = (code, raw, label) => {
    const shown = errorText(raw);
    assert.notEqual(shown, raw, `${label} ${code} is shown unchanged`);
    assert.ok(!identifierOnly.test(shown), `${label} ${code} is shown as an identifier: ${shown}`);
    assert.match(shown, /[a-z]{3,} [a-z]{2,}/, `${label} ${code} has no sentence: ${shown}`);
    assert.equal(errorText(shown), shown, `${label} ${code}: formatting the shown text again must not change it`);
    return shown;
  };
  for (const code of nearby) {
    const bare = sentence(code, code, "bare");
    // Tauri prints an Android plugin rejection as "[code] - message".
    assert.equal(sentence(code, `[${code}] - ${code}`, "wrapped"), bare);
    assert.equal(sentence(code, `[${code}]`, "code-only"), bare);
    assert.equal(sentence(code, new Error(`  ${code}\n`), "padded"), bare);
    // Rust sentences that carry the native code as their tail.
    for (const prefix of ["Creating the Wi-Fi hotspot failed:", "The sender could not join the temporary Wi-Fi network:"]) {
      assert.equal(sentence(code, `${prefix} ${code}`, "prefixed"), `${prefix} ${bare}`);
      assert.equal(sentence(code, `${prefix} [${code}] - ${code}`, "prefixed and wrapped"), `${prefix} ${bare}`);
    }
  }
  for (const code of native) {
    sentence(code, code, "bare");
    sentence(code, `[${code}] - ${code}`, "wrapped");
  }
});

test("native code families map to their instruction in every form they arrive in", async () => {
  const errorText = await loadErrorText();
  const bluetoothOff = "Bluetooth is off. Turn on Bluetooth, then try again.";
  const bluetoothDenied = "Allow Bluetooth access for Kiwix in the device settings, then try again.";
  const bluetoothUnsupported = "This device cannot share over Bluetooth. Use the same local network instead.";
  const localNetwork = "The local network connection was lost or is not available. Connect both devices to the same Wi-Fi, then try again.";
  const noTemporaryWifi = "This device cannot create or join a temporary Wi-Fi network. Use the same Wi-Fi network.";
  const joinLost = "The temporary Wi-Fi connection ended or changed. Keep both apps open and try again.";
  const joinCancelled = "The temporary Wi-Fi could not start or was cancelled. Try again.";
  const joinTimedOut = "The temporary Wi-Fi is still starting. Wait a moment and try again.";
  for (const [raw, expected] of [
    ["BLE_POWERED_OFF", bluetoothOff],
    ["[NEARBY_BLUETOOTH_UNAVAILABLE] - NEARBY_BLUETOOTH_UNAVAILABLE", bluetoothOff],
    ["BLE_PERMISSION_DENIED", bluetoothDenied],
    ["BLE_UNSUPPORTED", bluetoothUnsupported],
    ["[NEARBY_BLUETOOTH_UNSUPPORTED] - NEARBY_BLUETOOTH_UNSUPPORTED", bluetoothUnsupported],
    ["MOBILE_LOCAL_NETWORK_BLOCKED", localNetwork],
    ["LOCAL_NETWORK_CHANGED", localNetwork],
    ["[NEARBY_ROUTE_LOST] - NEARBY_ROUTE_LOST", localNetwork],
    ["Creating the Wi-Fi hotspot failed: [Hotspot use is blocked by device policy] - HOTSPOT_TETHERING_DISALLOWED",
      `Creating the Wi-Fi hotspot failed: ${noTemporaryWifi}`],
    ["[HOTSPOT_TETHERING_DISALLOWED] - Hotspot use is blocked by device policy", noTemporaryWifi],
    ["[NEARBY_JOIN_LOST] - NEARBY_JOIN_LOST", joinLost],
    ["JOIN_ROUTE_CHANGED", joinLost],
    ["[NEARBY_JOIN_CANCELLED] - NEARBY_JOIN_CANCELLED", joinCancelled],
    ["JOIN_CANCELLED", joinCancelled],
    ["[NEARBY_JOIN_TIMEOUT] - NEARBY_JOIN_TIMEOUT", joinTimedOut],
    ["JOIN_ASSOCIATION_QUERY_TIMEOUT", joinTimedOut],
    ["The sender could not join the temporary Wi-Fi network: JOIN_USER_DENIED",
      "The sender could not join the temporary Wi-Fi network: Allow Wi-Fi and location access for Nearby, then try again."],
    // An unmapped Nearby code keeps its identifier for a support request, inside a sentence.
    ["NEARBY_ROUTE_LIMIT", "Nearby could not connect. Try again. (NEARBY_ROUTE_LIMIT)"],
    ["[NEARBY_ROUTE_LIMIT] - NEARBY_ROUTE_LIMIT", "Nearby could not connect. Try again. (NEARBY_ROUTE_LIMIT)"],
    // A code of any other family is still never the whole message.
    ["DOCUMENT_PERMISSION_DENIED", "That action did not finish. Try again. (DOCUMENT_PERMISSION_DENIED)"],
    ["[EXPORT_DURABILITY_ERROR] - EXPORT_DURABILITY_ERROR", "That action did not finish. Try again. (EXPORT_DURABILITY_ERROR)"],
  ]) assert.equal(errorText(raw), expected, raw);
});

test("a sentence is never mistaken for a native code", async () => {
  const errorText = await loadErrorText();
  for (const message of [
    "Download failed: HTTP 404",
    "Could not open README",
    "The ZIM file is damaged",
    "KIWIX is not running",
    "Unsupported URL scheme: FTP",
    // Only the Nearby families are known to be codes after a sentence.
    "Unknown archive name: WIKIPEDIA_EN",
    "Could not open the archive TED_TALKS today",
    // The native side already wrote a sentence next to an unmapped code.
    "[NEARBY_FUTURE_STATE] - Wi-Fi is busy with another mode",
    "[Wi-Fi is busy with another mode] - NEARBY_FUTURE_STATE",
    "[DOCUMENT_PROVIDER_ERROR] - The storage provider did not answer",
    "First line: NEARBY_JOIN_TIMEOUT\nSecond line",
    "/storage/emulated/0/wiki.zim: NEARBY_JOIN_TIMEOUT\n/storage/emulated/0/other.zim: unreadable",
  ]) assert.equal(errorText(message), message, message);
});

test("a cancellation the user chose is typed and never painted as a failure", async () => {
  const cancelled = new UserCancelledError("Download cancelled.");
  assert.equal(cancelled.code, CANCELLED_CODE);
  assert.equal(isUserCancelledError(cancelled), true);
  assert.equal(isUserCancelledError(new Error("Download cancelled.")), false, "the sentence alone is not the type");
  assert.equal(isUserCancelledError(new SuspendedError("suspended")), false);
  assert.equal(isSuspendedError(cancelled), false, "a cancellation is never retried as a lifecycle gate");
  const errorText = await loadErrorText();
  assert.equal(errorText(cancelled), "", "no inline owner has text to paint");
  assert.equal(errorText(new Error("Download cancelled.")), "Download cancelled.",
    "a refusal the user did not choose stays an ordinary visible error");
});

test("only the user's own answer to the low-storage prompt is a cancellation", async () => {
  const path = "../src/api.ts";
  const tree = ts.createSourceFile(path, await readFile(new URL(path, import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const wanted = ["diskSpaceConfirmation", "registerDiskSpaceConfirmation", "decideDiskSpaceWarning", "confirmDiskSpaceWarning", "request"];
  const parts = tree.statements.filter((statement) =>
    (ts.isFunctionDeclaration(statement) && wanted.includes(statement.name?.text)) ||
    (ts.isVariableStatement(statement) && statement.declarationList.declarations.some((item) => wanted.includes(item.name.getText(tree)))))
    .map((statement) => statement.getText(tree).replace(/^export /, ""));
  assert.equal(parts.length, wanted.length);
  const warning = { acknowledgement: "ack-1", directory: "/downloads", availableBytes: 10, remainingBytes: 5 };
  const start = async ({ decision, register = true, privacy = { generation: 4, privateMode: false, readOnly: false }, current = true }) => {
    const sent = [];
    const context = vm.createContext({
      Error, UserCancelledError, structuredClone, t: interpolate, localSwarmStopGeneration: 0,
      checkLocalSwarmAdmission() {},
      capturePrivacyContext: () => privacy,
      isPrivacyContextCurrent: () => current,
      parseDiskSpaceWarning: (error) => error?.diskSpace ? warning : null,
      requestOnce: async (command, args) => {
        sent.push({ command, args: { ...args } });
        if (!args.diskSpaceAcknowledgement) throw Object.assign(new Error("low storage"), { diskSpace: true });
        return { id: "job-1" };
      },
    });
    vm.runInContext(ts.transpileModule(`${parts.join("\n")}
      globalThis.register = registerDiskSpaceConfirmation; globalThis.request = request;`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText, context);
    let prompts = 0;
    if (register) context.register(async () => { prompts++; return decision; });
    let result, failure;
    try { result = await context.request("start_download", { url: "https://example.org/a.zim" }); }
    catch (error) { failure = error; }
    return { result, failure, sent, prompts };
  };
  const accepted = await start({ decision: "accepted" });
  assert.deepEqual({ ...accepted.result }, { id: "job-1" });
  assert.equal(accepted.sent.length, 2);
  assert.equal(accepted.sent[1].args.diskSpaceAcknowledgement, "ack-1");

  const declined = await start({ decision: "declined" });
  assert.ok(isUserCancelledError(declined.failure), "Cancel on the prompt is the user's choice");
  assert.equal(declined.sent.length, 1, "nothing is retried without consent");
  assert.equal(declined.prompts, 1);

  // Not shown, or withdrawn by the lifecycle: the download did not start and
  // the user did not choose that, so it stays an ordinary visible error.
  for (const [name, options] of [
    ["withdrawn by the lifecycle", { decision: "unavailable" }],
    ["no prompt surface is mounted", { decision: "accepted", register: false }],
    ["read-only", { decision: "accepted", privacy: { generation: 4, privateMode: false, readOnly: true } }],
    ["consent under an expired privacy generation", { decision: "accepted", current: false }],
  ]) {
    const refused = await start(options);
    assert.ok(refused.failure instanceof Error, name);
    assert.equal(isUserCancelledError(refused.failure), false, name);
    assert.equal(refused.sent.length, 1, name);
  }
});

test("an error notice with nothing to say is not rendered, but a retry stays reachable", async () => {
  const path = "../src/components/UI.tsx";
  const tree = ts.createSourceFile(path, await readFile(new URL(path, import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const notice = tree.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "ErrorNotice");
  assert.ok(notice);
  const context = vm.createContext({
    React, Error, isSuspendedError, isUserCancelledError, isIOS: false, t: interpolate,
    AlertCircle: () => null, X: () => null,
  });
  vm.runInContext(ts.transpileModule(`${errorTextSource(tree)}
    ${notice.getText(tree).replace(/^export /, "")}
    globalThis.ErrorNotice = ErrorNotice;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText, context);
  const render = (props) => renderToStaticMarkup(React.createElement(context.ErrorNotice, props));
  assert.equal(render({ message: "", onDismiss() {} }), "", "an empty alert would announce an error with no text");
  assert.match(render({ message: "Storage is full" }), /role="alert".*Storage is full/s);
  assert.match(render({ message: "", onRetry() {} }), /Try again/, "a retry button is never removed");
});

test("the App reporter owns one surface per failure and paints no cancellation", async () => {
  const load = async (path) => ts.createSourceFile(path, await readFile(new URL(path, import.meta.url), "utf8"),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const [app, ui] = await Promise.all([load("../src/App.tsx"), load("../src/components/UI.tsx")]);
  let reporter;
  (function visit(node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(app) === "onError") reporter ??= node.initializer.arguments[0];
    ts.forEachChild(node, visit);
  })(app);
  assert.ok(reporter);
  const quit = { open: true };
  const recovery = { open: true };
  const run = (failure, { dialogs = [], severity, technical = false } = {}) => {
    const log = new DiagnosticLog();
    const shown = [];
    const context = vm.createContext({
      Error, reportError, diagnostics: log, normalizeSeverity, isSuspendedError, isUserCancelledError,
      errorSurface, openDialogs, document: { querySelectorAll: (selector) => selector === "dialog[open]" ? dialogs : [] },
      quitDialog: { current: quit }, mobileRecoveryDialog: { current: recovery },
      showTechnicalErrors: { current: technical }, setError: (value) => shown.push({ ...value }),
      isIOS: false, t: interpolate, failure, severity,
    });
    vm.runInContext(ts.transpileModule(`${errorTextSource(ui)}
      (${reporter.getText(app)})(failure, severity);`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText, context);
    return { shown, log };
  };
  const failure = new Error("The write was refused");
  assert.deepEqual(run(failure).shown, [{ message: failure.message, severity: "blocking", surface: "band" }]);
  // A modal covers the band: the same failure is owned by the top layer, once.
  assert.deepEqual(run(failure, { dialogs: [{ open: true }] }).shown,
    [{ message: failure.message, severity: "blocking", surface: "dialog" }]);
  assert.deepEqual(run(failure, { dialogs: [{ open: true }, { open: true }] }).shown.map((entry) => entry.surface), ["dialog"],
    "nested dialogs still produce one owner");
  // The shell's own quit and recovery dialogs are not a reason to interrupt:
  // the first hides the shell, the second already shows this message.
  assert.equal(run(failure, { dialogs: [quit] }).shown[0].surface, "band");
  assert.equal(run(failure, { dialogs: [recovery] }).shown[0].surface, "band");
  assert.equal(run(failure, { dialogs: [quit, { open: true }] }).shown[0].surface, "dialog");
  // A recoverable diagnostic never interrupts a dialog.
  assert.equal(run(failure, { dialogs: [{ open: true }], severity: "recoverable", technical: true }).shown[0].surface, "band");
  assert.deepEqual(run(failure, { dialogs: [{ open: true }], severity: "recoverable" }).shown, []);
  // The user's own cancellation paints nothing on either surface.
  for (const dialogs of [[], [{ open: true }]])
    assert.deepEqual(run(new UserCancelledError("Download cancelled."), { dialogs }).shown, []);
  const lifecycle = run(new SuspendedError("The application is not interactive"), { dialogs: [{ open: true }] });
  assert.deepEqual(lifecycle.shown, []);
  assert.deepEqual(lifecycle.log.snapshot(), []);
});

test("the error surface is the band unless a blocking failure is covered by a dialog", () => {
  assert.equal(errorSurface("blocking", false), "band");
  assert.equal(errorSurface(undefined, false), "band");
  assert.equal(errorSurface("blocking", true), "dialog");
  assert.equal(errorSurface(undefined, true), "dialog");
  assert.equal(errorSurface("recoverable", true), "band");
  assert.equal(errorSurface("recoverable", false), "band");
  assert.equal(errorSurface("suspended", true), "band");
});

test("selected-storage operations preserve terminal SuspendedError through Library and App reporting", async () => {
  const loadTree = async (path) => {
    const source = await readFile(new URL(path, import.meta.url), "utf8");
    return ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  };
  const [selected, library, app, ui] = await Promise.all([
    loadTree("../src/components/SelectedStorageActions.tsx"),
    loadTree("../src/components/Library.tsx"),
    loadTree("../src/App.tsx"),
    loadTree("../src/components/UI.tsx"),
  ]);
  const find = (tree, predicate) => {
    const found = [];
    function visit(node) {
      if (predicate(node)) found.push(node);
      ts.forEachChild(node, visit);
    }
    visit(tree);
    return found;
  };
  const operations = find(selected, (node) => ts.isFunctionDeclaration(node) &&
    ["chooseLocation", "relocate"].includes(node.name?.text));
  const forwards = find(library, (node) => ts.isJsxSelfClosingElement(node) &&
    node.tagName.getText(library) === "SelectedStorageActions").map((node) =>
    node.attributes.properties.find((attribute) =>
      ts.isJsxAttribute(attribute) && attribute.name.getText(library) === "onError").initializer.expression);
  const reporter = find(app, (node) => ts.isVariableDeclaration(node) &&
    node.name.getText(app) === "onError")[0].initializer.arguments[0];
  const formatter = find(ui, (node) => ts.isFunctionDeclaration(node) && node.name?.text === "errorText")[0];
  assert.equal(operations.length, 2);
  assert.equal(forwards.length, 2, "test bulk and detail storage consumers");
  assert.ok(reporter);
  assert.ok(formatter);
  const compile = (source) => ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  for (const operation of operations) for (const forward of forwards) {
    for (const failure of [
      new SuspendedError("The originating foreground was retired", false),
      new Error("Storage permission was revoked"),
    ]) {
      const log = new DiagnosticLog();
      const bands = [];
      const context = vm.createContext({
        Error, reportError, diagnostics: log, normalizeSeverity, isSuspendedError, isUserCancelledError,
        errorSurface, openDialogs, document: { querySelectorAll: () => [] },
        quitDialog: { current: null }, mobileRecoveryDialog: { current: null },
        showTechnicalErrors: { current: true }, setError: (value) => bands.push(value),
        isIOS: false, t: (value) => value,
        capturePrivacyContext: () => ({ generation: 7, privateMode: false, readOnly: false }),
        isPrivacyContextCurrent: () => true,
        unavailable: false, target: "/managed", removeOld: false, books: [{ id: "book" }],
        api: async () => { throw failure; },
        onChanged: () => { throw new Error("failed operation must not refresh"); },
        setOpen() {}, setBusy() {}, setLocations() {}, setTarget() {}, setRemoveOld() {}, setNotice() {},
      });
      vm.runInContext(compile(`${errorTextSource(ui)}
        const rootOnError = ${reporter.getText(app)};
        const onError = ((onError) => (${forward.getText(library)}))(rootOnError);
        ${operation.getText(selected)}`), context);
      await context[operation.name.text]();
      if (isSuspendedError(failure)) {
        assert.deepEqual(log.snapshot(), [], operation.name.text);
        assert.deepEqual(bands, [], "lifecycle cancellation must not paint the global error band");
      } else {
        assert.equal(log.snapshot().length, 1);
        assert.equal(log.snapshot()[0].message, failure.message);
        assert.equal(bands.length, 1);
        assert.equal(bands[0].message, failure.message, "genuine storage failure remains visible");
        assert.equal(bands[0].surface, "band", "with no dialog open the band owns the failure");
      }
    }
  }
});
