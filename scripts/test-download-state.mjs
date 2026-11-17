import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { cleanupDownloadBooks, downloadStatusDecision, downloadLiveSummary, downloadNotificationPhase, rankJobs, remainingTime } from "../src/download-state.ts";

function job(overrides = {}) {
  return {
    id: "job",
    url: "https://example.org/archive.zim",
    fileName: "archive.zim",
    destination: "/downloads/archive.zim",
    status: "paused",
    downloaded: 0,
    total: 128,
    error: null,
    speed: 0,
    ...overrides,
  };
}

test("a crash-interrupted pause outranks its informational error", () => {
  const decision = downloadStatusDecision(
    job({
      interrupted: true,
      error: "Download interrupted by application shutdown; resume when ready",
    }),
    false,
    false,
  );
  assert.deepEqual(decision, { kind: "paused", interrupted: true });
});

test("an ordinary paused job with an error still reads as paused", () => {
  assert.deepEqual(
    downloadStatusDecision(job({ error: "The network connection was lost" }), false, false),
    { kind: "paused", interrupted: false },
  );
  assert.deepEqual(downloadStatusDecision(job(), false, false), {
    kind: "paused",
    interrupted: false,
  });
});

test("the disk-space confirmation stays ahead of every status", () => {
  assert.deepEqual(
    downloadStatusDecision(job({ status: "failed", error: "LOW_DISK_SPACE:{}" }), false, true),
    { kind: "confirmation" },
  );
});

test("a real error still reports failed when the job is not paused", () => {
  assert.deepEqual(
    downloadStatusDecision(job({ status: "failed", error: "The transfer stopped" }), false, false),
    { kind: "failed" },
  );
  assert.deepEqual(
    downloadStatusDecision(job({ status: "downloading", interrupted: true, error: "boom" }), false, false),
    { kind: "failed" },
  );
});

test("a ready completed job reads completed", () => {
  assert.deepEqual(
    downloadStatusDecision(job({ status: "complete", interrupted: false }), true, false),
    { kind: "completed" },
  );
});

test("a verifying job is a local finalize, never a stalled download", () => {
  const decision = downloadStatusDecision(
    job({ status: "verifying", downloaded: 128, total: 128, error: null }),
    false,
    false,
  );
  assert.deepEqual(decision, { kind: "verifying" });
  assert.notEqual(decision.kind, "status");
});

test("verification outranks a stale informational error", () => {
  assert.deepEqual(
    downloadStatusDecision(
      job({ status: "verifying", downloaded: 128, total: 128, error: "The network connection was lost" }),
      false,
      false,
    ),
    { kind: "verifying" },
  );
});

test("the card projects a live rate and remaining time for an active transfer", () => {
  assert.deepEqual(
    downloadLiveSummary(job({ status: "downloading", speed: 2048, etaSeconds: 90.2 })),
    { kind: "active", bytesPerSecond: 2048, etaSeconds: 90.2 },
  );
  assert.deepEqual(
    downloadLiveSummary(job({ status: "downloading", speed: Number.NaN, etaSeconds: Number.NaN })),
    { kind: "active", bytesPerSecond: 0, etaSeconds: null },
  );
  assert.deepEqual(
    downloadLiveSummary(job({ status: "connecting", speed: 0, etaSeconds: null })),
    { kind: "active", bytesPerSecond: 0, etaSeconds: null },
  );
});

test("paused and verifying rows replace the rate with a note instead of a stale number", () => {
  assert.deepEqual(downloadLiveSummary(job({ status: "paused", speed: 4096 })), { kind: "paused" });
  assert.deepEqual(
    downloadLiveSummary(job({ status: "verifying", speed: 4096 })),
    { kind: "verifying" },
  );
});

test("terminal rows carry no live line", () => {
  for (const status of ["complete", "completed", "seeding", "failed", "cancelled"])
    assert.equal(downloadLiveSummary(job({ status })), null, status);
  assert.equal(downloadLiveSummary(job({ status: "downloading", error: "boom" })), null);
});

test("remaining time uses the coarsest unit a person can act on", () => {
  assert.deepEqual(remainingTime(0), { count: 1, unit: "seconds" });
  assert.deepEqual(remainingTime(9.2), { count: 10, unit: "seconds" });
  assert.deepEqual(remainingTime(60), { count: 1, unit: "minutes" });
  assert.deepEqual(remainingTime(90), { count: 2, unit: "minutes" });
  assert.deepEqual(remainingTime(3600), { count: 1, unit: "hours" });
  assert.deepEqual(remainingTime(7200), { count: 2, unit: "hours" });
});

