import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Check,
  Copy,
  FolderOpen,
  Pause,
  Play,
  RefreshCw,
  Share2,
  Upload,
  X,
} from "lucide-react";
import { api, canShareSystem, capturePrivacyContext, confirmDiskSpaceWarning, editionDownload, isIOS, isPrivacyContextCurrent, parseDiskSpaceWarning, saveBytes, shareArchive, type PrivacyContext } from "../api";
import { t, formatBytes, formatNumber, languageName } from "../i18n";
import type { Book, DownloadAdmission, DownloadJob, DownloadNotificationAction, DownloadTarget, EditionIntent, EditionOriginalDiscarded, EditionPreparation, EditionStatus, EditionTransaction, MapJob, Settings, TabLocation } from "../types";
import { isUserCancelledError, type ErrorSeverity } from "../error-visibility";
import { cleanupDownloadBooks, downloadLiveSummary, downloadNotificationPhase, downloadStatusDecision, mapDownloadConfiguration, rankJobs, readyDownloadBooks, remainingTime } from "../download-state";
import { displayPath } from "../display-path";
import { SwipeRow } from "./SwipeRow";
import { MapBlockProgress, MapRecoveryPaths, MapTransferRate } from "./MapBlockProgress";
import { DownloadCleanupDialog } from "./DownloadCleanupDialog";
import {
  ErrorNotice,
  Loading,
  PageHeader,
  Modal,
  confirmAction,
  errorText,
  jobStatus,
  pathFileName,
  terminalStatus,
} from "./UI";
interface Props {
  settings: Settings;
  books: Book[];
  /** `showGlobally: false` records a failure this pane already shows inline. */
  onError: (error: unknown, severity?: ErrorSeverity, showGlobally?: boolean) => void;
  onRefresh: () => void;
  focusTarget?: DownloadTarget | null;
  notificationAction?: DownloadNotificationAction | null;
  notificationInteractive: boolean;
  notificationInteractionRevision: number;
  onBeginNotificationAction: (requestId: string) => boolean;
  onFinishNotificationAction: (requestId: string, complete: boolean) => void;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
  onDismissTarget: (target: DownloadTarget) => void;
  onMaps: () => void;
  onOpen: (location: TabLocation, target: DownloadTarget) => void;
}
type CleanupTarget =
  { kind: "download"; job: DownloadJob; retained?: boolean } | { kind: "map"; job: MapJob };
