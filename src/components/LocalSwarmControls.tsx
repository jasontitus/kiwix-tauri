import { QRCodeSVG } from "qrcode.react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ArrowDownToLine, BookOpen, Radio, Square, Upload } from "lucide-react";
import { api, capturePrivacyContext, isAndroid, isIOS, isPrivacyContextCurrent, type PrivacyContext } from "../api";
import { formatBytes, t } from "../i18n";
import type { Book, DownloadAdmission, DownloadJob, DownloadTarget, LocalSwarmConnection, LocalSwarmInspection, LocalSwarmPeer, LocalSwarmStatus, Settings, SharedLibraryInventory, TabLocation } from "../types";
import { confirmAction, errorText, Modal } from "./UI";
import { isSuspendedError } from "../error-visibility";
import { SettingsSaveStatus, useSettingsAutosave } from "./SettingsAutosave";
import { validSharingName } from "../sharing-name";
import { queueSharedLibrary, type SharedLibraryBatchProgress } from "../shared-library-batch";
import "./localswarm-controls.css";

interface Props {
  books: Book[];
  settings: Settings;
  discoveryIntent: number;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
  onOpen: (location: TabLocation) => void;
  onLibrary: () => void;
  onChange: () => void;
  onWebSharing: () => void;
}
const emptyStatus: LocalSwarmStatus = { discovery: false, peers: [], seeds: [], error: null };
/** Mirrors `SHARED_EARLIER_SESSION` in `src-tauri/src/acquisition.rs`. */
const EARLIER_SHARED_DOWNLOAD = "An earlier Nearby download from a previous sharing session is still in Downloads";
/** Raw backend text, not `errorText`, which may rewrite matching messages. */
function isEarlierSharedDownload(error: unknown): boolean {
  return (error instanceof Error ? error.message : String(error)).startsWith(EARLIER_SHARED_DOWNLOAD);
}