test("notification replay follows native acquisition ownership and phase, not HTTP provenance", () => {
  const active = ["queued", "connecting", "resolving", "initializing", "metadata", "checking", "downloading", "verifying", "publishing"];
  for (const transport of [undefined, "http", "quic-library", "unknown", "torrent"]) {
    for (const status of active) {
      assert.equal(downloadNotificationPhase(job({ transport, status })), "active", `${transport}:${status}`);
    }
    for (const status of ["paused", "failed", "error"]) {
      assert.equal(downloadNotificationPhase(job({ transport, status })), status === "paused" ? "paused" : "failed",
        `${transport}:${status}`);
    }
    for (const status of ["creating", "complete", "completed", "seeding", "cancelled"]) {
      assert.equal(downloadNotificationPhase(job({ transport, status })), null, `${transport}:${status}`);
    }
  }
  for (const transport of ["localswarm", "map", "future-transport"]) {
    for (const status of active.concat(["paused", "failed", "error"])) {
      assert.equal(downloadNotificationPhase(job({ transport, status })), null, `${transport}:${status}`);
    }
  }
});

test("new and legacy library acquisition cleanup retains installed destination candidates", () => {
  const installed = { id: "archive", installationId: "installed", path: "/downloads/archive.zim" };
  const unrelated = { id: "other", installationId: "other-installation", path: "/downloads/other.zim" };
  for (const transport of [undefined, "http", "quic-library", "unknown"]) {
    for (const status of ["complete", "completed"]) {
      const completed = job({ transport, status, url: "kiwix-library://session/archive" });
      assert.deepEqual(cleanupDownloadBooks(completed, [installed, unrelated]), [installed]);
    }
    assert.deepEqual(cleanupDownloadBooks(job({ transport, status: "paused" }), [installed]), []);
  }
  for (const transport of ["torrent", "localswarm", "map", "future-transport"]) {
    assert.deepEqual(cleanupDownloadBooks(job({ transport, status: "complete" }), [installed]), []);
    assert.deepEqual(cleanupDownloadBooks(job({
      transport, status: "complete", completedPaths: [installed.path],
    }), [installed, unrelated]), [installed]);
  }
});

