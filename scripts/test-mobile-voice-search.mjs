import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { shouldKeepReaderForBackgroundMedia } from "../src/media-policy.ts";

const source = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let lifecycleEffect;
function visit(node) {
  if (ts.isCallExpression(node) && node.expression.getText(tree) === "useEffect" &&
      node.arguments[0]?.getText(tree).includes('"mobile_voice_search"')) lifecycleEffect = node.arguments[0];
  ts.forEachChild(node, visit);
}
visit(tree);
// Native listener registration is the only omitted statement: focus and
// visibility listeners, queueing, checkpointing, consumption and cleanup are
// the production effect. Expose its queue only to observe settled transitions.
const effectBody = lifecycleEffect.body.statements.filter((statement) =>
  !(ts.isExpressionStatement(statement) && statement.getText(tree).includes('import("@tauri-apps/api/event")')),
).map((statement) => {
  const observer = ts.isReturnStatement(statement) && statement.expression && ts.isArrowFunction(statement.expression)
    ? "globalThis.drainLifecycle = () => queue;\n" : "";
  return observer + statement.getText(tree);
}).join("\n");
const effect = ts.transpileModule(`globalThis.cleanup = (() => { ${effectBody} })();`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

const readerSource = await readFile(new URL("../src/components/Reader.tsx", import.meta.url), "utf8");
const readerTree = ts.createSourceFile("Reader.tsx", readerSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let readerEffect;
let historyCleanup;
function findReaderEffect(node) {
  if (ts.isCallExpression(node) && node.expression.getText(readerTree) === "useEffect" &&
      node.arguments[0]?.getText(readerTree).includes("if (resolvedKey.current === key) {"))
    readerEffect = node.arguments[0];
  if (ts.isVariableDeclaration(node) && node.name.getText(readerTree) === "clearHistoryTraversal")
    historyCleanup = node.initializer.arguments[0];
  ts.forEachChild(node, findReaderEffect);
}
findReaderEffect(readerTree);
const admissionPrefix = [];
for (const statement of readerEffect.body.statements) {
  if (ts.isVariableStatement(statement) && statement.declarationList.declarations.some((item) => item.name.getText(readerTree) === "sequence")) break;
  admissionPrefix.push(statement.getText(readerTree));
}
// Exercise the production identity gate; the observer at its accepted-navigation
// boundary makes a DOM/player replacement visible without a native WebView.
const readerAdmission = ts.transpileModule(`globalThis.clearHistoryTraversal = ${historyCleanup.getText(readerTree)};
globalThis.readerAdmission = () => {
  ${admissionPrefix.join("\n")}
  replaceReaderDocument();
};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const speechSource = await readFile(new URL("../src/native-speech.ts", import.meta.url), "utf8");
const speechTree = ts.createSourceFile("native-speech.ts", speechSource, ts.ScriptTarget.Latest, true);
let speechPrepare;
function findPrepare(node) {
  if (ts.isCallExpression(node) && node.expression.getText(speechTree) === "api" &&
      node.arguments[0]?.getText(speechTree) === '"mobile_speech_prepare"') speechPrepare = node;
  ts.forEachChild(node, findPrepare);
}
findPrepare(speechTree);
const prepareSpeech = ts.transpileModule(`globalThis.prepareSpeech = async () => {
  const session = activeFrame.current;
  const context = { sessionId: session.sessionId, archiveId: session.location.bookId,
    origin: session.location.origin, privacyGeneration: session.privacyGeneration };
  const request = { id: "speech-request" };
  return ${speechPrepare.getText(speechTree)};
};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

function shell(mediaState, platform = "ios") {
  const events = new Map();
  const commands = [];
  const errors = [];
  const queries = [];
  const checkpoints = [];
  const policy = { generation: 7, epoch: 2, privateMode: false, readOnly: false };
  const entry = { kind: "voice-search", requestId: "voice-1", value: "" };
  let state = { active: true, interactive: true, retirementRevision: 0,
    backgroundRequestId: null, incoming: [], entries: mediaState ? [] : [entry] };
  let resolveVoice;
  let rejectVoice;
  let acknowledgmentFailures = 0;
  let backgroundRequests = 0;
  const props = { suspended: false, mediaRetained: false, privacyGeneration: 7, interactive: true, interactionRevision: 0 };
  const tab = { id: "tab-1", bookId: "book-1", path: "article" };
  const key = () => `${tab.id}\n${tab.bookId}\n${tab.path}\n0\n${props.privacyGeneration}`;
  let readerDocument = { sessionId: "session-1", player: { currentTime: 27 } };
  let replacements = 0;
  const session = { sessionId: "session-1", frameId: "frame-1", privacyGeneration: 7,
    installationId: "installation-1", location: { bookId: tab.bookId, origin: "http://archive.localhost:8123" } };
  const node = { contentWindow: {} };
  let nativeOwner = session.sessionId;
  let dormantGeneration = policy.generation;
  const speechCapability = platform === "android" ? { id: "original-request", token: "original-capability", phase: "started" } : null;
  let nativeStops = 0;
  let nativeQueue = Promise.resolve();
  let activationGate;
  let releaseActivation;
  let readerCleanup;
  let readerDependencies = "";
  let readerReady = true;
  let interaction = { interactive: true, revision: 0 };
  function renderReader() {
    if (!mediaState) return;
    Object.assign(context.latest.current, props);
    const dependencies = JSON.stringify([props.suspended, props.mediaRetained, props.privacyGeneration, props.interactive]);
    if (readerDependencies === dependencies) return;
    readerCleanup?.();
    readerDependencies = dependencies;
    readerCleanup = context.readerAdmission();
  }
  const voice = new Promise((resolve, reject) => { resolveVoice = resolve; rejectVoice = reject; });
  const target = {
    addEventListener(name, callback) {
      const callbacks = events.get(name) || new Set();
      callbacks.add(callback);
      events.set(name, callbacks);
    },
    removeEventListener(name, callback) { events.get(name)?.delete(callback); },
    setTimeout, clearTimeout,
  };
  const document = { ...target, visibilityState: "visible" };
  const ref = (current) => ({ current });
  const context = vm.createContext({
    isMobile: true, isAndroid: !mediaState || platform.startsWith("android"), window: target, document,
    clearTimeout, pendingHistoryTraversal: ref(null),
    mounted: ref(true), mobileReconcile: ref(null), mobileVoiceSearches: ref(new Map()),
    processedEntries: ref(new Set()), mobileRetirementRevision: ref(0),
    mobileExternalSelection: ref(null), mobileManagedImport: ref(null), mobileManagedBusy: ref(false),
    mobileLibraryChanges: ref({ requested: 0, applied: 0 }), mobileBooks: ref([]),
    dismissedIncoming: ref(new Set()), privacyPending: ref(false), quitRequest: ref(null),
    hydrated: ref(true), refreshSerial: ref(0), snapshotSuspended: ref(false),
    latestSession: ref({ settings: { privateMode: false, readOnly: false } }), readerMedia: ref(mediaState || {}),
    capturePrivacyContext: () => ({ ...policy }),
    isPrivacyPolicyContextCurrent: (privacy) => privacy.epoch === policy.epoch &&
      privacy.privateMode === policy.privateMode && privacy.readOnly === policy.readOnly,
    requestLibrarySearch: (query) => queries.push(query), getLocale: () => "en",
    onError: (error) => errors.push(error), isSuspendedError: () => false,
    errorText: (error) => String(error),
    props, tab, reload: 0, openSequence: ref(0), resolvedKey: ref(key()), freshOrigin: ref(false),
    pendingNavigation: ref(null), tabRef: ref(tab), publishTab() {},
    activeFrame: ref(session), displayedFrame: ref(session), frameRef: ref(node), retainedActivation: ref(null),
    alive: ref(true), canNavigateReaderStorage: () => true,
    latest: ref({ ...props, settings: { privateMode: false, readOnly: false },
      books: [{ id: tab.bookId, installationId: session.installationId }] }),
    setFrameReady(value) { readerReady = value; },
    setFrame(update) { context.displayedFrame.current = update(context.displayedFrame.current); },
    report: (error) => errors.push(error),
    lifecycle(operation) {
      const result = nativeQueue.then(operation);
      nativeQueue = result.catch(() => {});
      return result;
    },
    replaceReaderDocument() {
      readerDocument = { sessionId: `replacement-${++replacements}`, player: { currentTime: 0 } };
      context.resolvedKey.current = key();
      context.freshOrigin.current = false;
      nativeOwner = readerDocument.sessionId;
    },
    flushSync: (operation) => { operation(); renderReader(); },
    setMobileMediaRetained: (value) => { props.mediaRetained = value; },
    setMobileSuspended: (value) => { props.suspended = value; },
    setMobileInteraction(update) {
      interaction = update(interaction);
      props.interactive = interaction.interactive;
      props.interactionRevision = interaction.revision;
      renderReader();
    },
    setMobileIncoming() {}, setMobileRecoveryNeeded() {},
    setImporting() {}, setDocumentProgress() {},
    shouldKeepReaderForBackgroundMedia,
    freezeReaderStorage: async () => { checkpoints.push("freeze"); return () => checkpoints.push("release"); },
    flushSession: async () => { checkpoints.push("flush"); },
    refresh: async () => {
      if (state.retirementRevision !== context.mobileRetirementRevision.current) policy.generation = 8;
      props.privacyGeneration = policy.generation;
      return true;
    },
    withMobileBackgroundCheckpoint: async (requestId, operation) => { checkpoints.push(requestId); await operation(); },
    api: async (command, args) => {
      commands.push(command);
      if (command === "mobile_lifecycle_state") return state;
      if (command === "mobile_voice_search") return voice;
      if (command === "sensor_activate" || command === "mobile_speech_restore_reader") {
        if (command === "sensor_activate" && speechCapability) {
          nativeStops++;
          speechCapability.phase = "retiring"; // Ordinary dispatch must still retire replacements.
        }
        assert.equal(args.sessionId, session.sessionId);
        assert.equal(args.archiveId, session.location.bookId);
        assert.equal(args.origin, session.location.origin);
        assert.equal(args.privacyGeneration, session.privacyGeneration);
        if (activationGate) await activationGate;
        if (command === "mobile_speech_restore_reader") {
          assert.equal(args.installationId, session.installationId);
          if (!state.active || !state.interactive || args.privacyGeneration !== policy.generation ||
              (speechCapability && speechCapability.phase !== "started"))
            throw new Error("SPEECH_SESSION_RETIRED");
        }
        nativeOwner = args.sessionId;
        return { camera: "notAsked", location: "notAsked", nativeLocation: true };
      }
      if (command === "sensor_deactivate") {
        if (nativeOwner === args.sessionId) nativeOwner = null;
      }
      if (command === "mobile_speech_prepare") {
        if (nativeOwner !== args.sessionId) throw new Error("SPEECH_READER_SESSION_RETIRED");
        return { id: args.id, token: "prepared-token" };
      }
      if (command === "mobile_speech_pause") {
        if (nativeOwner !== session.sessionId || speechCapability?.phase !== "started" ||
            args.id !== speechCapability.id || args.token !== speechCapability.token)
          throw new Error("SPEECH_SESSION_RETIRED");
        return { id: args.id, paused: true };
      }
      if (command === "mobile_ack_entry") {
        assert.equal(args.requestId, entry.requestId);
        if (acknowledgmentFailures-- > 0) throw new Error("acknowledgment failed");
        state = { ...state, entries: [] };
      }
    },
  });
  vm.runInContext(readerAdmission, context);
  vm.runInContext(prepareSpeech, context);
  vm.runInContext(effect, context);
  const wake = () => { for (const callback of events.get("focus") || []) callback(); };
  return {
    context, policy, commands, errors, queries, checkpoints, wake,
    resolveVoice, rejectVoice,
    props, readerDocument: () => readerDocument, replacements: () => replacements,
    retire() { state = { ...state, retirementRevision: state.retirementRevision + 1 }; },
    readerReady: () => readerReady, nativeOwner: () => nativeOwner,
    prepareSpeech: () => context.prepareSpeech(),
    speechCapability, nativeStops: () => nativeStops,
    pauseSpeech: () => context.api("mobile_speech_pause", { id: speechCapability.id, token: speechCapability.token }),
    nativeProgress() {
      const valid = nativeOwner === session.sessionId && policy.generation === session.privacyGeneration;
      if (!valid && speechCapability) { speechCapability.phase = "retiring"; nativeStops++; }
      return valid;
    },
    holdActivation() {
      activationGate = new Promise((resolve) => { releaseActivation = resolve; });
      return () => { releaseActivation(); activationGate = undefined; };
    },
    changePrivacy() {
      policy.generation++;
      props.privacyGeneration = policy.generation;
      renderReader();
    },
    failAcknowledgment() { acknowledgmentFailures = 1; },
    async drain({ native = true } = {}) {
      await new Promise(setImmediate);
      await context.drainLifecycle();
      if (native) await nativeQueue;
    },
    background({ retired = true } = {}) {
      // Models native background suspension: requests/grants are inactive,
      // while a dormant exact-owner attestation can survive source retention.
      dormantGeneration = policy.generation;
      nativeOwner = null;
      state = { ...state, active: false, interactive: false,
        retirementRevision: retired ? state.retirementRevision + 1 : state.retirementRevision,
        backgroundRequestId: `checkpoint-${++backgroundRequests}` };
      document.visibilityState = "hidden";
      wake();
    },
    foreground() {
      // Native restoration precedes active publication, including a progress
      // event delivered before the frontend confirmation can be dispatched.
      if (platform.startsWith("android") && dormantGeneration === policy.generation &&
          state.retirementRevision === context.mobileRetirementRevision.current &&
          !context.latestSession.current.settings.privateMode && !context.latestSession.current.settings.readOnly &&
          (!speechCapability || speechCapability.phase === "started"))
        nativeOwner = session.sessionId;
      state = { ...state, active: true, interactive: true, backgroundRequestId: null };
      document.visibilityState = "visible";
      wake();
    },
    close() { readerCleanup?.(); context.alive.current = false; context.mounted.current = false; context.cleanup(); },
  };
}

test("delayed recognition does not block lifecycle wakes/checkpoints and is consumed once after foreground", { timeout: 3000 }, async () => {
  const app = shell();
  app.wake();
  await app.drain();
  assert.deepEqual(app.queries, [""]);
  assert.equal(app.commands.filter((command) => command === "mobile_voice_search").length, 1);
  app.wake(); // A focus revision while the recognizer is still pending is not cancellation.
  await app.drain();
  app.background();
  await app.drain();
  assert.ok(app.checkpoints.includes("checkpoint-1"), "background checkpoint runs before recognition resolves");
  assert.ok(app.commands.includes("mobile_background_ack"));
  app.resolveVoice({ query: "  Spoken article  " });
  await app.drain();
  assert.deepEqual(app.queries, [""], "hidden results cannot navigate");
  assert.equal(app.commands.includes("mobile_ack_entry"), false, "pending continuation retains its native entry");
  app.failAcknowledgment();
  app.foreground();
  await app.drain();
  assert.equal(app.policy.generation, 8, "background reader authority was refreshed");
  assert.deepEqual(app.queries, ["", "Spoken article"], "same-policy result survives reader generation retirement");
  app.wake();
  await app.drain();
  app.wake();
  await app.drain();
  assert.deepEqual(app.queries, ["", "Spoken article"]);
  assert.equal(app.commands.filter((command) => command === "mobile_voice_search").length, 1);
  assert.equal(app.commands.filter((command) => command === "mobile_ack_entry").length, 2, "failed ack retries without applying again");
  assert.equal(app.errors.length, 1, "genuine acknowledgment failure is not suppressed");
  app.close();
});

test("a privacy-policy cutover discards a delayed transcript instead of borrowing foreground authority", { timeout: 3000 }, async () => {
  const app = shell();
  app.wake();
  await app.drain();
  app.background();
  await app.drain();
  app.policy.epoch++;
  app.policy.privateMode = true;
  app.resolveVoice({ query: "Old private transcript" });
  await app.drain();
  app.foreground();
  await app.drain();
  assert.deepEqual(app.queries, [""]);
  assert.equal(app.commands.filter((command) => command === "mobile_ack_entry").length, 1);
  assert.deepEqual(app.errors, []);
  app.close();
});

test("a recognizer failure is reported once after return, but a retired shell never consumes its continuation", { timeout: 3000 }, async () => {
  const app = shell();
  app.wake();
  await app.drain();
  app.background();
  await app.drain();
  const failure = new Error("Speech permission was revoked");
  app.rejectVoice(failure);
  await app.drain();
  assert.deepEqual(app.errors, []);
  app.foreground();
  await app.drain();
  app.wake();
  await app.drain();
  assert.deepEqual(app.errors, [failure]);
  app.close();

  const retired = shell();
  retired.wake();
  await retired.drain();
  retired.close();
  retired.resolveVoice({ query: "Unowned replacement window result" });
  await retired.drain();
  assert.deepEqual(retired.queries, [""]);
  assert.equal(retired.commands.includes("mobile_ack_entry"), false);
});

test("valid audible/PiP retention keeps the same reader document, session and playback position across foreground", { timeout: 3000 }, async () => {
  for (const media of [{ audible: true, pictureInPicture: false }, { audible: false, pictureInPicture: true }]) {
    const app = shell(media);
    const document = app.readerDocument();
    app.wake();
    await app.drain();
    app.background({ retired: false });
    await app.drain();
    assert.equal(app.props.suspended, true, "all suspension/action guards remain active");
    assert.equal(app.props.mediaRetained, true);
    assert.equal(app.checkpoints.includes("freeze"), false);
    assert.equal(app.readerDocument(), document);
    assert.equal(app.nativeOwner(), null, "native background handling still deactivates sensors");
    assert.equal(app.readerReady(), false, "speech and iframe actions wait for exact-session restoration");
    app.foreground();
    await app.drain();
    assert.equal(app.props.suspended, false);
    assert.equal(app.props.mediaRetained, false);
    assert.equal(app.readerDocument(), document);
    assert.equal(app.readerDocument().sessionId, "session-1");
    assert.equal(app.readerDocument().player.currentTime, 27);
    assert.equal(app.replacements(), 0);
    assert.equal(app.nativeOwner(), "session-1", "foreground reactivates the retained owner without navigation");
    assert.equal(app.readerReady(), true);
    assert.equal((await app.prepareSpeech()).token, "prepared-token");
    assert.ok(app.commands.includes("sensor_activate"));
    assert.ok(app.commands.includes("mobile_speech_prepare"));
    app.close();
  }
});

test("non-media, privacy/read-only changes and native retirement cannot reuse retained document authority", { timeout: 3000 }, async () => {
  for (const boundary of ["non-media", "private", "read-only", "retirement", "quitting"]) {
    const app = shell({ audible: boundary !== "non-media", pictureInPicture: false });
    const document = app.readerDocument();
    app.wake();
    await app.drain();
    if (boundary === "private") app.context.latestSession.current.settings.privateMode = true;
    if (boundary === "read-only") app.context.latestSession.current.settings.readOnly = true;
    if (boundary === "quitting") app.context.quitRequest.current = {};
    app.background({ retired: false });
    await app.drain();
    if (boundary === "retirement") {
      assert.equal(app.props.mediaRetained, true);
      app.retire(); // Native release/revocation wins over stale audible UI state.
    } else {
      assert.equal(app.props.mediaRetained, false, boundary);
      assert.ok(app.checkpoints.includes("freeze"), boundary);
    }
    app.foreground();
    await app.drain();
    assert.notEqual(app.readerDocument(), document, boundary);
    assert.equal(app.replacements(), 1, boundary);
    app.close();
  }
});

test("retirement or privacy changes beat delayed retained-session activation and retire its late native owner", { timeout: 3000 }, async () => {
  for (const boundary of ["retirement", "privacy"]) {
    const app = shell({ audible: true, pictureInPicture: false });
    app.wake();
    await app.drain();
    app.background({ retired: false });
    await app.drain();
    const release = app.holdActivation();
    app.foreground();
    await app.drain({ native: false });
    assert.equal(app.readerReady(), false);
    assert.equal(app.nativeOwner(), null);
    if (boundary === "retirement") {
      app.retire();
      app.wake();
      await app.drain({ native: false });
    } else app.changePrivacy();
    release();
    await app.drain();
    assert.ok(app.commands.includes("sensor_deactivate"), boundary);
    assert.notEqual(app.nativeOwner(), "session-1", "a late activation cannot revive the old native owner");
    assert.equal(app.readerReady(), false, "late activation cannot enable the retired reader");
    assert.equal(app.replacements(), 1, boundary);
    app.close();
  }
});

test("Android retained read-aloud restores its exact native reader owner without stopping or replacing speech", { timeout: 3000 }, async () => {
  const app = shell({ audible: true, pictureInPicture: false }, "android");
  const document = app.readerDocument();
  app.wake();
  await app.drain();
  app.background({ retired: false });
  await app.drain();
  assert.equal(app.nativeOwner(), null);
  assert.equal(app.speechCapability.phase, "started");
  app.foreground();
  assert.equal(app.nativeProgress(), true, "native progress before JS restoration must retain speech");
  assert.equal(app.commands.includes("mobile_speech_restore_reader"), false, "frontend has not confirmed restoration yet");
  await app.drain();
  assert.equal(app.readerDocument(), document);
  assert.equal(app.readerDocument().player.currentTime, 27);
  assert.equal(app.nativeOwner(), "session-1");
  assert.equal(app.speechCapability.token, "original-capability");
  assert.equal(app.speechCapability.phase, "started");
  assert.equal(app.nativeStops(), 0);
  assert.equal(app.commands.includes("sensor_activate"), false, "ordinary activation would retire retained read-aloud");
  assert.equal(app.commands.includes("sensor_deactivate"), false);
  assert.ok(app.commands.includes("mobile_speech_restore_reader"));
  assert.equal((await app.pauseSpeech()).paused, true, "original transport capability still works");
  app.close();
});

test("Android archive audible/PiP retention without TTS restores only the existing reader owner", { timeout: 3000 }, async () => {
  for (const media of [{ audible: true, pictureInPicture: false }, { audible: false, pictureInPicture: true }]) {
    const app = shell(media, "android-media");
    const document = app.readerDocument();
    app.wake();
    await app.drain();
    app.background({ retired: false });
    await app.drain();
    assert.equal(app.nativeOwner(), null);
    app.foreground();
    await app.drain();
    assert.equal(app.readerDocument(), document);
    assert.equal(app.readerDocument().player.currentTime, 27);
    assert.equal(app.speechCapability, null, "no TTS capability is invented as a restoration prerequisite");
    assert.equal(app.nativeOwner(), "session-1");
    assert.equal(app.nativeStops(), 0);
    assert.ok(app.commands.includes("mobile_speech_restore_reader"));
    assert.equal((await app.prepareSpeech()).token, "prepared-token");
    app.close();
  }
});

test("another Android background transition beats delayed frontend confirmation without retiring retained speech", { timeout: 3000 }, async () => {
  const app = shell({ audible: true, pictureInPicture: false }, "android");
  app.wake();
  await app.drain();
  app.background({ retired: false });
  await app.drain();
  const release = app.holdActivation();
  app.foreground();
  await app.drain({ native: false });
  app.background({ retired: false });
  await app.drain({ native: false });
  release();
  await app.drain();
  assert.equal(app.nativeOwner(), null, "obsolete confirmation cannot reactivate a background sensor owner");
  assert.equal(app.readerReady(), false);
  assert.equal(app.commands.includes("sensor_deactivate"), false, "cleanup must not retire retained read-aloud");
  assert.equal(app.nativeStops(), 0);
  assert.equal(app.speechCapability.phase, "started");
  app.close();
});