export default function LocalSwarmControls({ books, settings, discoveryIntent, onDownload, onOpen, onLibrary, onChange, onWebSharing }: Props) {
  const id = useId();
  const [status, setStatus] = useState<LocalSwarmStatus>(emptyStatus);
  const [error, setError] = useState("");
  const [statusError, setStatusError] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [shareStarting, setShareStarting] = useState(false);
  const [pendingPeerId, setPendingPeerId] = useState<string | null>(null);
  const pendingPeerRef = useRef<string | null>(null);
  const [selectedPeer, setSelectedPeer] = useState<LocalSwarmPeer | null>(null);
  const selectedPeerRef = useRef<LocalSwarmPeer | null>(null);
  const [pin, setPin] = useState("");
  const [inventory, setInventory] = useState<SharedLibraryInventory | null>(null);
  const inventoryRef = useRef<SharedLibraryInventory | null>(null);
  const [selectedSharedBooks, setSelectedSharedBooks] = useState<ReadonlySet<string>>(() => new Set());
  const [sharedBookQuery, setSharedBookQuery] = useState("");
  const [batchProgress, setBatchProgress] = useState<SharedLibraryBatchProgress | null>(null);
  const [batchAdding, setBatchAdding] = useState(false);
  const [batchStopping, setBatchStopping] = useState(false);
  const [batchPreparationError, setBatchPreparationError] = useState("");
  const [batchDownload, setBatchDownload] = useState<{ id: string; privacy: PrivacyContext } | null>(null);
  const batchRun = useRef<{ stopped: boolean } | null>(null);
  const [libraryDownloading, setLibraryDownloading] = useState<string | null>(null);
  const [existingArchiveId, setExistingArchiveId] = useState<string | null>(null);
  const [bookId, setBookId] = useState("");
  const [seedBook, setSeedBook] = useState<Book | null>(null);
  const [nickname, setNickname] = useState(settings.sharingName);
  const nameEdited = useRef(false);
  const preferences = useSettingsAutosave(settings, {
    onRefresh: onChange,
    onError: (value) => setError(errorText(value)),
  });
  const alive = useRef(false);
  const autoDiscovery = useRef(false);
  const operation = useRef(false);
  const operationToken = useRef(0);
  const revision = useRef(0);
  const statusRequest = useRef(0);
  const operationEpoch = useRef(0);
  const stopPending = useRef(false);
  const blocked = settings.privateMode || settings.readOnly;
  const blockedRef = useRef(blocked);
  blockedRef.current = blocked;
  const consumedDiscoveryIntent = useRef(0);
  const availableBooks = books.filter((book) => book.available);
  const existingArchive = books.find((book) => book.id === existingArchiveId);
  const librarySharing = status.librarySharing;
  const nativeDiscoveryReady = !status.nearby?.supported || status.nearby.nativeReady;
  const installedSharedBooks = new Map(books.map((book) => [book.id, book]));
  const downloadableSharedBooks = inventory?.books.filter((book) => !installedSharedBooks.has(book.archiveId ?? "")) ?? [];
  const selectedSharedCount = downloadableSharedBooks.filter((book) => selectedSharedBooks.has(book.id)).length;
  const sharedQuery = sharedBookQuery.trim().toLocaleLowerCase();
  const visibleSharedBooks = inventory?.books.filter((book) => !sharedQuery || `${book.title} ${book.fileName}`.toLocaleLowerCase().includes(sharedQuery)) ?? [];

  useEffect(() => {
    if (!nameEdited.current && settings.sharingName === preferences.draft.sharingName) setNickname(settings.sharingName);
  }, [settings.sharingName, preferences.draft.sharingName]);

  const selectPeer = useCallback((peer: LocalSwarmPeer | null) => {
    selectedPeerRef.current = peer;
    setSelectedPeer(peer);
    inventoryRef.current = null;
    setInventory(null);
    setSelectedSharedBooks(new Set());
    setSharedBookQuery("");
    if (batchRun.current) batchRun.current.stopped = true;
    batchRun.current = null;
    setBatchProgress(null);
    setBatchAdding(false);
    setBatchStopping(false);
    setBatchPreparationError("");
    setBatchDownload(null);
    setExistingArchiveId(null);
    setPin("");
  }, []);

  const load = useCallback(async () => {
    if (document.hidden) return;
    const request = ++statusRequest.current;
    const context = capturePrivacyContext();
    try {
      const value = await api<LocalSwarmStatus>("localswarm_status");
      if (!alive.current || request !== statusRequest.current || !context || !isPrivacyContextCurrent(context)) return;
      const selected = selectedPeerRef.current;
      if (selected && pendingPeerRef.current !== selected.id) {
        const latest = value.peers.find((peer) => peer.id === selected.id)
          ?? (selected.transport === "quic-library"
            ? value.peers.find((peer) => peer.transport === "quic-library" && peer.peerId === selected.peerId)
            : undefined);
        // Discovery presentation may stop without revoking an accepted native
        // connection. Backend receipts retain and enforce its actual authority.
        const changed = latest && (latest.transport !== selected.transport || latest.peerId !== selected.peerId ||
          (latest.transport === "quic" && selected.transport === "quic" && latest.swarmId !== selected.swarmId) ||
          (latest.transport === "quic-library" && selected.transport === "quic-library" &&
            latest.librarySessionId && inventoryRef.current && latest.librarySessionId !== inventoryRef.current.sessionId));
        // A missing mDNS entry is a transient discovery gap, not proof that
        // the accepted QUIC library route died. Only a vanished Bluetooth
        // peer, a vanished single-archive share, or a changed session
        // identity invalidates the open catalog.
        const lost = !latest && (selected.transport === "quic" ||
          (selected.transport === "quic-library" && selected.discovery === "bluetooth"));
        if (lost || changed) {
          revision.current++;
          selectPeer(null);
          setError(t("This share changed or is no longer available. Choose the sender again."));
        }
      }
      setStatus(value);
      setStatusError("");
      setLoading(false);
    } catch (value) {
      if (alive.current && request === statusRequest.current && context && isPrivacyContextCurrent(context)) {
        if (!isSuspendedError(value)) setStatusError(errorText(value));
        setLoading(false);
      }
    }
  }, [selectPeer]);

  useEffect(() => {
    alive.current = true;
    let stopped = false;
    let timer: number | undefined;
    async function tick() {
      if (stopped || document.hidden) return;
      await load();
      if (!stopped && !document.hidden) timer = window.setTimeout(tick, 1200);
    }
    function visibility() {
      window.clearTimeout(timer);
      statusRequest.current++;
      if (document.hidden) {
        if (batchRun.current) batchRun.current.stopped = true;
      } else void tick();
    }
    document.addEventListener("visibilitychange", visibility);
    void tick();
    return () => {
      alive.current = false;
      stopped = true;
      revision.current++;
      statusRequest.current++;
      if (batchRun.current) batchRun.current.stopped = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibility);
      // Leaving this view does not disconnect already accepted consumers/jobs.
    };
  }, [load]);

  useEffect(() => {
    revision.current++;
    operationEpoch.current++;
    statusRequest.current++;
    selectPeer(null);
    setSeedBook(null);
    setError("");
    setStatusError("");
    setStatus(emptyStatus);
    void load();
  }, [settings.privateMode, settings.readOnly, load, selectPeer]);

  async function run(task: (current: () => boolean, privacy: PrivacyContext) => Promise<void>) {
    const context = capturePrivacyContext();
    if (operation.current || stopPending.current || blockedRef.current || !context || context.privateMode || context.readOnly) return;
    const token = ++operationToken.current;
    operation.current = true;
    setBusy(true);
    setError("");
    statusRequest.current++;
    const epoch = operationEpoch.current;
    const current = () => alive.current && !blockedRef.current && epoch === operationEpoch.current && isPrivacyContextCurrent(context);
    try {
      await task(current, context);
      if (current()) await load();
    } catch (value) {
      if (current() && !isSuspendedError(value)) setError(errorText(value));
    } finally {
      if (operationToken.current === token) {
        operation.current = false;
        if (alive.current) setBusy(stopPending.current);
      }
    }
  }

  useEffect(() => {
    // Initialize both discovery transports once. LAN browsing alone does not
    // prove Bluetooth started, and listener setup may finish after first render.
    if (autoDiscovery.current || loading || blocked || stopPending.current || !nativeDiscoveryReady) return;
    autoDiscovery.current = true;
    void api("localswarm_discovery", { enabled: true })
      .then(() => { if (alive.current) void load(); })
      .catch((value) => {
        if (alive.current && !isSuspendedError(value)) setError(errorText(value));
      });
  }, [blocked, loading, nativeDiscoveryReady, load]);

  useEffect(() => {
    if (!discoveryIntent || discoveryIntent === consumedDiscoveryIntent.current) return;
    if (blocked) {
      consumedDiscoveryIntent.current = discoveryIntent;
      return;
    }
    if (loading || !nativeDiscoveryReady) return;
    consumedDiscoveryIntent.current = discoveryIntent;
    autoDiscovery.current = true;
    if (!status.discovery || (status.nearby?.supported && !status.nearby.discovery)) {
      void run(async () => { await api("localswarm_discovery", { enabled: true }); });
    }
  }, [blocked, loading, nativeDiscoveryReady, status.discovery, status.nearby?.discovery, discoveryIntent]);

  function closeReceive() {
    // Closing while a Bluetooth sender is still being joined abandons that
    // connection. Without telling the device, the Wi-Fi join prompt can still
    // appear after the dialog is gone and the phone stays on the sender's
    // network. A list that already loaded keeps its connection, and the
    // device itself refuses to end one that is established (`pendingOnly`):
    // accepted downloads depend on it.
    const peer = selectedPeerRef.current;
    const abandoned = peer?.transport === "quic-library" && peer.discovery === "bluetooth" &&
      pendingPeerRef.current === peer.id && !inventoryRef.current && false ? status.nearby?.consumer?.owner : undefined;
    const finding = !!status.nearby?.discovery;
    revision.current++;
    operationToken.current++;
    pendingPeerRef.current = null;
    setPendingPeerId(null);
    operation.current = false;
    if (alive.current) setBusy(false);
    selectPeer(null);
    setError("");
    if (abandoned) {
      void run(async (current) => {
        await api("nearby_stop", { owner: abandoned, pendingOnly: true });
        // Stopping the connection also stops looking over Bluetooth; the
        // person was finding devices, so keep finding them.
        if (finding && current()) await api("localswarm_discovery", { enabled: true });
      });
    }
  }
  function openExisting(book: Book) {
    if (blockedRef.current || !book.available) return;
    closeReceive();
    onOpen({ bookId: book.id, path: "", title: book.title, scroll: 0 });
  }
  function choosePeer(peer: LocalSwarmPeer) {
    if (blockedRef.current || operation.current || stopPending.current) return;
    revision.current++;
    selectPeer(peer);
    setError("");
    if (peer.transport === "quic-library") browseLibrary(peer);
    else if (!peer.locked) receiveBook(peer);
  }
  function receiveBook(peer: LocalSwarmPeer, suppliedPin?: string) {
    if (peer.transport !== "quic") return;
    const version = ++revision.current;
    const args: LocalSwarmConnection = {
      host: peer.host, port: peer.port, swarmId: peer.swarmId, discoveryId: peer.id,
      ...(suppliedPin ? { pin: suppliedPin } : {}),
    };
    void run(async (current, privacy) => {
      pendingPeerRef.current = peer.id;
      setPendingPeerId(peer.id);
      try {
        const inspected = await api<LocalSwarmInspection>("localswarm_inspect", { ...args, privacyGeneration: privacy.generation });
        if (!current() || revision.current !== version) return;
        if (!inspected.supported) throw new Error(inspected.reason || t("This share is not a supported ZIM archive or split archive."));
        if (inspected.archiveId && books.some((book) => book.id === inspected.archiveId)) {
          selectPeer(null);
          setExistingArchiveId(inspected.archiveId);
          return;
        }
        const job = await api<DownloadJob>("localswarm_download", {
          ...args, host: inspected.connectedHost || args.host,
          directory: settings.downloadDirectory || undefined,
          privacyGeneration: privacy.generation, admissionGeneration: inspected.admissionGeneration,
        }, () => current() && !document.hidden && revision.current === version && selectedPeerRef.current === peer);
        const present = current() && revision.current === version;
        // Track accepted work even when its initiating presentation retired.
        onDownload({ kind: "download", id: job.id }, { privacy, focus: present });
        if (present) closeReceive();
      } catch (value) {
        if (current() && revision.current === version) throw value;
      } finally {
        if (pendingPeerRef.current === peer.id) {
          pendingPeerRef.current = null;
          if (alive.current) setPendingPeerId(null);
        }
      }
    });
  }
  function browseLibrary(peer: LocalSwarmPeer) {
    // choosePeer auto-browses; the modal's Browse/Refresh can fire while the
    // first connect is still joining. A second shared_library_browse would
    // start another nearby_connect, cancelling the pending Wi-Fi join.
    if (pendingPeerRef.current === peer.id) return;
    pendingPeerRef.current = peer.id;
    const version = ++revision.current;
    void run(async (current, privacy) => {
      setPendingPeerId(peer.id);
      const ownerCurrent = () => current() && version === revision.current && selectedPeerRef.current === peer;
      try {
        const value = await api<SharedLibraryInventory>("shared_library_browse", {
          peerId: peer.id, privacyGeneration: privacy.generation,
        }, ownerCurrent);
        if (!ownerCurrent()) return;
        inventoryRef.current = value;
        setInventory(value);
      } catch (value) {
        if (ownerCurrent()) throw value;
      } finally {
        if (pendingPeerRef.current === peer.id) {
          pendingPeerRef.current = null;
          if (alive.current) setPendingPeerId(null);
        }
      }
    });
  }
  function downloadLibrary(bookId: string) {
    const receipt = inventoryRef.current;
    const peer = selectedPeerRef.current;
    if (!receipt || peer?.transport !== "quic-library") return;
    const offered = receipt.books.find((book) => book.id === bookId);
    const installed = books.find((book) => book.id === offered?.archiveId);
    if (installed) {
      if (installed.available) openExisting(installed);
      else { closeReceive(); onLibrary(); }
      return;
    }
    const version = revision.current;
    void run(async (current, privacy) => {
      setLibraryDownloading(bookId);
      try {
        const admissionCurrent = () => current() && !document.hidden && version === revision.current && selectedPeerRef.current === peer && inventoryRef.current === receipt;
        const start = (replaceEarlier: boolean) => api<DownloadJob>("shared_library_download", {
          receiptId: receipt.receiptId, bookId, privacyGeneration: privacy.generation,
          ...(replaceEarlier ? { replaceEarlier: true } : {}),
        }, admissionCurrent);
        let job: DownloadJob;
        try {
          job = await start(false);
        } catch (value) {
          // An unfinished download of this file from an earlier sharing
          // session (for example, the sender restarted) cannot resume, but
          // still owns the destination. Offer to replace it instead of
          // sending the reader to Downloads to delete it first.
          if (!isEarlierSharedDownload(value) || !admissionCurrent()) throw value;
          const replace = await confirmAction(
            t("An earlier download of “{title}” did not finish and cannot be resumed. Replace it with a new download? Its partly downloaded data will be discarded.", { title: offered?.title || bookId }),
            { confirmLabel: t("Replace download") },
          );
          if (!replace || !admissionCurrent()) return;
          job = await start(true);
        }
        onDownload({ kind: "download", id: job.id }, { privacy, focus: current() && version === revision.current });
      } catch (value) {
        if (current() && version === revision.current) throw value;
      } finally {
        if (alive.current) setLibraryDownloading(null);
      }
    });
  }
  function downloadLibraryBatch(selection?: ReadonlySet<string>) {
    const receipt = inventoryRef.current;
    const peer = selectedPeerRef.current;
    if (!receipt || peer?.transport !== "quic-library" || document.hidden) return;
    const requested = selection ? { ...receipt, books: receipt.books.filter((book) => selection.has(book.id)) } : receipt;
    if (requested.books.length === 0) return;
    const installed = books.map(({ id, available }) => ({ id, available }));
    const version = revision.current;
    void run(async (current, privacy) => {
      const token = { stopped: false };
      batchRun.current = token;
      setBatchAdding(true);
      setBatchStopping(false);
      setBatchPreparationError("");
      setBatchProgress({ total: requested.books.length, processed: 0, queued: 0, installed: 0, active: 0, unavailable: 0, duplicates: 0, stopped: false });
      const ownerCurrent = () => current() && version === revision.current && selectedPeerRef.current === peer && inventoryRef.current === receipt && batchRun.current === token;
      const admissionCurrent = () => ownerCurrent() && !token.stopped && !document.hidden;
      let replaceEarlier: boolean | undefined;
      try {
        const activeJobs = await api<DownloadJob[]>("downloads");
        const result = await queueSharedLibrary(requested, installed, activeJobs, {
          isCurrent: admissionCurrent,
          start: async (bookId) => {
            const request = (replaceEarlier: boolean) => api<DownloadJob>("shared_library_download", {
              receiptId: receipt.receiptId, bookId, privacyGeneration: privacy.generation,
              ...(replaceEarlier ? { replaceEarlier: true } : {}),
            }, admissionCurrent);
            try {
              return await request(false);
            } catch (value) {
              // Ask once per batch; the answer covers every affected archive.
              if (!isEarlierSharedDownload(value) || !admissionCurrent()) throw value;
              replaceEarlier ??= await confirmAction(
                t("Some archives have earlier downloads that did not finish and cannot be resumed. Replace them with new downloads? Their partly downloaded data will be discarded."),
                { confirmLabel: t("Replace downloads") },
              );
              if (!replaceEarlier || !admissionCurrent()) throw value;
              return await request(true);
            }
          },
          onQueued: (job) => {
            onDownload({ kind: "download", id: job.id }, { privacy, focus: false });
            if (ownerCurrent()) setBatchDownload({ id: job.id, privacy });
          },
          onProgress: (progress) => { if (ownerCurrent()) setBatchProgress(progress); },
        });
        if (ownerCurrent()) setBatchProgress(result);
      } catch (value) {
        if (ownerCurrent()) {
          setBatchPreparationError(errorText(value));
          setBatchProgress((progress) => progress && { ...progress, stopped: true });
        }
      } finally {
        if (alive.current && batchRun.current === token) {
          setBatchAdding(false);
          setBatchStopping(false);
        }
      }
    });
  }
  function stopAddingLibrary() {
    if (!batchRun.current || !batchAdding) return;
    batchRun.current.stopped = true;
    setBatchStopping(true);
  }
  function viewBatchDownloads() {
    if (batchAdding || !batchDownload || blockedRef.current || !isPrivacyContextCurrent(batchDownload.privacy)) return;
    const accepted = batchDownload;
    closeReceive();
    onDownload({ kind: "download", id: accepted.id }, { privacy: accepted.privacy, focus: true });
  }
  function shareAll(hotspot = false) {
    void run(async (current, privacy) => {
      setShareStarting(true);
      try {
        await api("localswarm_share_all", { privacyGeneration: privacy.generation, ...(hotspot ? { hotspot: true } : {}) });
        if (current()) onChange();
      } finally {
        if (alive.current) setShareStarting(false);
      }
    });
  }
  function shareBook(event: React.FormEvent) {
    event.preventDefault();
    const book = seedBook;
    if (!book || !books.some((entry) => entry.id === book.id && entry.installationId === book.installationId && entry.available)) return;
    setSeedBook(null);
    void run(async () => { await api("localswarm_seed", { bookId: book.id }); });
  }
  function saveNickname(event: React.FormEvent) {
    event.preventDefault();
    if (blockedRef.current || busy || !capturePrivacyContext()) return;
    const name = nickname.trim();
    if (!validSharingName(name)) {
      setError(t("Use a shorter device name without control or direction-override characters."));
      return;
    }
    nameEdited.current = false;
    setNickname(name);
    preferences.change("sharingName", name);
  }
  async function stop() {
    const context = capturePrivacyContext();
    if (stopPending.current || blockedRef.current || !context || context.privateMode || context.readOnly) return;
    stopPending.current = true;
    autoDiscovery.current = true;
    consumedDiscoveryIntent.current = discoveryIntent;
    operationEpoch.current++;
    revision.current++;
    statusRequest.current++;
    setStopping(true);
    setBusy(true);
    selectPeer(null);
    setSeedBook(null);
    setError("");
    try {
      await api("localswarm_stop");
      if (alive.current && isPrivacyContextCurrent(context)) {
        await load();
        if (alive.current && isPrivacyContextCurrent(context)) onChange();
      }
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(context)) setError(errorText(value));
    } finally {
      stopPending.current = false;
      if (alive.current) {
        setStopping(false);
        setBusy(operation.current);
      }
    }
  }

  const groups = new Map<string, LocalSwarmPeer[]>();
  for (const peer of status.peers) {
    const key = peer.transport === "quic-library" ? `library:${peer.peerId}` : peer.swarmId;
    const group = groups.get(key) || [];
    group.push(peer);
    groups.set(key, group);
  }
  // Bluetooth off or not allowed fails after "Share all" and "Find nearby"
  // have already returned, so it is only visible here.
  const publisher = status.nearby?.publisher;
  const publisherError = publisher?.phase === "failed" ? publisher.error : null;
  const finderError = status.nearby?.error;
  const connection = selectedPeer?.transport === "quic-library" && selectedPeer.discovery === "bluetooth" &&
    status.nearby?.consumer?.peerId === selectedPeer.id ? status.nearby.consumer : null;
  const connectionText = connection?.phase === "joining"
    ? t("Joining the sender’s network… Approve the system prompt if asked.")
    : connection?.phase === "connected" ? t("Connected. Loading the shared library…")
    : t("Connecting to the sender… Keep both apps open and approve any system connection prompt.");

  function peerCard(peer: LocalSwarmPeer) {
    return <article className="localswarm-peer" key={peer.id} aria-busy={pendingPeerId === peer.id}>
      <BookOpen size={22} aria-hidden="true" />
      <div className="localswarm-row-copy">
        <strong id={`${id}-peer-${peer.id}`}><bdi>{peer.publisherName || peer.name || t("Nearby device")}</bdi></strong>
        <span className="muted">{peer.transport === "quic-library"
          ? <>{t("Shared library")}{peer.libraryCount != null ? <> · {t("{count} archives", { count: peer.libraryCount })}</> : null}</>
          : <><bdi>{peer.name}</bdi> · {formatBytes(peer.totalBytes)}</>}</span>
      </div>
      <button id={`${id}-choose-${peer.id}`} type="button" aria-labelledby={`${id}-choose-${peer.id} ${id}-peer-${peer.id}`} disabled={blocked || busy} onClick={() => choosePeer(peer)}>{t("Choose")}</button>
    </article>;
  }

  return (
    <section className="localswarm" aria-labelledby={`${id}-title`}>
      <header className="localswarm-heading"><h1 id={`${id}-title`}>{t("Nearby")}</h1></header>
      <section className="localswarm-share-card" aria-labelledby={`${id}-share-title`} aria-busy={shareStarting}>
        <div className="localswarm-toolbar">
          <div className="localswarm-row-copy">
            <h2 id={`${id}-share-title`}>{librarySharing?.running ? t("You’re sharing your library") : t("Share your library")}</h2>
            <span className="muted">{t("On the other device, open Nearby, choose this device, then Download all.")}</span>
          </div>
          {librarySharing?.running
            ? <button type="button" disabled={blocked || busy} onClick={() => void run(async () => { await api("localswarm_stop_library"); })}><Square size={16} aria-hidden="true" />{t("Stop sharing")}</button>
            : <>
                <button className="primary" type="button" disabled={blocked || busy || loading || availableBooks.length === 0} onClick={() => shareAll()}><Upload size={18} aria-hidden="true" />{shareStarting ? t("Starting sharing…") : t("Share all")}</button>
                {isAndroid && <button type="button" title={t("Use when the other device is on a different Wi-Fi network.")} disabled={blocked || busy || loading || availableBooks.length === 0} onClick={() => shareAll(true)}>{t("Use temporary Wi-Fi")}</button>}
              </>}
        </div>
        <p className="localswarm-share-status" role="status">{librarySharing?.running
          ? t("{count} archives · {peers} connected devices · {size} sent", { count: librarySharing.libraryCount, peers: librarySharing.peerCount, size: formatBytes(librarySharing.uploaded) })
          : availableBooks.length === 0 ? t("Add an available archive to your library before sharing.") : t("Your library stays unshared until you choose Share all. Nearby devices can download while sharing is on.")}</p>
        {librarySharing?.running && librarySharing.publisherName && <p className="muted">{t("This device")}: <bdi>{librarySharing.publisherName}</bdi></p>}
        <p className="muted">{t("Keep this app open while sharing. Received downloads are never shared automatically.")}</p>
        {isAndroid && !librarySharing?.running && <p className="muted">{t("On a different Wi-Fi network? Use temporary Wi-Fi.")}</p>}
        {librarySharing?.error && !busy && <p className="inline-error" role="alert">{errorText(librarySharing.error)}</p>}
        {librarySharing?.running && publisherError && <p className="inline-error" role="alert">{t("Devices on other Wi-Fi networks cannot find this share over Bluetooth: {reason} Devices on this Wi-Fi can still find it. To try Bluetooth again, stop sharing and choose Share all.", { reason: errorText(publisherError) })}</p>}
      </section>
      {isAndroid && status.hotspot?.phase === "active" && (
        <section className="localswarm-hotspot" aria-labelledby={`${id}-hotspot-title`}>
          <h2 id={`${id}-hotspot-title`}>{t("Temporary Wi-Fi")}</h2>
          <p className="muted">{t("On the other device, join this network or approve the join prompt.")}</p>
          <p>{t("Network: {ssid}", { ssid: status.hotspot.ssid })}</p>
          <p>{t("Password: {passphrase}", { passphrase: status.hotspot.passphrase })}</p>
          <div className="localswarm-hotspot-qr">
            <QRCodeSVG
              value={status.hotspot.qr}
              size={160}
              level="M"
              marginSize={4}
              bgColor="#ffffff"
              fgColor="#000000"
              role="img"
              title={t("Temporary Wi-Fi QR code")}
              aria-label={t("Temporary Wi-Fi QR code")}
            />
          </div>
        </section>
      )}
      {blocked && <p className="localswarm-notice" role="status">{t("Nearby is unavailable in private or read-only mode. Finding, receiving and sharing are stopped.")}</p>}
      {!selectedPeer && (error || statusError || status.error) && <p className="inline-error" role="alert">{errorText(error || statusError || status.error)}</p>}
      <section className="localswarm-receive" aria-labelledby={`${id}-receive-title`}>
        <div className="localswarm-toolbar">
          <div className="localswarm-row-copy"><h2 id={`${id}-receive-title`}>{t("Receive from a nearby device")}</h2></div>
          <button type="button" disabled={blocked || busy || loading || !nativeDiscoveryReady} onClick={() => {
            autoDiscovery.current = true;
            void run(async () => { await api("localswarm_discovery", { enabled: !status.discovery }); });
          }}>
            {status.discovery ? <Square size={15} aria-hidden="true" /> : <Radio size={17} aria-hidden="true" />}{status.discovery ? t("Stop finding") : t("Find nearby")}
          </button>
        </div>
        <p className="muted">{t("Ask the other person to open Nearby and choose Share all. Then choose their device below.")}</p>
        {isIOS && status.discovery && <p className="muted">{t("If iOS asks, allow Local Network access to find devices on this Wi-Fi.")}</p>}
        {loading && <p className="muted" role="status">{t("Loading…")}</p>}
        {!blocked && finderError && <div className="localswarm-finder-error">
          <p className="inline-error" role="alert">{t("Finding devices over Bluetooth stopped: {reason} Devices on this Wi-Fi can still appear below.", { reason: errorText(finderError) })}</p>
          <button type="button" disabled={busy || loading || !nativeDiscoveryReady} onClick={() => {
            autoDiscovery.current = true;
            void run(async () => { await api("localswarm_discovery", { enabled: true }); });
          }}><Radio size={17} aria-hidden="true" />{t("Try Bluetooth again")}</button>
        </div>}
        {!blocked && status.discovery && <div className="localswarm-peers">
          {groups.size === 0 && <p className="localswarm-empty muted" role="status">{t("Looking for nearby devices… Keep both apps open.")}</p>}
          {[...groups].map(([key, peers]) => {
            const ordered = [...peers].sort((left, right) =>
              (right.transport === "quic-library" ? right.libraryGeneration ?? 0 : 0) -
              (left.transport === "quic-library" ? left.libraryGeneration ?? 0 : 0));
            return <div className="localswarm-peer-group" key={key}>
              {peerCard(ordered[0])}
              {ordered.length > 1 && <details className="localswarm-secondary">
                <summary>{t("{count} other sessions", { count: ordered.length - 1 })}</summary>
                {ordered.slice(1).map(peerCard)}
              </details>}
            </div>;
          })}
        </div>}
      </section>
      {!blocked && status.seeds.length > 0 && <section className="localswarm-seeds" aria-labelledby={`${id}-offers-title`}>
        <h2 id={`${id}-offers-title`}>{t("Books you’re sharing")}</h2>
        {status.seeds.map((seed) => <article className="localswarm-seed" key={seed.id}>
          <div className="localswarm-toolbar"><BookOpen size={21} aria-hidden="true" /><div className="localswarm-row-copy"><h3><bdi>{seed.name}</bdi></h3><span className="muted">{t("{count} connected devices · {size} sent", { count: seed.peerCount, size: formatBytes(seed.uploaded) })}</span></div><button type="button" disabled={busy} onClick={() => void run(async () => { await api("localswarm_stop_seed", { id: seed.id }); })}><Square size={15} aria-hidden="true" />{t("Stop sharing")}</button></div>
        </article>)}
      </section>}
      <details className="localswarm-secondary localswarm-book-offer">
        <summary>{t("Share a book instead")}</summary>
        <p className="muted">{t("Share just one available book from your library.")}</p>
        <div className="localswarm-offer-controls">
          <label className="field">{t("Library book")}<select value={bookId} disabled={blocked || busy} onChange={(event) => setBookId(event.target.value)}><option value="">{t("Choose a book")}</option>{availableBooks.map((book) => <option value={book.id} key={book.id}>{book.title}</option>)}</select></label>
          <button type="button" disabled={blocked || busy || !availableBooks.some((book) => book.id === bookId)} onClick={() => setSeedBook(availableBooks.find((book) => book.id === bookId) || null)}><Upload size={17} aria-hidden="true" />{t("Share")}</button>
        </div>
      </details>
      <details className="localswarm-secondary">
        <summary>{t("Device name")}</summary>
        <form onSubmit={saveNickname}>
          <label className="field">{t("Nearby device name")}<input value={nickname} maxLength={160} disabled={blocked || busy} placeholder={t("Automatic platform name")} autoComplete="off" onChange={(event) => { nameEdited.current = true; setNickname(event.target.value); }} /><span className="muted">{t("Leave blank to use the platform name. Changes apply to new shares; names are not proof of identity.")}</span></label>
          <button type="submit" disabled={blocked || busy || nickname.trim() === preferences.draft.sharingName}>{t("Save device name")}</button>
        </form>
        <SettingsSaveStatus />
      </details>
      <details className="localswarm-secondary">
        <summary>{t("Other ways to share")}</summary>
        <p className="muted">{t("For someone using a browser or an OPDS reader, enable Web sharing separately in Settings. Use only on a trusted network; Web sharing is unencrypted.")}</p>
        <button type="button" onClick={onWebSharing}>{t("Share to a browser")}</button>
      </details>
      <footer className="localswarm-footer">
        <div className="localswarm-toolbar"><button type="button" className="danger" disabled={blocked || stopping} onClick={() => void stop()}><Square size={15} aria-hidden="true" />{stopping ? t("Stopping…") : t("Stop all")}</button><span className="muted">{t("Stops finding and sharing, and pauses nearby transfers.")}</span></div>
        <p className="muted">{t("Nearby transfers appear in Downloads. Check who is sharing and only share content you have permission to distribute.")}</p>
      </footer>
      {selectedPeer && !blocked && <Modal title={selectedPeer.publisherName || inventory?.name || selectedPeer.name || t("Nearby device")} onClose={closeReceive}>
        {inventory ? <div className="localswarm-library">
          <p className="muted" role="status">{t("Connected")} · {t("{count} archives", { count: inventory.books.length })}</p>
          {inventory.books.length > 0 && <section className="localswarm-library-batch" aria-labelledby={`${id}-batch-title`}>
            <h2 id={`${id}-batch-title`}>{t("Download this shared library")}</h2>
            <p className="muted">{t("Adds missing archives to Downloads. Keep this view open while adding. Closing it stops adding; accepted downloads continue. Received archives are never shared automatically.")}</p>
            <div className="localswarm-toolbar">
              <button type="button" className={selectedSharedCount ? undefined : "primary"} disabled={busy || blocked} onClick={() => downloadLibraryBatch()}><ArrowDownToLine size={17} aria-hidden="true" />{batchAdding ? t("Adding downloads…") : t("Download all")}</button>
              <button type="button" className={selectedSharedCount ? "primary" : undefined} disabled={busy || blocked || !selectedSharedCount} onClick={() => downloadLibraryBatch(new Set(downloadableSharedBooks.filter((book) => selectedSharedBooks.has(book.id)).map((book) => book.id)))}>{t("Download selected ({count})", { count: selectedSharedCount })}</button>
              {batchAdding && <button type="button" disabled={batchStopping} onClick={stopAddingLibrary}><Square size={15} aria-hidden="true" />{batchStopping ? t("Stopping adding…") : t("Stop adding")}</button>}
              {batchDownload && <button type="button" disabled={busy || batchAdding || blocked} onClick={viewBatchDownloads}>{t("View downloads")}</button>}
            </div>
            {batchProgress && <div className="localswarm-library-batch-progress" role="status" aria-live="polite" aria-atomic="true">
              <p><strong>{batchAdding ? batchStopping ? t("Stopping adding downloads…") : t("Adding downloads…") : batchProgress.stopped || batchProgress.failure || batchPreparationError ? t("Stopped adding downloads.") : t("Finished adding downloads.")}</strong></p>
              <progress max={Math.max(1, batchProgress.total)} value={batchProgress.processed} aria-label={t("Archives checked")} />
              <p>{t("{processed} of {total} archives checked · {queued} added to Downloads", { processed: batchProgress.processed, total: batchProgress.total, queued: batchProgress.queued })}</p>
              <p className="muted">{t("Already in Library: {installed} · Already in Downloads: {active} · Missing library files: {unavailable} · Duplicate entries: {duplicates}", { installed: batchProgress.installed, active: batchProgress.active, unavailable: batchProgress.unavailable, duplicates: batchProgress.duplicates })}</p>
              {!batchAdding && <p className="muted">{t("Not checked: {count}. Added downloads may still be queued or transferring; see Downloads for completion.", { count: Math.max(0, batchProgress.total - batchProgress.processed) })}</p>}
            </div>}
            {batchPreparationError && <p className="inline-error" role="alert">{t("Could not finish adding downloads: {error}. Accepted downloads are kept. You can try Download all again.", { error: batchPreparationError })}</p>}
            {batchProgress?.failure && <p className="inline-error" role="alert">{t("Could not add “{title}”: {error}. Previously accepted downloads are kept. You can try Download all again or download archives individually.", { title: batchProgress.failure.title || batchProgress.failure.bookId, error: errorText(batchProgress.failure.error) })}</p>}
            {!!batchProgress?.unavailable && <><p className="muted">{t("Missing library files were not downloaded. Use Locate in Library, or remove the missing library entry before downloading it again.")}</p><button type="button" disabled={busy || batchAdding} onClick={() => { closeReceive(); onLibrary(); }}>{t("Library")}</button></>}
          </section>}
          {error && <p className="inline-error" role="alert">{error}</p>}
          {inventory.books.length === 0 && <p className="muted">{t("No available archives in this share.")}</p>}
          {!!inventory.books.length && <div className="localswarm-library-selection">
            <label className="field">{t("Find archives")}<input type="search" value={sharedBookQuery} disabled={busy} onChange={(event) => setSharedBookQuery(event.target.value)} /></label>
            <div className="localswarm-toolbar">
              <span role="status">{t("Selected: {count}", { count: selectedSharedCount })}</span>
              <button type="button" disabled={busy || !selectedSharedCount} onClick={() => setSelectedSharedBooks(new Set())}>{t("Clear selection")}</button>
            </div>
            {visibleSharedBooks.length === 0 && <p className="muted">{t("No matching archives.")}</p>}
          </div>}
          {visibleSharedBooks.map((book) => {
            const installed = installedSharedBooks.get(book.archiveId ?? "");
            return <article className="localswarm-library-book" key={book.id}>
              <label className="localswarm-library-select">
                <input type="checkbox" aria-label={t("Select {title}", { title: book.title || book.fileName })} disabled={busy || !!installed} checked={!installed && selectedSharedBooks.has(book.id)} onChange={(event) => {
                  const checked = event.target.checked;
                  setSelectedSharedBooks((previous) => { const next = new Set(previous); if (checked) next.add(book.id); else next.delete(book.id); return next; });
                }} />
                <span className="localswarm-row-copy"><strong><bdi>{book.title || book.fileName}</bdi></strong><span className="muted">{formatBytes(book.size)}{installed && <> · {t("In library")}{!installed.available && <> · {t("File missing")}</>}</>}</span></span>
              </label>
              <button type="button" disabled={busy} onClick={() => downloadLibrary(book.id)}>{installed ? <BookOpen size={17} aria-hidden="true" /> : <ArrowDownToLine size={17} aria-hidden="true" />}{installed ? installed.available ? t("Open {title}", { title: installed.title }) : t("Library") : libraryDownloading === book.id ? t("Starting download…") : t("Download")}</button>
            </article>;
          })}
          <div className="modal-actions"><button type="button" disabled={busy || pendingPeerId === selectedPeer.id} onClick={() => { const peer = selectedPeerRef.current; if (peer) { selectPeer(peer); browseLibrary(peer); } }}>{t("Refresh")}</button><button type="button" onClick={closeReceive}>{t("Close")}</button></div>
        </div> : selectedPeer.transport === "quic" && selectedPeer.locked ? <form className="localswarm-seed-confirm" onSubmit={(event) => { event.preventDefault(); if (pin) receiveBook(selectedPeer, pin); }}>
          <p>{t("The sender protected this book with a PIN. Ask them for it to download.")}</p>
          {error && <p className="inline-error" role="alert">{error}</p>}
          <label className="field">{t("Share PIN")}<input type="password" value={pin} autoComplete="off" autoFocus disabled={busy} onChange={(event) => setPin(event.target.value)} /></label>
          <div className="modal-actions"><button type="button" onClick={closeReceive}>{t("Cancel")}</button><button className="primary" type="submit" disabled={busy || !pin}>{busy ? t("Starting download…") : t("Download")}</button></div>
        </form> : <div className="localswarm-connection">
          {error || connection?.error ? <p className="inline-error" role="alert">{errorText(error || connection?.error)}</p> : <p role="status">{pendingPeerId ? selectedPeer.transport === "quic-library" ? selectedPeer.discovery === "bluetooth" ? connectionText : t("Loading the shared library…") : t("Starting download…") : t("Choose Browse to open this shared library.")}</p>}
          <div className="modal-actions"><button type="button" onClick={closeReceive}>{t("Close")}</button><button type="button" disabled={busy || pendingPeerId === selectedPeer.id} onClick={() => selectedPeer.transport === "quic-library" ? browseLibrary(selectedPeer) : receiveBook(selectedPeer)}>{error ? t("Retry") : selectedPeer.transport === "quic-library" ? t("Browse") : t("Download")}</button></div>
        </div>}
      </Modal>}
      {!blocked && existingArchive && <Modal title={existingArchive.title} onClose={() => setExistingArchiveId(null)}>
        <p role="status">{t("In library")}{!existingArchive.available && <> · {t("File missing")}</>}</p>
        <div className="modal-actions"><button type="button" onClick={() => setExistingArchiveId(null)}>{t("Close")}</button>{existingArchive.available ? <button type="button" className="primary" onClick={() => openExisting(existingArchive)}><BookOpen size={17} aria-hidden="true" />{t("Open {title}", { title: existingArchive.title })}</button> : <button type="button" className="primary" onClick={() => { setExistingArchiveId(null); onLibrary(); }}>{t("Library")}</button>}</div>
      </Modal>}
      {seedBook && !blocked && <Modal title={t("Share “{name}”?", { name: seedBook.title })} onClose={() => setSeedBook(null)}>
        <form onSubmit={shareBook} className="localswarm-seed-confirm"><p>{t("This book will be available to nearby devices until you stop sharing. Only share content you have permission to distribute.")}</p><div className="modal-actions"><button type="button" onClick={() => setSeedBook(null)}>{t("Cancel")}</button><button type="submit" className="primary" disabled={busy}><Upload size={16} aria-hidden="true" />{t("Share book")}</button></div></form>
      </Modal>}
    </section>
  );
}