/** A live transfer needs frequent progress; a pane with only terminal rows does not. */
const ACTIVE_POLL_MS = 1200;
const IDLE_POLL_MS = 5000;
/** Book identity as the library and edition records spell it. */
function installationKey(id: string, installationId: string) {
  return `${id}\u0000${installationId}`;
}
type CleanupRequest = CleanupTarget & {
  privacy: PrivacyContext; books: Book[]; removedInstallations: Set<string>;
};
function EditionDownloadActions({ intent, authorized: locallyAuthorized, blocked, busy, targetCurrent, onReview, onReplace, onCancel }: {
  intent: EditionIntent;
  authorized: boolean;
  blocked: boolean;
  busy: boolean;
  targetCurrent: boolean;
  onReview: () => void;
  onReplace: () => void;
  onCancel: () => void;
}) {
  if (intent.status === "cancelled") return <p className="muted">{t("Edition update cancelled. Your original archive is unchanged.")}</p>;
  if (intent.status === "committed") return <p className="success-text">{t("Edition replaced. The original archive is retained for rollback.")}</p>;
  const ready = !!intent.verified;
  const authorized = locallyAuthorized && !intent.needsAuthorization;
  return (
    <div className="edition-result">
      <h3>{t("Archive edition")}</h3>
      <p>{t("Installed edition: {title} ({date})", { title: intent.target.title, date: intent.target.date || t("Date unknown") })}</p>
      <p>{t("New edition: {title} ({date})", { title: intent.selection.entry.title, date: intent.selection.entry.date || t("Date unknown") })}</p>
      <p className="file-path"><bdi>{intent.selection.sourceUrl}</bdi></p>
      <p>{t("Full download: {size}", { size: intent.selection.entry.size > 0 ? formatBytes(intent.selection.entry.size) : t("Size unknown") })}</p>
      <p role="status">{ready
        ? t("Ready — the full archive has been verified. It will not replace your installed edition until you choose Replace.")
        : t("The original archive stays installed while the full new edition downloads and is verified.")}</p>
      <p className="muted">{t("Original archive files are retained. Replacement does not transfer website storage, sensor permissions or sharing consent.")}</p>
      {!targetCurrent && <p className="warning-text">{t("The installed archive changed. Check for editions again before replacing.")}</p>}
      {intent.error && <p className="inline-error" role="alert">{errorText(intent.error)}</p>}
      {intent.needsAuthorization && <p className="warning-text">{t("Review this edition again to authorize replacement in this session. Verified files are retained.")}</p>}
      <div className="download-actions">
        {(ready || intent.needsAuthorization || intent.status === "reserved") && !authorized && (
          <button disabled={blocked || busy || !targetCurrent} onClick={onReview}>
            {intent.status === "reserved" && !ready ? t("Retry full download") : t("Review replacement")}
          </button>
        )}
        {ready && authorized && (
          <button className="primary" disabled={blocked || busy || !targetCurrent} onClick={onReplace}>{t("Replace")}</button>
        )}
        <button disabled={blocked || busy} onClick={onCancel}>{t("Cancel edition update")}</button>
      </div>
    </div>
  );
}
export default function Downloads({
  settings,
  books,
  onError,
  onRefresh,
  focusTarget,
  notificationAction,
  notificationInteractive,
  notificationInteractionRevision,
  onBeginNotificationAction,
  onFinishNotificationAction,
  onDownload,
  onDismissTarget,
  onMaps,
  onOpen,
}: Props) {
  const [jobs, setJobs] = useState<DownloadJob[]>([]);
  const [mapJob, setMapJob] = useState<MapJob | null>(null);
  const [editions, setEditions] = useState<EditionStatus>({ intents: [], transactions: [] });
  const [editionError, setEditionError] = useState("");
  const [editionActionError, setEditionActionError] = useState("");
  const [editionWorking, setEditionWorking] = useState<string | null>(null);
  const [originalOutcome, setOriginalOutcome] = useState<{ id: string; message: string; failed: boolean } | null>(null);
  const originalOutcomeFocus = useRef(false);
  const [editionAuthorizations, setEditionAuthorizations] = useState<Record<string, { token: string; privacy: PrivacyContext }>>({});
  const editionPending = useRef(false);
  const previousEditionRevision = useRef("");
  const [retainedFiles, setRetainedFiles] = useState<DownloadJob[]>([]);
  const [retainedLoading, setRetainedLoading] = useState(false);
  const [retainedError, setRetainedError] = useState("");
  const retainedFilesRef = useRef(retainedFiles);
  retainedFilesRef.current = retainedFiles;
  const retainedFilesOpen = useRef(false);
  const retainedRequest = useRef(0);
  const [mapError, setMapError] = useState("");
  const [mapWorking, setMapWorking] = useState(false);
  const previousMap = useRef("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [historyNotice, setHistoryNotice] = useState("");
  const [downloadNotice, setDownloadNotice] = useState("");
  const [working, setWorking] = useState<string | null>(null);
  const [copiedMagnet, setCopiedMagnet] = useState<string | null>(null);
  const [source, setSource] = useState("");
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState("");
  const startPending = useRef(false);
  const sourceRevision = useRef(0);
  const mapPending = useRef(false);
  const focusedCard = useRef<HTMLElement | null>(null);
  const focusedTarget = useRef<{ target?: DownloadTarget | null; handled: boolean }>({ handled: false });
  const targetRef = useRef(focusTarget);
  targetRef.current = focusTarget;
  const previous = useRef(new Map<string, string>());
  const previousPayload = useRef({ jobs: "", map: "", editions: "" });
  const alive = useRef(true);
  const request = useRef(0);
  const [resumeRequest, setResumeRequest] = useState<{ job: DownloadJob; privacy: PrivacyContext } | null>(null);
  const resumeJob = resumeRequest?.job ?? null;
  const [resumePin, setResumePin] = useState("");
  const [resumeError, setResumeError] = useState("");
  const actionPending = useRef(false);
  const [cancellation, setCancellation] = useState<(
    { kind: "download"; job: DownloadJob } | { kind: "map"; job: MapJob }
  ) & { privacy: PrivacyContext; notificationRequestId?: string; notificationPhase?: DownloadNotificationAction["phase"] } | null>(null);
  const [shareBusy, setShareBusy] = useState<string | null>(null);
  async function shareBook(book: Book) {
    if (shareBusy || settings.readOnly || settings.privateMode) return;
    setShareBusy(book.installationId);
    try {
      await shareArchive(book.id, book.installationId);
    } catch (error) {
      onError(error);
    } finally {
      setShareBusy(null);
    }
  }
  const [cleanup, setCleanup] = useState<CleanupRequest | null>(null);
  const cleanupRef = useRef(cleanup);
  cleanupRef.current = cleanup;
  const [cleanupBusy, setCleanupBusy] = useState(false);
  const [cleanupError, setCleanupError] = useState("");
  const cleanupPending = useRef(false);
  const booksRef = useRef(books);
  booksRef.current = books;
  const localBlocked = settings.privateMode || settings.readOnly;
  const localBlockedRef = useRef(localBlocked);
  localBlockedRef.current = localBlocked;
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const jobsRef = useRef(jobs);
  jobsRef.current = jobs;
  const seenJobs = useRef(new globalThis.Map<string, number>());
  const seenJobsCount = useRef(0);
  const notificationInteractiveRef = useRef(notificationInteractive);
  notificationInteractiveRef.current = notificationInteractive;
  const notificationOwner = useRef<{ requestId: string; stage: "running" | "confirming" } | null>(null);
  function finishNotificationAction(requestId: string, complete: boolean) {
    if (notificationOwner.current?.requestId !== requestId) return;
    notificationOwner.current = null;
    onFinishNotificationAction(requestId, complete);
  }
  useEffect(() => () => {
    const owner = notificationOwner.current;
    if (owner?.stage === "confirming") {
      notificationOwner.current = null;
      onFinishNotificationAction(owner.requestId, false);
    }
  }, [onFinishNotificationAction]);
  useEffect(() => {
    const owner = notificationOwner.current;
    if (owner?.stage !== "confirming") return;
    const privacy = capturePrivacyContext();
    const revoked = !!privacy && (privacy.privateMode || privacy.readOnly ||
      privacy.epoch !== notificationAction?.privacyEpoch);
    if (!notificationInteractive || revoked || notificationAction?.requestId !== owner.requestId) {
      setCancellation(null);
      finishNotificationAction(owner.requestId, revoked);
    }
  }, [notificationInteractive, notificationInteractionRevision, notificationAction]);
  const mapJobRef = useRef(mapJob);
  mapJobRef.current = mapJob;
  const configuration = mapDownloadConfiguration(settings, books);
  const configurationRef = useRef(configuration);
  configurationRef.current = configuration;
  const load = useCallback(async () => {
    const id = ++request.current;
    const privacy = capturePrivacyContext();
    // Whether anything is still changing, so the poll can slow down when it is
    // not. The freshly fetched payload decides, not a ref that lags a render.
    let live = false;
    try {
      const [downloads, map, notice, edition] = await Promise.allSettled([
        api<DownloadJob[]>("downloads"),
        api<MapJob | null>("map_status"),
        api<{ path?: string | null; downloadPath?: string | null } | null>("transfer_history_notice"),
        api<EditionStatus>("edition_status"),
      ]);
      if (!alive.current || id !== request.current || !privacy || !isPrivacyContextCurrent(privacy)) return;
      setHistoryNotice(
        notice.status === "fulfilled" && notice.value ? notice.value.path || "" : "",
      );
      setDownloadNotice(
        notice.status === "fulfilled" && notice.value ? notice.value.downloadPath || "" : "",
      );
      if (edition.status === "fulfilled") {
        const payload = JSON.stringify(edition.value);
        if (payload !== previousPayload.current.editions) {
          previousPayload.current.editions = payload;
          setEditions(edition.value);
        }
        const revision = JSON.stringify(edition.value.transactions.map((transaction) =>
          [transaction.id, transaction.revision, transaction.rolledBackInstallationId]));
        if (previousEditionRevision.current && previousEditionRevision.current !== revision) onRefresh();
        previousEditionRevision.current = revision;
        setEditionError("");
        live = edition.value.intents.some((intent) => !["committed", "cancelled"].includes(intent.status));
      } else {
        setEditionError(errorText(edition.reason));
      }
      if (map.status === "fulfilled") {
        const next = map.value;
        const signature = next
          ? JSON.stringify([
              next.id,
              ["completed", "complete"].includes(next.status),
              next.importState,
              next.imports?.map((entry) => [entry.receipt, entry.state]),
            ])
          : "";
        const meaningful =
          next &&
          (["completed", "complete"].includes(next.status) ||
            next.importState != null);
        if (meaningful && signature !== previousMap.current) onRefresh();
        previousMap.current = signature;
        // An idle poll usually repeats the previous payload; storing it would
        // re-render every row and rescan the library for nothing.
        const payload = JSON.stringify(next);
        if (payload !== previousPayload.current.map) {
          previousPayload.current.map = payload;
          setMapJob(next);
        }
        setMapError("");
        live = live || (!!next &&
          (!terminalStatus(next.status) || ["queued", "verifying", "ready"].includes(next.importState || "")));
      } else {
        setMapError(errorText(map.reason));
      }
      if (downloads.status === "rejected") throw downloads.reason;
      const values = downloads.value;
      let imported = false;
      for (const job of values) {
        if (
          ["completed", "complete", "seeding"].includes(job.status) &&
          previous.current.get(job.id) !== job.status + (job.importState || "")
        )
          imported = true;
        previous.current.set(job.id, job.status + (job.importState || ""));
      }
      const payload = JSON.stringify(values);
      if (payload !== previousPayload.current.jobs) {
        previousPayload.current.jobs = payload;
        setJobs(values);
      }
      setError("");
      if (imported) onRefresh();
      live = live || values.some((job) =>
        !terminalStatus(job.status) || job.seeding === true ||
        ["queued", "verifying", "ready"].includes(job.importState || ""));
      return live;
    } catch (e) {
      if (alive.current && id === request.current && privacy && isPrivacyContextCurrent(privacy)) setError(errorText(e));
    } finally {
      if (alive.current && id === request.current && privacy && isPrivacyContextCurrent(privacy)) setLoading(false);
    }
  }, [onRefresh]);
  async function loadRetainedFiles() {
    const id = ++retainedRequest.current;
    const privacy = capturePrivacyContext();
    if (!privacy || !alive.current) return;
    setRetainedLoading(true);
    setRetainedError("");
    try {
      const values = await api<DownloadJob[]>("retained_download_files");
      if (!alive.current || id !== retainedRequest.current || !isPrivacyContextCurrent(privacy)) return;
      setRetainedFiles(values);
    } catch (value) {
      if (alive.current && id === retainedRequest.current && isPrivacyContextCurrent(privacy)) setRetainedError(errorText(value));
    } finally {
      if (alive.current && id === retainedRequest.current && isPrivacyContextCurrent(privacy)) setRetainedLoading(false);
    }
  }
  useEffect(() => {
    alive.current = true;
    let timer: number;
    let stopped = false;
    let ticking = false;
    async function tick() {
      if (stopped || ticking || document.visibilityState === "hidden") return;
      ticking = true;
      // Only a transfer, import or edition in flight needs the fast poll.
      const live = await load();
      ticking = false;
      if (!stopped && !document.hidden)
        timer = window.setTimeout(tick, live ? ACTIVE_POLL_MS : IDLE_POLL_MS);
    }
    // Returning to the window must not wait out an idle interval.
    function onVisibility() {
      if (stopped) return;
      window.clearTimeout(timer);
      if (document.visibilityState !== "hidden") void tick();
    }
    document.addEventListener("visibilitychange", onVisibility);
    void tick();
    return () => {
      stopped = true;
      alive.current = false;
      request.current++;
      retainedRequest.current++;
      document.removeEventListener("visibilitychange", onVisibility);
      window.clearTimeout(timer);
    };
  }, [load]);
  useEffect(() => {
    setResumeRequest(null);
    setResumePin("");
    setResumeError("");
    setCleanup(null);
    setCleanupError("");
    retainedRequest.current++;
    setRetainedFiles([]);
    setRetainedError("");
    setRetainedLoading(false);
    previousPayload.current.editions = "";
    setEditions({ intents: [], transactions: [] });
    setEditionAuthorizations({});
    setEditionError("");
    setEditionActionError("");
    setOriginalOutcome(null);
  }, [settings.privateMode, settings.readOnly]);
  const focusedJob = focusTarget?.kind === "map"
    ? mapJob?.id === focusTarget.id ? mapJob : null
    : focusTarget ? jobs.find((job) => job.id === focusTarget.id) || null : null;
  // `readyDownloadBooks` reads only publication state and the library, so these
  // signatures are its complete input. Keying the memo on them keeps progress
  // ticks from rescanning the whole library once per job.
  const readySignature = JSON.stringify(jobs.map((job) => [
    job.id, job.status, job.error, job.importError, job.importState,
    job.imports?.map((entry) => [entry.state, entry.paths]),
  ]));
  const readyBooksByJob = useMemo(() => {
    const index = new globalThis.Map<string, Book[]>();
    for (const job of jobsRef.current) index.set(job.id, readyDownloadBooks(job, books));
    return index;
  }, [readySignature, books]);
  // Downloads the user resumed or retried here count as started again.
  const restartedJobs = useRef(new globalThis.Map<string, number>());
  // One newest-first list: download jobs and the map job, by when each started.
  const orderedEntries = useMemo(() => {
    const seen = seenJobs.current;
    const entries: ({ kind: "job"; id: string; createdAt?: string; job: DownloadJob } |
      { kind: "map"; id: string; createdAt?: string })[] =
      jobs.map((job) => ({ kind: "job", id: job.id, createdAt: job.createdAt, job }));
    if (mapJob) entries.push({ kind: "map", id: `map:${mapJob.id}`, createdAt: mapJob.createdAt });
    const present = new globalThis.Set(entries.map((entry) => entry.id));
    if (seen.size > present.size) {
      for (const id of seen.keys()) if (!present.has(id)) seen.delete(id);
    }
    for (const entry of entries) if (!seen.has(entry.id)) seen.set(entry.id, ++seenJobsCount.current);
    return rankJobs(entries, seen, restartedJobs.current);
  }, [jobs, mapJob]);
  const mapReadySignature = JSON.stringify(mapJob && [
    mapJob.status, mapJob.error, mapJob.importError, mapJob.importState,
    mapJob.imports?.map((entry) => [entry.state, entry.paths]),
  ]);
  const mapBooks = useMemo(
    () => (mapJobRef.current ? readyDownloadBooks(mapJobRef.current, books) : []),
    [mapReadySignature, books],
  );
  // Edition rows only ask whether an installation is still in the library.
  const libraryIndex = useMemo(() => {
    const installations = new globalThis.Set<string>();
    const identities = new globalThis.Set<string>();
    for (const book of books) {
      installations.add(book.installationId);
      identities.add(installationKey(book.id, book.installationId));
    }
    return { installations, identities };
  }, [books]);
  const mapTitle = mapBooks[0]?.title || (mapJob ? t("Map: {region}", { region: mapJob.regionId }) : "");
  const mapProgress = mapJob && mapJob.total > 0
    ? Math.min(100, Math.max(0, mapJob.written / mapJob.total * 100))
    : null;
  const mapCheckingImport = !!mapJob && ["queued", "verifying", "ready"].includes(mapJob.importState || "");
  const mapCanCleanup = !!mapJob && (terminalStatus(mapJob.status) || mapJob.status === "paused");
  function downloadStatus(job: DownloadJob | MapJob, ready: boolean) {
    if ("shared" in job && job.shared) {
      if (job.error) return jobStatus("failed");
      if (job.status === "creating" || job.status === "checking" || job.status === "queued")
        return t("Preparing");
      return job.seeding ? t("Sharing") : t("Share stopped");
    }
    const decision = downloadStatusDecision(job, ready, parseDiskSpaceWarning(job.error) !== null);
    switch (decision.kind) {
      case "confirmation": return t("Confirmation required");
      case "cancelled": return jobStatus("cancelled");
      case "paused": return decision.interrupted ? t("Paused — interrupted; resume when ready") : jobStatus("paused");
      case "verifying": return t("Verifying…");
      case "failed": return jobStatus("failed");
      case "seeding": return jobStatus("seeding");
      case "completed": return t("Completed");
      case "unavailable": return t("Unavailable");
      case "preparing": return t("Preparing");
      case "status": return jobStatus(decision.status);
    }
  }
  useEffect(() => {
    if (focusedTarget.current.target !== focusTarget) {
      focusedTarget.current = { target: focusTarget, handled: false };
    }
    if (!focusTarget || !focusedJob || focusedTarget.current.handled || !focusedCard.current) return;
    focusedTarget.current.handled = true;
    focusedCard.current.focus({ preventScroll: true });
    focusedCard.current.scrollIntoView({ block: "start", behavior: "auto" });
  }, [focusTarget, focusedJob]);
  useEffect(() => {
    if (!notificationAction || !notificationInteractive || loading || working || actionPending.current ||
        cancellation || resumeRequest || cleanup || notificationOwner.current) return;
    const privacy = capturePrivacyContext();
    if (!privacy) return;
    if (privacy.privateMode || privacy.readOnly || privacy.epoch !== notificationAction.privacyEpoch) {
      onFinishNotificationAction(notificationAction.requestId, true);
      return;
    }
    if (privacy.generation !== notificationAction.privacyGeneration || !isPrivacyContextCurrent(privacy) ||
        !onBeginNotificationAction(notificationAction.requestId)) return;
    const requestId = notificationAction.requestId;
    notificationOwner.current = { requestId, stage: "running" };
    void (async () => {
      try {
        // Polling UI state is not action authority. Re-read the acquisition after
        // resumed admission, then match the native authenticated state phase.
        const latest = await api<DownloadJob[]>("downloads");
        if (!alive.current || !notificationInteractiveRef.current || !isPrivacyContextCurrent(privacy)) {
          finishNotificationAction(requestId, false);
          return;
        }
        jobsRef.current = latest;
        previousPayload.current.jobs = JSON.stringify(latest);
        setJobs(latest);
        const job = latest.find((entry) => entry.id === notificationAction.id);
        const requested = notificationAction.action;
        const eligible = job && downloadNotificationPhase(job) === notificationAction.phase &&
          (requested === "pause" ? notificationAction.phase === "active" :
            requested === "resume" ? notificationAction.phase === "paused" :
              requested === "retry" ? notificationAction.phase === "failed" :
                notificationAction.phase === "active" || notificationAction.phase === "paused");
        if (!eligible || !job) { finishNotificationAction(requestId, true); return; }
        const outcome = await action(job, requested === "pause" ? "pause_download" :
          requested === "cancel" ? "cancel_download" : "resume_download",
          { notificationRequestId: requestId, notificationPhase: notificationAction.phase, resumePrivacy: privacy });
        if (outcome !== "confirmation") finishNotificationAction(requestId, outcome === "complete");
      } catch (error) {
        const current = alive.current && notificationInteractiveRef.current && isPrivacyContextCurrent(privacy);
        if (current) onError(error);
        finishNotificationAction(requestId, false);
      }
    })();
  }, [notificationAction, notificationInteractive, notificationInteractionRevision, loading, working, jobs,
      cancellation, resumeRequest, cleanup, onBeginNotificationAction, onFinishNotificationAction]);
  function closeResume() {
    setResumeRequest(null);
    setResumePin("");
    setResumeError("");
  }
  async function action(job: DownloadJob, command: string, options: { pin?: string; receipt?: string; cancellationPrivacy?: PrivacyContext; resumePrivacy?: PrivacyContext; notificationRequestId?: string; notificationPhase?: DownloadNotificationAction["phase"] } = {}): Promise<"complete" | "confirmation" | "deferred"> {
    const { pin } = options;
    const context = options.resumePrivacy ?? options.cancellationPrivacy ?? capturePrivacyContext();
    if (
      actionPending.current ||
      cleanupPending.current ||
      settingsRef.current.readOnly ||
      !context || context.readOnly || !isPrivacyContextCurrent(context) ||
      (job.transport === "localswarm" && (localBlockedRef.current || context.privateMode)) ||
      (!!options.notificationRequestId && (context.privateMode || !notificationInteractiveRef.current))
    ) return "deferred";
    if (command === "resume_download" && job.transport === "localswarm" && job.requiresPin && pin === undefined) {
      if (!context || !isPrivacyContextCurrent(context)) return "deferred";
      setResumePin("");
      setResumeError("");
      setResumeRequest({ job, privacy: context });
      return "confirmation";
    }
    if (command === "cancel_download") {
      if (!context || !alive.current || !isPrivacyContextCurrent(context)) return "deferred";
      const latest = jobsRef.current.find((entry) => entry.id === job.id);
      if (!latest || latest.destination !== job.destination || terminalStatus(latest.status)) return "complete";
      if (!options.cancellationPrivacy) {
        setCancellation({ kind: "download", job, privacy: context, notificationRequestId: options.notificationRequestId,
          notificationPhase: options.notificationPhase });
        if (options.notificationRequestId && notificationOwner.current?.requestId === options.notificationRequestId)
          notificationOwner.current.stage = "confirming";
        return "confirmation";
      }
    }
    const spaceWarning = command === "resume_download" ? parseDiskSpaceWarning(job.error) : null;
    const target = targetRef.current;
    const currentConfiguration = configurationRef.current;
    const current = () => alive.current && !settingsRef.current.readOnly &&
      isPrivacyContextCurrent(context) &&
      (job.transport !== "localswarm" || !localBlockedRef.current) &&
      (!options.notificationRequestId || notificationInteractiveRef.current);
    if (!current()) return "deferred";
    actionPending.current = true;
    setWorking(job.id);
    try {
      if (options.notificationRequestId && options.cancellationPrivacy) {
        const latest = await api<DownloadJob[]>("downloads");
        if (!current()) return "deferred";
        const currentJob = latest.find((entry) => entry.id === job.id);
        if (!currentJob || currentJob.destination !== job.destination ||
            downloadNotificationPhase(currentJob) !== options.notificationPhase) return "complete";
        jobsRef.current = latest;
        previousPayload.current.jobs = JSON.stringify(latest);
        setJobs(latest);
      }
      if (spaceWarning) {
        if (!await confirmDiskSpaceWarning(spaceWarning)) return current() ? "complete" : "deferred";
        if (!current()) return "deferred";
        const latest = jobsRef.current.find((entry) => entry.id === job.id);
        if (!latest || targetRef.current !== target || configurationRef.current !== currentConfiguration ||
          latest.destination !== job.destination || latest.status !== job.status ||
          parseDiskSpaceWarning(latest.error)?.acknowledgement !== spaceWarning.acknowledgement) return "complete";
      }
      await api(command, command === "retry_import"
        ? { receipt: options.receipt }
        : { id: job.id, ...(pin !== undefined ? { pin } : {}), ...(spaceWarning ? { diskSpaceAcknowledgement: spaceWarning.acknowledgement } : {}),
            ...(command === "resume_download" && job.transport === "localswarm"
              ? { admissionGeneration: job.admissionGeneration, privacyGeneration: context?.generation } : {}) });
      // Record completion even if the view retires while the accepted command
      // returns: native ack retries must never submit the same command again.
      if (options.notificationRequestId) finishNotificationAction(options.notificationRequestId, true);
      if (command === "resume_download") restartedJobs.current.set(job.id, Date.now());
      if (current()) {
        if (pin !== undefined) closeResume();
        onRefresh();
        await load();
      }
    } catch (e) {
      if (current()) {
        if (pin !== undefined) setResumeError(errorText(e));
        else onError(e);
      }
      return current() ? "complete" : "deferred";
    } finally {
      actionPending.current = false;
      if (alive.current) setWorking(null);
    }
    return "complete";
  }
  function closeCancellation() {
    if (cancellation?.notificationRequestId) finishNotificationAction(cancellation.notificationRequestId, true);
    setCancellation(null);
  }
  async function start(e: React.FormEvent) {
    e.preventDefault();
    const privacy = capturePrivacyContext();
    if (!source.trim() || startPending.current || localBlocked ||
      !privacy || privacy.privateMode || privacy.readOnly) return;
    const revision = sourceRevision.current;
    const url = source.trim();
    startPending.current = true;
    setStarting(true);
    setStartError("");
    const current = () => alive.current && isPrivacyContextCurrent(privacy);
    try {
      const job = await api<DownloadJob>("start_download", {
        url,
        directory: settings.downloadDirectory || undefined,
      });
      const present = current();
      onDownload({ kind: "download", id: job.id }, { privacy, focus: present });
      if (!present) return;
      if (sourceRevision.current === revision) setSource("");
      await load();
    } catch (e) {
      if (current()) {
        // Shown beside the form; recorded, not painted a second time. Declining
        // the low-storage prompt is the user's own choice, not a failure.
        onError(e, "blocking", false);
        if (!isUserCancelledError(e)) setStartError(errorText(e));
      }
    } finally {
      startPending.current = false;
      if (alive.current) setStarting(false);
    }
  }
  async function seed(job: DownloadJob) {
    if (settings.privateMode) return;
    const enabled = !job.seeding;
    if (
      enabled &&
      !(await confirmAction(
        t(
          "Seed “{name}” to other BitTorrent peers? This uploads archive data and uses your network connection until you stop seeding.",
          { name: job.fileName },
        ),
        { confirmLabel: t("Seed archive") },
      ))
    )
      return;
    setWorking(job.id);
    try {
      await api("torrent_seeding", { id: job.id, enabled });
      await load();
    } catch (e) {
      onError(e);
    } finally {
      setWorking(null);
    }
  }
  async function copyMagnet(job: DownloadJob) {
    if (!job.magnet) return;
    try {
      await navigator.clipboard.writeText(job.magnet);
      if (alive.current) setCopiedMagnet(job.id);
    } catch (e) {
      if (alive.current) onError(e);
    }
  }
  async function saveTorrent(job: DownloadJob) {
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.readOnly) return;
    try {
      const bytes = await api<string>("torrent_metadata", { id: job.id });
      if (!alive.current) return;
      await saveBytes(`${job.fileName}.torrent`, bytes, "application/x-bittorrent", privacy);
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(e);
    }
  }
  async function mapAction(receipt?: string, cancellationPrivacy?: PrivacyContext, cancelledJobId?: string) {
    const currentJob = mapJobRef.current;
    const privacy = cancellationPrivacy ?? capturePrivacyContext();
    if (!alive.current || settingsRef.current.readOnly || mapPending.current || cleanupPending.current || mapWorking ||
      !currentJob || !privacy || !isPrivacyContextCurrent(privacy)) return;
    if (!receipt) {
      if (terminalStatus(currentJob.status)) return;
      if (!cancellationPrivacy) {
        setCancellation({ kind: "map", job: currentJob, privacy });
        return;
      }
      if (currentJob.id !== cancelledJobId) return;
    }
    mapPending.current = true;
    setMapWorking(true);
    try {
      await api(
        receipt ? "retry_import" : "map_cancel",
        receipt ? { receipt } : {},
      );
      if (alive.current && isPrivacyContextCurrent(privacy)) await load();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(value);
    } finally {
      mapPending.current = false;
      if (alive.current) setMapWorking(false);
    }
  }
  function openCleanup(target: CleanupTarget) {
    const privacy = capturePrivacyContext();
    if (!alive.current || cleanupPending.current || actionPending.current || mapPending.current ||
      !privacy || privacy.readOnly || settingsRef.current.readOnly || !isPrivacyContextCurrent(privacy) ||
      (target.kind === "download" && target.job.transport === "localswarm" && localBlockedRef.current)) return;
    if (target.kind === "map") {
      if (mapJobRef.current?.id !== target.job.id ||
        !(terminalStatus(target.job.status) || target.job.status === "paused")) return;
    } else {
      const latest = (target.retained ? retainedFilesRef.current : jobsRef.current).find((job) => job.id === target.job.id);
      if (!latest || latest.destination !== target.job.destination ||
        !(terminalStatus(latest.status) || latest.status === "seeding")) return;
    }
    setCleanupError("");
    setCleanup({ ...target, privacy, books: cleanupDownloadBooks(target.job, booksRef.current).map((book) => ({ ...book })),
      removedInstallations: new Set() });
  }
  async function confirmCleanup(deleteFiles: boolean) {
    const retained = cleanup;
    // A kept map row is cleanup-only: hiding it would strand its file with no
    // remaining surface to delete it. Other retained rows can be forgotten.
    const removeBlocked = retained && (retained.kind === "map"
      ? retained.job.canRemoveFromList !== true
      : retained.retained === true && retained.job.transport === "map");
    if (!retained || cleanupPending.current || actionPending.current || mapPending.current ||
      (!deleteFiles && removeBlocked)) return;
    const nativeKind = retained.kind === "map" || retained.job.transport === "map" ? "map" : "download";
    const current = () => {
      if (!alive.current || cleanupRef.current !== retained || settingsRef.current.readOnly ||
        !isPrivacyContextCurrent(retained.privacy)) return false;
      let currentBooks: Book[];
      if (retained.kind === "map") {
        const latest = mapJobRef.current;
        if (!latest || latest.id !== retained.job.id || latest.path !== retained.job.path ||
          latest.status !== retained.job.status || !(terminalStatus(latest.status) || latest.status === "paused")) return false;
        currentBooks = cleanupDownloadBooks(latest, booksRef.current);
      } else {
        const latest = (retained.retained ? retainedFilesRef.current : jobsRef.current).find((job) => job.id === retained.job.id);
        if (!latest || latest.destination !== retained.job.destination || latest.status !== retained.job.status ||
          !(terminalStatus(latest.status) || latest.status === "seeding") ||
          (latest.transport === "localswarm" && localBlockedRef.current)) return false;
        currentBooks = cleanupDownloadBooks(latest, booksRef.current);
      }
      if (!deleteFiles) return true;
      const expected = retained.books.filter((book) => !retained.removedInstallations.has(book.installationId));
      const remaining = currentBooks.filter((book) => !retained.removedInstallations.has(book.installationId));
      return remaining.length === expected.length && remaining.every((book) => expected.some((captured) =>
        captured.id === book.id && captured.installationId === book.installationId && captured.path === book.path));
    };
    if (!current()) {
      setCleanupError(t("This download changed. Close this dialog and try again."));
      return;
    }
    cleanupPending.current = true;
    if (retained.kind === "map") { mapPending.current = true; setMapWorking(true); }
    else { actionPending.current = true; setWorking(retained.job.id); }
    setCleanupBusy(true);
    setCleanupError("");
    try {
      if (deleteFiles) {
        await api("prepare_download_cleanup", {
          kind: nativeKind, id: retained.job.id, privacyGeneration: retained.privacy.generation,
          installations: retained.books
            .filter((book) => !retained.removedInstallations.has(book.installationId))
            .map((book) => ({ id: book.id, installationId: book.installationId, path: book.path })),
        });
        if (!current()) throw new Error(t("This download changed. Close this dialog and try again."));
        // Removals stay sequential: each one revalidates that the download and
        // library still match the dialog before the next file is deleted. The
        // library refresh happens once after the batch instead of per removal.
        for (const book of retained.books) {
          if (retained.removedInstallations.has(book.installationId)) continue;
          if (!current()) throw new Error(t("This download changed. Close this dialog and try again."));
          await api("remove_book", {
            id: book.id, installationId: book.installationId, deleteFile: true,
            privacyGeneration: retained.privacy.generation,
            cleanupSource: { kind: nativeKind, id: retained.job.id },
          });
          retained.removedInstallations.add(book.installationId);
        }
      }
      if (!current()) throw new Error(t("This download changed. Close this dialog and try again."));
      await api(nativeKind === "map"
        ? deleteFiles ? "map_delete_download" : "map_remove_history"
        : deleteFiles ? "delete_download_files"
        : retained.kind === "download" && retained.retained ? "forget_download_record"
        : "remove_download_history", {
        ...(nativeKind === "map" && deleteFiles ? { jobId: retained.job.id } : { id: retained.job.id }),
        privacyGeneration: retained.privacy.generation,
      });
      if (!alive.current || !isPrivacyContextCurrent(retained.privacy)) return;
      const target = targetRef.current;
      if (target?.kind === retained.kind && target.id === retained.job.id) onDismissTarget(target);
      setCleanup(null);
      onRefresh();
      await load();
      if (retainedFilesOpen.current) await loadRetainedFiles();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(retained.privacy)) {
        // The dialog shows the failure; it is recorded, not painted twice.
        setCleanupError(errorText(value));
        onError(value, "blocking", false);
        onRefresh();
        await load();
        if (retainedFilesOpen.current) await loadRetainedFiles();
      }
    } finally {
      cleanupPending.current = false;
      if (retained.kind === "map") mapPending.current = false;
      else actionPending.current = false;
      if (alive.current) { setCleanupBusy(false); setMapWorking(false); setWorking(null); }
    }
  }
  async function changeMapState(command: "map_pause" | "map_resume") {
    const job = mapJobRef.current;
    const privacy = capturePrivacyContext();
    if (!job || !privacy || privacy.readOnly || settingsRef.current.readOnly ||
      mapPending.current || cleanupPending.current || !isPrivacyContextCurrent(privacy) ||
      (command === "map_resume" && (!job.resumable || localBlockedRef.current))) return;
    mapPending.current = true;
    setMapWorking(true);
    try {
      const next = await api<MapJob>(command, { jobId: job.id, privacyGeneration: privacy.generation });
      if (!alive.current || !isPrivacyContextCurrent(privacy)) return;
      if (mapJobRef.current?.id === job.id) setMapJob(next);
      await load();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(value);
    } finally {
      mapPending.current = false;
      if (alive.current) setMapWorking(false);
    }
  }
  async function forgetMapRecovery(path: string) {
    const job = mapJobRef.current;
    const privacy = capturePrivacyContext();
    if (!alive.current || !job || !privacy || privacy.privateMode || privacy.readOnly || localBlockedRef.current ||
      mapPending.current || cleanupPending.current || !isPrivacyContextCurrent(privacy) ||
      !job.recoveryPaths?.includes(path)) return;
    mapPending.current = true;
    setMapWorking(true);
    try {
      // Only the warning is dismissed; the file stays where it is.
      await api("map_forget_recovery", { path, privacyGeneration: privacy.generation });
      if (alive.current && isPrivacyContextCurrent(privacy)) await load();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        onError(value);
        await load();
      }
    } finally {
      mapPending.current = false;
      if (alive.current) setMapWorking(false);
    }
  }
  async function continueMap() {
    const target = targetRef.current;
    const retained = target?.kind === "map" ? target.request : undefined;
    const warning = mapJobRef.current && parseDiskSpaceWarning(mapJobRef.current.error);
    if (!alive.current || mapPending.current || localBlockedRef.current || !retained || !warning ||
      target?.id !== mapJobRef.current?.id || !isPrivacyContextCurrent(retained.privacy) ||
      retained.privacy.privateMode || retained.privacy.readOnly) return;
    if (retained.configuration !== configurationRef.current) {
      setMapError(t("Download settings changed. Start this map download again."));
      return;
    }
    mapPending.current = true;
    setMapWorking(true);
    setMapError("");
    try {
      if (!await confirmDiskSpaceWarning(warning)) return;
      const latest = mapJobRef.current;
      if (!alive.current || !latest || localBlockedRef.current || targetRef.current !== target ||
        target?.id !== latest.id || !isPrivacyContextCurrent(retained.privacy) ||
        parseDiskSpaceWarning(latest.error)?.acknowledgement !== warning.acknowledgement) return;
      if (retained.configuration !== configurationRef.current) {
        setMapError(t("Download settings changed. Start this map download again."));
        return;
      }
      const next = await api<MapJob>(latest.resumable ? "map_resume" : "map_start", {
        ...(latest.resumable ? { jobId: latest.id } : {
          regionId: retained.regionId,
          language: retained.language,
          decisionToken: retained.decisionToken,
          replaceExisting: retained.replaceExisting,
        }),
        privacyGeneration: retained.privacy.generation,
        diskSpaceAcknowledgement: warning.acknowledgement,
      });
      const present = alive.current && targetRef.current === target && isPrivacyContextCurrent(retained.privacy);
      onDownload({ kind: "map", id: next.id, request: retained }, { privacy: retained.privacy, focus: present });
      if (!present) return;
      setMapJob(next);
      await load();
    } catch (value) {
      // One owner: the page-level map notice is cleared by the next status
      // poll and sits above a card that may be scrolled away.
      if (alive.current && targetRef.current === target && isPrivacyContextCurrent(retained.privacy))
        onError(value);
    } finally {
      mapPending.current = false;
      if (alive.current) setMapWorking(false);
    }
  }
  async function editionAction(intent: EditionIntent, action: "review" | "replace" | "cancel") {
    const privacy = capturePrivacyContext();
    if (editionPending.current || localBlockedRef.current || !privacy || privacy.privateMode || privacy.readOnly) return;
    const installed = booksRef.current.find((book) => book.id === intent.target.id &&
      book.installationId === intent.target.installationId);
    if (action !== "cancel" && !installed) {
      setEditionActionError(t("The installed archive changed. Check for editions again."));
      return;
    }
    const authorization = editionAuthorizations[intent.id];
    if (action === "replace" && (intent.needsAuthorization || !authorization || !isPrivacyContextCurrent(authorization.privacy))) return;
    editionPending.current = true;
    setEditionWorking(intent.id);
    setEditionActionError("");
    try {
      if (action === "review") {
        const preparation = await api<EditionPreparation>("edition_prepare", {
          intentId: intent.id,
          bookId: intent.target.id,
          installationId: intent.target.installationId,
          privacyGeneration: privacy.generation,
        });
        if (!alive.current || !isPrivacyContextCurrent(privacy) || localBlockedRef.current) return;
        if (preparation.intentId !== intent.id) throw new Error(t("This edition decision changed. Check for editions again."));
        setEditionAuthorizations({ [intent.id]: { token: preparation.token, privacy } });
        if (!intent.downloadId && !intent.verified) {
          const target = await editionDownload(preparation.token);
          onDownload(target, { privacy, focus: alive.current && isPrivacyContextCurrent(privacy) });
        }
      } else if (action === "replace") {
        await api<EditionTransaction>("edition_commit", {
          intentId: intent.id,
          decisionToken: authorization.token,
          installationId: intent.target.installationId,
          privacyGeneration: privacy.generation,
        });
        onRefresh();
      } else {
        const confirmed = await confirmAction(t("Cancel this edition update? Your original archive remains installed and its files are kept."), {
          confirmLabel: t("Cancel edition update"),
        });
        if (!confirmed || !alive.current || !isPrivacyContextCurrent(privacy) || localBlockedRef.current) return;
        await api("edition_cancel", { intentId: intent.id, privacyGeneration: privacy.generation });
      }
      if (alive.current && isPrivacyContextCurrent(privacy)) await load();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        setEditionAuthorizations((previous) => {
          const next = { ...previous };
          delete next[intent.id];
          return next;
        });
        // A Store durability error may already have committed the transition.
        onRefresh();
        await load();
        setEditionActionError(errorText(value));
      }
    } finally {
      editionPending.current = false;
      if (alive.current) setEditionWorking(null);
    }
  }
  async function rollbackEdition(transaction: EditionTransaction) {
    const privacy = capturePrivacyContext();
    if (editionPending.current || localBlockedRef.current || !privacy || privacy.privateMode || privacy.readOnly ||
      !booksRef.current.some((book) => book.installationId === transaction.replacement.installationId)) return;
    editionPending.current = true;
    setEditionWorking(transaction.id);
    setEditionActionError("");
    try {
      const confirmed = await confirmAction(t("Roll back to the retained original archive? Reading references move back to that edition; both archive files are kept."), {
        confirmLabel: t("Roll back edition"),
      });
      if (!confirmed || !alive.current || !isPrivacyContextCurrent(privacy) || localBlockedRef.current) return;
      await api<EditionTransaction>("edition_rollback", {
        transactionId: transaction.id,
        expectedInstallationId: transaction.replacement.installationId,
        privacyGeneration: privacy.generation,
      });
      onRefresh();
      if (alive.current && isPrivacyContextCurrent(privacy)) await load();
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        onRefresh();
        await load();
        // Shown in this transaction's card, where the action was, not at the
        // top of a page that may be scrolled away; recorded, not painted twice.
        onError(value, "blocking", false);
        originalOutcomeFocus.current = true;
        setOriginalOutcome({ id: transaction.id, failed: true, message: errorText(value) });
      }
    } finally {
      editionPending.current = false;
      if (alive.current) setEditionWorking(null);
    }
  }
  function rollbackFromCard(transaction: EditionTransaction) {
    setOriginalOutcome(null);
    originalOutcomeFocus.current = false;
    void rollbackEdition(transaction);
  }
  async function discardOriginal(transaction: EditionTransaction) {
    const privacy = capturePrivacyContext();
    if (editionPending.current || localBlockedRef.current || !privacy || privacy.privateMode || privacy.readOnly ||
      transaction.originalDiscarded || transaction.rolledBackInstallationId !== null) return;
    const original = transaction.original;
    // Name the exact file: an original outside Kiwix's storage is deleted too.
    const file = original.sourceReference
      ? pathFileName(original.sourceReference.displayName)
      : isIOS ? pathFileName(original.path) : displayPath(original.path);
    // A provider-backed original is not Kiwix's to delete: only its hold is released.
    const external = original.sourceReference != null;
    // delete_book removes the whole part set of a split archive, named by its first part.
    const split = /\.zimaa$/i.test(original.path);
    editionPending.current = true;
    setEditionWorking(transaction.id);
    setOriginalOutcome(null);
    originalOutcomeFocus.current = false;
    try {
      const confirmed = await confirmAction(external
        ? t("Stop keeping the retained original “{title}” ({file})? Kiwix cannot delete it: the file stays where it is, in the app or storage that provides it. This edition can then no longer be rolled back.", { title: original.title, file })
        : (settingsRef.current.trashDeletedFiles
          ? t("Move the retained original “{title}” ({file}) to the system trash? This edition can then no longer be rolled back.", { title: original.title, file })
          : t("Permanently delete the retained original “{title}” ({file})? This cannot be undone, and this edition can then no longer be rolled back.", { title: original.title, file })) +
          (split ? "\n\n" + t("This is the first part of a split archive: every part of it is removed.") : ""), {
        confirmLabel: external ? t("Stop keeping original") : t("Delete retained original"),
        danger: !external,
      });
      if (!confirmed || !alive.current || !isPrivacyContextCurrent(privacy) || localBlockedRef.current) return;
      const result = await api<EditionOriginalDiscarded>("edition_discard_original", {
        transactionId: transaction.id,
        privacyGeneration: privacy.generation,
      });
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        onRefresh();
        await load();
        // Set after the refreshed status, so the outcome renders with the
        // card's final buttons and its focus decision sees them.
        originalOutcomeFocus.current = true;
        setOriginalOutcome({
          id: transaction.id,
          failed: false,
          message: result.originalFile === "deleted"
            ? t("The retained original was deleted. This edition can no longer be rolled back.")
            : result.originalFile === "trashed"
              ? t("The retained original was moved to the system trash. This edition can no longer be rolled back.")
              : result.originalFile === "missing"
                ? t("The retained original was already gone, so nothing was deleted. This edition can no longer be rolled back.")
                : t("Kiwix no longer keeps the retained original. Its file was not deleted; remove it in the app or storage that provides it if you no longer need it."),
        });
      }
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        // Shown in the transaction's card; recorded, not painted twice.
        onError(value, "blocking", false);
        onRefresh();
        await load();
        // The refreshed status can remove the button (already discarded).
        originalOutcomeFocus.current = true;
        setOriginalOutcome({ id: transaction.id, failed: true, message: errorText(value) });
      }
    } finally {
      editionPending.current = false;
      if (alive.current) setEditionWorking(null);
    }
  }
  useEffect(() => {
    // The action button leaves once the original is gone; keep keyboard and
    // screen-reader focus on the outcome instead of dropping it to the page.
    // Outcomes are set after the refreshed status, so this render already has
    // the card's final buttons: decide once.
    if (!originalOutcome || !originalOutcomeFocus.current) return;
    originalOutcomeFocus.current = false;
    const active = document.activeElement;
    if (active && active !== document.body && active.isConnected) return;
    document.getElementById(`edition-original-${originalOutcome.id}`)?.focus();
  }, [originalOutcome]);
  function eta(seconds: number) {
    const remaining = remainingTime(seconds);
    return remaining.unit === "seconds"
      ? t("{count} seconds remaining", { count: remaining.count })
      : remaining.unit === "minutes"
        ? t("{count} minutes remaining", { count: remaining.count })
        : t("{count} hours remaining", { count: remaining.count });
  }
  // The map download card takes its place in the newest-first list.
  const mapCard = mapJob ? (
        <SwipeRow key={`${mapJob.id}:${mapCanCleanup ? "cleanup" : "cancel"}`}
          className="swipe-map-job"
          actionLabel={mapCanCleanup ? t("Delete download") : t("Cancel map download")}
          disabled={mapWorking || settings.readOnly}
          onAction={() => { if (mapCanCleanup) openCleanup({ kind: "map", job: mapJob }); else void mapAction(); }}>
        <article className="panel download-card map-job"
          ref={focusTarget?.kind === "map" && focusTarget.id === mapJob.id ? focusedCard : undefined}
          tabIndex={-1}
          aria-label={mapTitle}>
          <div className="section-heading">
            <h2><bdi>{mapTitle}</bdi></h2>
            <span className="download-status" role="status" aria-live="polite">
              {downloadStatus(mapJob, mapBooks.length > 0)}
            </span>
            {!mapCanCleanup && (
              <button
                className="icon-button"
                disabled={mapWorking || settings.readOnly}
                onClick={() => mapAction()}
                aria-label={t("Cancel map download")}
              >
                <X size={18} />
              </button>
            )}
            {mapCanCleanup && (
              <button disabled={mapWorking || settings.readOnly} onClick={() => openCleanup({ kind: "map", job: mapJob })}>
                {t("Delete download")}
              </button>
            )}
            {mapJob.resumable && ["failed", "paused", "cancelled"].includes(mapJob.status) && (
              <button disabled={mapWorking || localBlocked} onClick={() => void changeMapState("map_resume")}>
                <Play size={16} />{t("Resume")}
              </button>
            )}
            {!mapCanCleanup && (
              <button disabled={mapWorking || settings.readOnly} onClick={() => void changeMapState("map_pause")}>
                <Pause size={16} />{t("Pause")}
              </button>
            )}
          </div>
          {mapBooks.map((book) => (
            <button className="primary download-open" key={book.installationId}
              onClick={() => onOpen(
                { bookId: book.id, path: "", title: book.title, scroll: 0 },
                { kind: "map", id: mapJob.id },
              )}>
              <FolderOpen size={28} />{t("Open {title}", { title: book.title })}
            </button>
          ))}
          {!mapBooks.length && !mapJob.error && !mapJob.importError &&
            (!terminalStatus(mapJob.status) || mapCheckingImport) && (
            <div className="download-progress">
              <span>{t("Progress")}</span>
              <progress aria-label={t("Download progress for {name}", { name: mapTitle })}
                max={mapCheckingImport || mapProgress === null ? undefined : 100}
                value={mapCheckingImport ? undefined : mapProgress ?? undefined} />
              {!mapCheckingImport && mapProgress !== null && <strong>{t("{percent}%", { percent: formatNumber(Math.floor(mapProgress)) })}</strong>}
            </div>
          )}
          {!mapBooks.length && !terminalStatus(mapJob.status) && <MapBlockProgress job={mapJob} />}
          {!mapBooks.length && <MapTransferRate job={mapJob} />}
          <details className="download-details">
            <summary>{t("Details for {title}", { title: mapTitle })}</summary>
          <p className="muted">{languageName(mapJob.language)}</p>
          <div className="download-stats">
            <span>
              {t("Current block")}: {mapJob.incomingTotal
                ? t("{downloaded} of {total}", {
                    downloaded: formatBytes(mapJob.incoming),
                    total: formatBytes(mapJob.incomingTotal),
                  })
                : formatBytes(mapJob.incoming)}
            </span>
            <span>
              {t("Local assembly")}: {mapJob.total
                ? t("{written} of {total}", {
                    written: formatBytes(mapJob.written),
                    total: formatBytes(mapJob.total),
                  })
                : formatBytes(mapJob.written)}
            </span>
          </div>
          {!isIOS && mapJob.path && <p className="file-path muted">{displayPath(mapJob.path)}</p>}
          </details>
          {mapJob.error && (
            <div role="alert">
              <p className="inline-error">
                {parseDiskSpaceWarning(mapJob.error)
                  ? t("Confirmation required")
                  : errorText(mapJob.error)}
              </p>
              {parseDiskSpaceWarning(mapJob.error) && (
                focusTarget?.kind === "map" && focusTarget.id === mapJob.id && focusTarget.request ? (
                  <button disabled={mapWorking || localBlocked} onClick={() => void continueMap()}>
                    {mapWorking ? t("Starting…") : t("Review disk space and continue")}
                  </button>
                ) : (
                  <button onClick={onMaps}>{t("Maps")}</button>
                )
              )}
            </div>
          )}
          <MapRecoveryPaths job={mapJob} disabled={mapWorking || localBlocked}
            onDismiss={(path) => void forgetMapRecovery(path)} />
          {["failed", "paused", "cancelled"].includes(mapJob.status) && !mapJob.resumable && !parseDiskSpaceWarning(mapJob.error) && (
            <div>
              <p className="muted">{t("This download has no resumable checkpoint. Start it again from Maps.")}</p>
              <button onClick={onMaps}>{t("Maps")}</button>
            </div>
          )}
          {mapJob.importError && (
            <p className="inline-error" role="alert">
              {t("Map import failed: {error}", { error: errorText(mapJob.importError) })}
            </p>
          )}
          {mapJob.imports
            ?.filter((entry) => entry.state === "failed")
            .map((entry) => (
              <button
                key={entry.receipt}
                disabled={mapWorking || settings.readOnly}
                onClick={() => mapAction(entry.receipt)}
              >
                <RefreshCw size={16} />
                {t("Retry archive import")}
              </button>
            ))}
        </article>
        </SwipeRow>
  ) : null;
  return (
    <section className="page downloads-page">
      <PageHeader title={t("Downloads")} />
      {historyNotice && (
        <p className="muted download-history-notice" role="status">
          {t("Saved download history could not be read, so the download list was reset. Unfinished downloads must be started again. The original file was kept as {name}.", { name: pathFileName(historyNotice) })}
        </p>
      )}
      {downloadNotice && (
        <p className="muted download-history-notice" role="status">
          {t("Saved download history could not be read, so the download list was reset. Unfinished downloads must be started again. The original file was kept as {name}.", { name: pathFileName(downloadNotice) })}
        </p>
      )}
      {focusTarget && !focusedJob && (
        <article className="panel download-pending" aria-busy={loading}>
          <p role="status">{loading ? t("Starting download…") : t("Unavailable")}</p>
          {!loading && <button onClick={() => onDismissTarget(focusTarget)}>{t("Close")}</button>}
        </article>
      )}
      {error && <ErrorNotice message={error} onRetry={load} />}
      {mapError && <ErrorNotice message={mapError} onRetry={load} />}
      {editionError && <ErrorNotice message={editionError} onRetry={load} />}
      {editionActionError && (
        <div role="alert">
          <p className="inline-error">{editionActionError}</p>
          <p className="muted">{t("If the catalog or installed archive changed, refresh Archive editions in Settings and prepare the edition again. Your original files are kept.")}</p>
        </div>
      )}
      {editions.intents.filter((intent) => !intent.downloadId || !jobs.some((job) => job.id === intent.downloadId)).map((intent) => (
        <article className="panel download-card" key={intent.id} aria-label={intent.selection.entry.title}>
          <EditionDownloadActions intent={intent}
            authorized={!!editionAuthorizations[intent.id] && isPrivacyContextCurrent(editionAuthorizations[intent.id].privacy)}
            blocked={localBlocked} busy={editionWorking !== null}
            targetCurrent={libraryIndex.identities.has(installationKey(intent.target.id, intent.target.installationId))}
            onReview={() => void editionAction(intent, "review")}
            onReplace={() => void editionAction(intent, "replace")}
            onCancel={() => void editionAction(intent, "cancel")} />
        </article>
      ))}
      {loading ? (
        <Loading />
      ) : !jobs.length && !mapJob && !editions.intents.length && !editions.transactions.length ? (
        <p className="muted download-empty" role="status">{t("No downloads")}</p>
      ) : (
        <div className="download-list">
          {orderedEntries.map((entry) => {
            if (entry.kind === "map") return <Fragment key={entry.id}>{mapCard}</Fragment>;
            const job = entry.job;
            const edition = editions.intents.find((intent) => intent.downloadId === job.id);
            const editionProtected = !!edition && !["committed", "cancelled"].includes(edition.status);
            const finished =
              terminalStatus(job.status) || job.status === "seeding";
            // Local verification owns every byte; show a full bar while the
            // hash/publish pass runs instead of a stalled 100% download.
            const progress = job.status === "verifying"
              ? 100
              : job.total
                ? Math.min(100, Math.max(0, (job.downloaded / job.total) * 100))
                : null;
            const completed = ["completed", "complete", "seeding"].includes(
              job.status,
            );
            const spaceWarning = parseDiskSpaceWarning(job.error);
            const readyBooks = readyBooksByJob.get(job.id) ?? [];
            const shared = job.shared === true;
            const ready = readyBooks.length > 0 || shared;
            const checkingImport = !edition?.verified && ["queued", "verifying", "ready"].includes(job.importState || "");
            // Rate, remaining time and byte counts belong on the card itself;
            // the details twisty keeps the technical transport fields.
            const live = !finished && !ready && !job.error && !job.importError
              ? downloadLiveSummary(job)
              : null;
            // A paused or failed transfer keeps the bytes it saved: show how
            // much Resume or Retry continues from, without a rate or ETA.
            const kept = !live && !ready && !job.importError && !checkingImport &&
              ["paused", "failed", "error"].includes(job.status) && job.downloaded > 0;
            const retryImports = edition?.status === "cancelled" ? [] : job.imports?.filter((entry) =>
              entry.state === "failed" || entry.state === "cancelled",
            ) || [];
            const hasActions = retryImports.length > 0 || !finished ||
              ["paused", "failed", "error"].includes(job.status);
            return (
              <SwipeRow key={`${job.id}:${finished ? "cleanup" : "cancel"}`}
                actionLabel={editionProtected ? t("Cancel edition update") : finished
                  ? shared ? t("Remove share") : t("Delete download")
                  : t("Cancel {name}", { name: job.fileName })}
                disabled={working !== null || editionWorking !== null || settings.readOnly || !!editionError ||
                  (editionProtected && localBlocked) || (job.transport === "localswarm" && localBlocked)}
                onAction={() => {
                  if (editionProtected) void editionAction(edition, "cancel");
                  else if (finished) openCleanup({ kind: "download", job });
                  else void action(job, "cancel_download");
                }}>
              <article className="download-card"
                ref={focusTarget?.kind === "download" && focusTarget.id === job.id ? focusedCard : undefined}
                tabIndex={-1}
                aria-label={job.fileName}>
                <div className="download-main">
                  <div className="section-heading">
                    <h2><bdi>{job.fileName}</bdi></h2>
                    <span className="download-status" role="status" aria-live="polite">
                      {editionProtected && edition.verified ? t("Ready — choose Replace") : downloadStatus(job, ready)}
                    </span>
                  </div>
                  {edition && (
                    <EditionDownloadActions intent={edition}
                      authorized={!!editionAuthorizations[edition.id] && isPrivacyContextCurrent(editionAuthorizations[edition.id].privacy)}
                      blocked={localBlocked} busy={editionWorking !== null}
                      targetCurrent={libraryIndex.identities.has(installationKey(edition.target.id, edition.target.installationId))}
                      onReview={() => void editionAction(edition, "review")}
                      onReplace={() => void editionAction(edition, "replace")}
                      onCancel={() => void editionAction(edition, "cancel")} />
                  )}
                  {readyBooks.map((book) => (
                    <Fragment key={book.installationId}>
                      <button className="primary download-open"
                        onClick={() => onOpen(
                          { bookId: book.id, path: "", title: book.title, scroll: 0 },
                          { kind: "download", id: job.id },
                        )}>
                        <FolderOpen size={28} />{t("Open {title}", { title: book.title })}
                      </button>
                      {canShareSystem && (
                        <button className="download-open" type="button"
                          disabled={settings.readOnly || settings.privateMode || shareBusy === book.installationId}
                          onClick={() => void shareBook(book)}>
                          <Share2 size={24} />{t("Share")}
                        </button>
                      )}
                    </Fragment>
                  ))}
                  {!ready && !job.error && !job.importError && (!finished || checkingImport) && (
                  <div className="download-progress">
                    <span>{t("Progress")}</span>
                    {progress === null || checkingImport || job.status === "creating" ? (
                      <progress
                        aria-label={t("Download progress for {name}", {
                          name: job.fileName,
                        })}
                      />
                    ) : (
                      <progress
                        value={job.status === "verifying" ? job.total || 1 : job.downloaded}
                        max={job.total || 1}
                        aria-label={t("Download progress for {name}", {
                          name: job.fileName,
                        })}
                      />
                    )}
                    {!checkingImport && job.status !== "creating" && <strong>
                      {progress === null
                        ? t("Size unknown")
                        : t("{percent}%", {
                            percent: formatNumber(Math.floor(progress)),
                          })}
                    </strong>}
                  </div>
                  )}
                  {live && (
                    <div className="download-live">
                      <span className="download-live-bytes">
                        {job.total
                          ? t("{downloaded} of {total}", {
                              downloaded: formatBytes(job.downloaded),
                              total: formatBytes(job.total),
                            })
                          : formatBytes(job.downloaded)}
                      </span>
                      {live.kind === "active" && (
                        <>
                          <span className="download-live-rate">
                            {t("{speed}/s", { speed: formatBytes(live.bytesPerSecond) })}
                          </span>
                          {live.etaSeconds != null && (
                            <span className="download-live-eta">{eta(live.etaSeconds)}</span>
                          )}
                        </>
                      )}
                      {live.kind === "paused" && (
                        <span className="download-live-note">{t("Paused")}</span>
                      )}
                      {live.kind === "verifying" && (
                        <span className="download-live-note">{t("Verifying…")}</span>
                      )}
                    </div>
                  )}
                  {kept && (
                    <>
                      {progress !== null && (
                        <div className="download-progress">
                          <span>{t("Progress")}</span>
                          <progress
                            value={job.downloaded}
                            max={job.total || 1}
                            aria-label={t("Download progress for {name}", {
                              name: job.fileName,
                            })}
                          />
                          <strong>
                            {t("{percent}%", { percent: formatNumber(Math.floor(progress)) })}
                          </strong>
                        </div>
                      )}
                      <div className="download-live">
                        <span className="download-live-bytes">
                          {job.total
                            ? t("{downloaded} of {total}", {
                                downloaded: formatBytes(job.downloaded),
                                total: formatBytes(job.total),
                              })
                            : formatBytes(job.downloaded)}
                        </span>
                      </div>
                    </>
                  )}
                  {job.transport === "torrent" && (
                    <div className="download-live download-torrent-summary">
                      <span className="badge">{t("BitTorrent")}</span>
                      <span>{t("Uploaded: {size}", { size: formatBytes(job.uploaded || 0) })}</span>
                      {job.seeding && (
                        <span className="download-live-rate">
                          {t("Upload: {speed}/s", { speed: formatBytes(job.uploadSpeed || 0) })}
                        </span>
                      )}
                    </div>
                  )}
                  {completed && job.transport === "torrent" && (
                    <div className="download-actions">
                      <button
                        onClick={() => seed(job)}
                        disabled={working !== null || settings.readOnly || settings.privateMode}
                      >
                        <Upload size={16} aria-hidden="true" />
                        {shared
                          ? job.seeding ? t("Stop sharing") : t("Share again")
                          : job.seeding ? t("Stop seeding") : t("Seed archive")}
                      </button>
                    </div>
                  )}
                  <details className="download-details">
                    <summary>{t("Details for {title}", { title: job.fileName })}</summary>
                  <div className="download-stats">
                    {job.transport === "localswarm" && <span>{t("LocalSwarm")}</span>}
                    {job.transport === "localswarm" && job.route && (
                      <span>{t("Route: {route}", { route: job.route })}</span>
                    )}
                  </div>
                  {shared && job.magnet && (
                    <div className="download-share">
                      <label>
                        <span>{t("Magnet link")}</span>
                        <textarea
                          className="share-magnet"
                          dir="ltr"
                          readOnly
                          rows={3}
                          value={job.magnet}
                          onFocus={(event) => event.currentTarget.select()}
                        />
                      </label>
                      <div className="download-actions">
                        <button
                          type="button"
                          disabled={working !== null || settings.readOnly}
                          onClick={() => void copyMagnet(job)}
                        >
                          {copiedMagnet === job.id ? <Check size={16} /> : <Copy size={16} />}
                          {copiedMagnet === job.id ? t("Copied") : t("Copy magnet link")}
                        </button>
                        <button
                          type="button"
                          disabled={working !== null || settings.readOnly}
                          onClick={() => void saveTorrent(job)}
                        >
                          <Upload size={16} />{t("Save .torrent file")}
                        </button>
                      </div>
                    </div>
                  )}
                  {!isIOS && (
                    <>
                      <p className="file-path muted" dir="ltr">
                        <FolderOpen size={13} aria-hidden="true" />
                        <bdi>{displayPath(job.destination)}</bdi>
                      </p>
                      {job.completedPaths?.map((path) => (
                        <p key={path} className="file-path muted" dir="ltr">
                          <bdi>{displayPath(path)}</bdi>
                        </p>
                      ))}
                    </>
                  )}
                    <div className="download-actions">
                  {finished && !editionProtected && !editionError && (
                      <button
                        disabled={working !== null || settings.readOnly || (job.transport === "localswarm" && localBlocked)}
                        onClick={() => openCleanup({ kind: "download", job })}
                      >
                        {shared ? t("Remove share") : t("Delete download")}
                      </button>
                    )}
                </div>
                  </details>
                  {job.error && (
                    <p className="inline-error" role="alert">
                      {spaceWarning
                        ? t("This download would leave less than 1 GB of free space in {directory}.", { directory: isIOS ? t("Kiwix storage") : spaceWarning.directory })
                        : errorText(job.error)}
                      {spaceWarning && <>{" "}{t("Projected free space: {size}", { size: formatBytes(spaceWarning.projectedFreeBytes) })}</>}
                    </p>
                  )}
                  {job.historyWarning && <p role="status">{errorText(job.historyWarning)}</p>}
                  {job.importError && (
                    <p className="inline-error" role="alert">
                      {t("Archive import failed: {error}", {
                        error: errorText(job.importError),
                      })}
                    </p>
                  )}
                </div>
                {hasActions && <div className="download-actions">
                  {retryImports.map((entry) => (
                      <button
                        key={entry.receipt}
                        disabled={working !== null || settings.readOnly || (job.transport === "localswarm" && localBlocked)}
                        onClick={() => void action(job, "retry_import", { receipt: entry.receipt })}
                      >
                        <RefreshCw size={16} />
                        {t("Retry archive import")}
                      </button>
                    ))}
                  {job.status === "paused" ||
                  job.status === "failed" ||
                  job.status === "error" ? (
                    <button
                      onClick={() => action(job, "resume_download")}
                      disabled={working !== null || settings.readOnly || (job.transport === "localswarm" && localBlocked)}
                      aria-label={spaceWarning ? t("Review low storage warning") : t("Resume {name}", { name: job.fileName })}
                    >
                      {job.status === "paused" ? (
                        <Play size={16} />
                      ) : (
                        <RefreshCw size={16} />
                      )}{" "}
                      {spaceWarning ? t("Review low storage warning") : job.status === "paused" ? t("Resume") : t("Retry")}
                    </button>
                  ) : (
                    !finished && (
                      <button
                        onClick={() => action(job, "pause_download")}
                        disabled={working !== null || settings.readOnly || (job.transport === "localswarm" && localBlocked)}
                      >
                        <Pause size={16} />
                        {t("Pause")}
                      </button>
                    )
                  )}
                  {!finished && !editionProtected && (
                    <button
                      className="icon-button"
                      onClick={() => action(job, "cancel_download")}
                      disabled={working !== null || settings.readOnly || (job.transport === "localswarm" && localBlocked)}
                      aria-label={t("Cancel {name}", { name: job.fileName })}
                    >
                      <X size={18} />
                    </button>
                  )}
                  </div>}
              </article>
              </SwipeRow>
            );
          })}
        </div>
      )}
      <details className="secondary-controls" onToggle={(event) => {
        retainedFilesOpen.current = event.currentTarget.open;
        if (event.currentTarget.open) void loadRetainedFiles();
      }}>
        <summary>{t("Kept download files")}</summary>
        <div>
          <p className="muted">{t("Downloads removed from the list can still have files on this device. Delete kept files here to free storage.")}</p>
          {retainedLoading ? <Loading /> : retainedError ? (
            <ErrorNotice message={retainedError} onRetry={() => void loadRetainedFiles()} />
          ) : !retainedFiles.length ? (
            <p className="muted" role="status">{t("No kept download files")}</p>
          ) : (
            <div className="download-list">
              {retainedFiles.map((job) => (
                <article className="download-card" key={job.id}>
                  <div className="section-heading">
                    <h2><bdi>{job.fileName}</bdi></h2>
                    <button disabled={working !== null || settings.readOnly ||
                      (job.transport === "localswarm" && localBlocked)}
                      onClick={() => openCleanup({ kind: "download", job, retained: true })}>
                      {t("Delete download and files")}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </div>
      </details>
      {/* Retained originals are history: below today's downloads. */}
      {editions.transactions.length > 0 && (
        <section className="panel" aria-label={t("Retained original editions")}>
          <h2>{t("Retained original editions")}</h2>
          <p className="muted">{t("Replacement keeps your original archive. Rollback verifies the retained original and moves reading references back without discarding newer notes or history.")}</p>
          {editions.transactions.map((transaction) => (
            <article className="edition-result" key={transaction.id}>
              <h3><bdi>{transaction.original.title}</bdi></h3>
              <p>{t("Original: {date}", { date: transaction.original.date || t("Date unknown") })}</p>
              <p>{t("Replacement: {date}", { date: transaction.replacement.date || t("Date unknown") })}</p>
              {transaction.rolledBackInstallationId
                ? <p role="status">{t("Rolled back. Both archive files were kept.")}</p>
                : transaction.originalDiscarded
                  ? <p className="muted">{t("The retained original is no longer kept, so this edition cannot be rolled back.")}</p>
                  : (
                    <div className="download-actions">
                      <button disabled={localBlocked || editionWorking !== null ||
                        !libraryIndex.installations.has(transaction.replacement.installationId)}
                        onClick={() => rollbackFromCard(transaction)}>
                        {t("Roll back to original edition")}
                      </button>
                      <button className={transaction.original.sourceReference != null ? undefined : "danger"}
                        disabled={localBlocked || editionWorking !== null}
                        onClick={() => void discardOriginal(transaction)}>
                        {transaction.original.sourceReference != null
                          ? t("Stop keeping original")
                          : t("Delete retained original")}
                      </button>
                    </div>
                  )}
              {originalOutcome?.id === transaction.id && (
                <p className={originalOutcome.failed ? "inline-error" : "success-text"}
                  id={`edition-original-${transaction.id}`} tabIndex={-1}
                  role={originalOutcome.failed ? "alert" : "status"}>
                  {originalOutcome.message}
                </p>
              )}
            </article>
          ))}
        </section>
      )}
      <form className="panel direct-download" onSubmit={start}>
        <h2>{t("Download from a URL or magnet link")}</h2>
        <div className="compact-toolbar direct-download-controls">
          <input
            aria-label={t("Download from a URL or magnet link")}
            value={source}
            onChange={(e) => { sourceRevision.current++; setSource(e.target.value); }}
            placeholder={t("HTTPS archive, .torrent URL or magnet link")}
            dir="ltr"
            required
          />
          <button
            className="primary"
            disabled={starting || localBlocked}
          >
            <ArrowDownToLine size={16} />
            {starting ? t("Starting…") : t("Start download")}
          </button>
        </div>
        <p className="muted">
          {t(
            "BitTorrent shares pieces with peers while downloading. Continued seeding after completion requires your permission.",
          )}
        </p>
      </form>
      {startError && <p className="inline-error" role="alert">{startError}</p>}
      {cleanup && !settings.readOnly && (
        <DownloadCleanupDialog
          name={cleanup.kind === "map" ? t("Map: {region}", { region: cleanup.job.regionId }) : cleanup.job.fileName}
          busy={cleanupBusy} error={cleanupError}
          archiveNames={cleanup.books.map((book) => book.title)}
          removeFromListDisabled={cleanup.kind === "map"
            ? cleanup.job.canRemoveFromList !== true
            : cleanup.retained === true && cleanup.job.transport === "map"}
          removeFromListLabel={cleanup.kind === "download" && cleanup.retained && cleanup.job.transport !== "map"
            ? t("Forget this download (keep the file)")
            : undefined}
          description={cleanup.kind === "download" && cleanup.retained
            ? cleanup.job.transport === "map"
              ? t("These download files were kept on this device. Delete them to free storage.")
              : t("These download files were kept on this device. Forget this download to remove it from the list and keep the file, or delete the files to free storage.")
            : cleanup.kind === "map" && cleanup.job.canRemoveFromList !== true
              ? t("Resume this download or delete its files before removing it from the list.")
              : undefined}
          onCancel={() => { if (!cleanupPending.current) { setCleanup(null); setCleanupError(""); } }}
          onRemoveFromList={() => void confirmCleanup(false)}
          onDeleteFiles={() => void confirmCleanup(true)}
        />
      )}
      {cancellation && !settings.readOnly && (
        <Modal title={cancellation.kind === "map" ? t("Cancel map download") : t("Cancel {name}", { name: cancellation.job.fileName })}
          presentation="confirmation"
          onClose={closeCancellation}>
          <p>{t("Cancel the download of “{name}”?", {
            name: cancellation.kind === "map" ? cancellation.job.regionId : cancellation.job.fileName,
          })}</p>
          <div className="modal-actions">
            <button type="button" onClick={closeCancellation}>{t("Close")}</button>
            <button type="button" className="primary" disabled={working !== null || mapWorking}
              onClick={(event) => {
                if (!event.nativeEvent.isTrusted) return;
                const retained = cancellation;
                setCancellation(null);
                if (retained.kind === "map") void mapAction(undefined, retained.privacy, retained.job.id);
                else {
                  if (retained.notificationRequestId && notificationOwner.current?.requestId === retained.notificationRequestId)
                    notificationOwner.current.stage = "running";
                  void action(retained.job, "cancel_download", { cancellationPrivacy: retained.privacy,
                    notificationRequestId: retained.notificationRequestId, notificationPhase: retained.notificationPhase }).then((outcome) => {
                    if (retained.notificationRequestId)
                      finishNotificationAction(retained.notificationRequestId, outcome === "complete");
                  });
                }
              }}>
              {cancellation.kind === "map" ? t("Cancel map download") : t("Cancel {name}", { name: cancellation.job.fileName })}
            </button>
          </div>
        </Modal>
      )}
      {resumeRequest && resumeJob && !localBlocked && (
        <Modal title={t("Resume “{name}”", { name: resumeJob.fileName })} onClose={closeResume}>
          <form onSubmit={(event) => {
            event.preventDefault();
            const pin = resumePin;
            setResumePin("");
            void action(resumeJob, "resume_download", { pin, resumePrivacy: resumeRequest.privacy });
          }}>
            <p>{t("This protected share needs its PIN again. PINs are never saved between sessions.")}</p>
            <label className="field">
              {t("Share PIN")}
              <input type="password" autoComplete="off" spellCheck={false} required value={resumePin} disabled={working === resumeJob.id} onChange={(event) => setResumePin(event.target.value)} />
            </label>
            {resumeError && <p className="inline-error" role="alert">{resumeError}</p>}
            <div className="modal-actions">
              <button type="button" onClick={closeResume}>{t("Cancel")}</button>
              <button type="submit" className="primary" disabled={working === resumeJob.id || !resumePin}>
                <Play size={16} />{t("Resume")}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </section>
  );
}