// Execute the production action closure with its boundary owners, following
// the provider-recovery consumer tests rather than mirroring command logic.
const downloadsSource = await readFile(new URL("../src/components/Downloads.tsx", import.meta.url), "utf8");
const downloadsTree = ts.createSourceFile("Downloads.tsx", downloadsSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let actionSource;
function findAction(node) {
  if (ts.isFunctionDeclaration(node) && node.name?.text === "action") actionSource = node.getText(downloadsTree);
  ts.forEachChild(node, findAction);
}
findAction(downloadsTree);
assert.ok(actionSource, "production Downloads action is present");
const compiledAction = ts.transpileModule(`${actionSource}\nglobalThis.perform = action;`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function downloadActions(currentJob) {
  const privacy = { generation: 1, epoch: 1, privateMode: false, readOnly: false };
  const commands = [];
  const confirmations = [];
  const completedNotifications = [];
  const context = vm.createContext({
    actionPending: { current: false }, cleanupPending: { current: false },
    settingsRef: { current: { readOnly: false } },
    capturePrivacyContext: () => privacy, isPrivacyContextCurrent: (value) => value === privacy,
    localBlockedRef: { current: false }, notificationInteractiveRef: { current: true },
    alive: { current: true }, jobsRef: { current: [currentJob] },
    terminalStatus: (status) => ["complete", "completed", "failed", "cancelled"].includes(status),
    setResumePin() {}, setResumeError() {}, setResumeRequest: (value) => confirmations.push(value),
    setCancellation: (value) => confirmations.push(value), notificationOwner: { current: null },
    parseDiskSpaceWarning: () => null, targetRef: { current: null },
    configurationRef: { current: "configuration" }, setWorking() {},
    restartedJobs: { current: new Map() },
    api: async (command, args) => {
      if (command === "downloads") return context.jobsRef.current;
      commands.push({ command, args });
    },
    previous: { current: new Map() }, previousPayload: { current: { jobs: "", map: "", editions: "" } },
    setJobs: (value) => { context.jobsRef.current = value; },
    downloadNotificationPhase,
    finishNotificationAction: (id, complete) => completedNotifications.push({ id, complete }),
    onRefresh() {}, load: async () => {}, closeResume() {},
    onError: (error) => { throw error; },
  });
  vm.runInContext(compiledAction, context);
  return { privacy, context, commands, confirmations, completedNotifications };
}

test("library transport provenance never opts generic resumes into selected-book PIN admission", async () => {
  for (const transport of ["quic-library", "unknown"]) {
    const currentJob = job({ transport, requiresPin: true, admissionGeneration: 17 });
    const ui = downloadActions(currentJob);
    assert.equal(await ui.context.perform(currentJob, "resume_download"), "complete");
    assert.deepEqual(ui.confirmations, []);
    assert.deepEqual(JSON.parse(JSON.stringify(ui.commands)), [
      { command: "resume_download", args: { id: currentJob.id } },
    ]);
  }
  const currentJob = job({ transport: "localswarm", requiresPin: true, admissionGeneration: 17 });
  const ui = downloadActions(currentJob);
  assert.equal(await ui.context.perform(currentJob, "resume_download"), "confirmation");
  assert.equal(ui.confirmations.length, 1);
  assert.deepEqual(ui.commands, []);
});

test("notification cancellation accepts native library acquisition IDs but revalidates phase and destination", async () => {
  for (const transport of ["quic-library", "unknown"]) {
    for (const changed of [null, { status: "downloading" }, { destination: "/downloads/replacement.zim" }, { transport: "localswarm" }]) {
      const currentJob = job({ transport });
      const ui = downloadActions(currentJob);
      if (changed) ui.context.jobsRef.current = [{ ...currentJob, ...changed }];
      assert.equal(await ui.context.perform(currentJob, "cancel_download", {
        notificationRequestId: "native-request", notificationPhase: "paused", cancellationPrivacy: ui.privacy,
      }), "complete");
      assert.deepEqual(JSON.parse(JSON.stringify(ui.commands)), changed ? [] : [
        { command: "cancel_download", args: { id: currentJob.id } },
      ]);
      assert.equal(ui.completedNotifications.length, changed ? 0 : 1);
    }
  }
});

test("library provenance does not bypass read-only, retired privacy, or interactive notification gates", async () => {
  for (const transport of ["quic-library", "unknown"]) {
    for (const gate of ["readOnly", "retired", "privateNotification", "backgroundNotification"]) {
      const currentJob = job({ transport });
      const ui = downloadActions(currentJob);
      if (gate === "readOnly") ui.context.settingsRef.current.readOnly = true;
      if (gate === "retired") ui.context.isPrivacyContextCurrent = () => false;
      if (gate === "privateNotification") ui.privacy.privateMode = true;
      if (gate === "backgroundNotification") ui.context.notificationInteractiveRef.current = false;
      assert.equal(await ui.context.perform(currentJob, "resume_download",
        gate.endsWith("Notification") ? { notificationRequestId: "native-request" } : {}), "deferred");
      assert.deepEqual(ui.commands, []);
      assert.deepEqual(ui.confirmations, []);
      assert.deepEqual(ui.completedNotifications, []);
    }
  }
});

test("downloads are listed newest first, whatever their state", () => {
  const older = job({ id: "older", status: "downloading", createdAt: "2026-01-01T00:00:00Z" });
  const paused = job({ id: "paused", status: "paused", createdAt: "2026-01-02T00:00:00Z" });
  const newest = job({
    id: "newest", status: "completed",
    createdAt: "2026-01-03T00:00:00Z", updatedAt: "2026-01-03T00:05:00Z",
  });
  const seen = new Map([["older", 1], ["paused", 2], ["newest", 3]]);
  assert.deepEqual(
    rankJobs([older, paused, newest], seen).map((entry) => entry.id),
    ["newest", "paused", "older"],
  );
});

test("a download keeps its place when it finishes", () => {
  const first = job({ id: "first", status: "completed", createdAt: "2026-01-01T00:00:00Z" });
  const running = job({ id: "running", status: "downloading", createdAt: "2026-01-02T00:00:00Z" });
  const seen = new Map([["first", 1], ["running", 2]]);
  const before = rankJobs([first, running], seen).map((entry) => entry.id);
  const after = rankJobs([first, { ...running, status: "completed", updatedAt: "2026-01-05T00:00:00Z" }], seen)
    .map((entry) => entry.id);
  assert.deepEqual(before, ["running", "first"]);
  assert.deepEqual(after, before);
});

test("resuming or retrying a download moves it back to the top", () => {
  const failed = job({ id: "failed", status: "failed", createdAt: "2026-01-01T00:00:00Z" });
  const recent = job({ id: "recent", status: "completed", createdAt: "2026-01-05T00:00:00Z" });
  const seen = new Map([["failed", 1], ["recent", 2]]);
  assert.deepEqual(rankJobs([failed, recent], seen).map((entry) => entry.id), ["recent", "failed"]);
  const restarted = new Map([["failed", Date.parse("2026-01-06T00:00:00Z")]]);
  assert.deepEqual(rankJobs([failed, recent], seen, restarted).map((entry) => entry.id), ["failed", "recent"]);
});

test("the map download takes its place by start time", () => {
  const older = { id: "older", createdAt: "2026-01-01T00:00:00Z" };
  const map = { id: "map:1", createdAt: "2026-01-02T00:00:00Z" };
  const newest = { id: "newest", createdAt: "2026-01-03T00:00:00Z" };
  const legacyMap = { id: "map:legacy" };
  const seen = new Map([["older", 1], ["map:1", 2], ["newest", 3], ["map:legacy", 4]]);
  assert.deepEqual(rankJobs([older, map, newest], seen).map((entry) => entry.id), ["newest", "map:1", "older"]);
  assert.deepEqual(rankJobs([legacyMap, older], seen).map((entry) => entry.id), ["older", "map:legacy"]);
});

test("jobs without a start stamp follow, newest first-seen first", () => {
  const stamped = job({ id: "stamped", status: "completed", createdAt: "2026-01-01T00:00:00Z" });
  const first = job({ id: "first", status: "completed" });
  const second = job({ id: "second", status: "completed" });
  const seen = new Map([["stamped", 1], ["first", 2], ["second", 3]]);
  assert.deepEqual(
    rankJobs([first, stamped, second], seen).map((entry) => entry.id),
    ["stamped", "second", "first"],
  );
});
