import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
} from "react";
import { flushSync } from "react-dom";
import {
  BookOpen,
  Bookmark,
  ChevronLeft,
  ChevronRight,
  Compass,
  Download,
  FolderOpen,
  Globe2,
  Library as LibraryIcon,
  Plus,
  Radio,
  RotateCcw,
  Search,
  Settings as SettingsIcon,
  Upload,
  X,
  AppWindow,
  PanelsTopLeft,
} from "lucide-react";
import {
  api,
  capturePrivacyContext,
  chooseDirectory,
  chooseFiles,
  requestFolderAccess,
  flushPendingPreferences,
  registerDiskSpaceConfirmation,
  type DiskSpaceWarning,
  isNative,
  isDesktop,
  isMobile,
  isAndroid,
  isIOS,
  isPrivacyContextCurrent,
  isPrivacyPolicyContextCurrent,
  type PrivacyContext,
  withMobileBackgroundCheckpoint,
} from "./api";
import { t, useLocale, formatBytes, formatNumber, localeDirection, setSystemLocale, getLocale } from "./i18n";
import { applyWindowTheme, usesOverlayTitleBar } from "./platform";
import { applySystemTextScale } from "./system-text-scale";
import {
  freezeReaderStorage,
  retireReaderDocuments,
  retireReaderStorageGeneration,
} from "./reader-storage";
import type { Book, DownloadAdmission, DownloadJob, DownloadNotificationAction, DownloadTarget, ImportResult, MapJob, Snapshot, Tab, TabLocation } from "./types";
import { readyDownloadBooks } from "./download-state";
import { insertTab, removeReopened } from "./tab-stack";
import { translateBookmarkDrafts, translateLocation, translateTabs } from "./reference-transitions";
import { privateSyncRequest } from "./private-sync-api";
import { diagnostics, reportError } from "./diagnostics";
import { reportBootPhase, reportFrontendReady } from "./startup-ready";
import { isProviderUnavailableFor, isSuspendedError, normalizeSeverity, type ErrorSeverity } from "./error-visibility";
import { shouldKeepReaderForBackgroundMedia } from "./media-policy";
import {
  ReaderMediaCheckpoint,
  type MediaCheckpointAuthority,
  type MediaCheckpointOwner,
  type MediaPlaybackPosition,
} from "./reader-media-checkpoint";
import Library from "./components/Library";
import CrashReportPrompt from "./components/CrashReport";
import IncomingArchiveDialog, { type IncomingArchiveRequest } from "./components/IncomingArchiveDialog";
import { retryFailedSettings, SettingsDirectoryMigration, SettingsPolicyConfirmation } from "./components/SettingsAutosave";
import UnifiedSearch from "./components/UnifiedSearch";
import "./tabs.css";
import Reader, {
  createReaderWorkspace,
  isApplicationShortcut,
  type ApplicationShortcut,
  type ReaderWorkspace,
} from "./components/Reader";
import { ErrorNotice, Loading, Modal, confirmAction, errorText } from "./components/UI";

// Keep secondary surfaces out of the reader/library startup module graph.
const Catalog = lazy(() => import("./components/Catalog"));
const Downloads = lazy(() => import("./components/Downloads"));
const LocalSwarmControls = lazy(() => import("./components/LocalSwarmControls"));
const Maps = lazy(() => import("./components/Maps"));
const Settings = lazy(() => import("./components/Settings"));
const Collections = lazy(() => import("./components/Collections"));

type Pane =
  | "library"
  | "catalog"
  | "maps"
  | "downloads"
  | "nearby"
  | "bookmarks"
  | "history"
  | "settings"
  | "reader";
const MAX_TABS = 200;
const KNOWN_PANES: readonly Pane[] = [
  "library",
  "catalog",
  "maps",
  "downloads",
  "nearby",
  "bookmarks",
  "history",
  "settings",
  "reader",
];
/** A restored route from an older/newer profile must never render nothing. */
function normalizePane(value: unknown): Pane {
  return KNOWN_PANES.includes(value as Pane) ? (value as Pane) : "library";
}
const TAB_DRAG_TYPE = "application/x-kiwix-tab";
// Read by public/theme-boot.js before the first paint. The native shell
// supplies the saved theme directly; this mirror covers browser previews and
// reloads where that boot hint is absent. Keep the key in sync with the script.
const SHELL_THEME_KEY = "kiwix.shell-theme";
const TAB_SELECTION_ACTIONS = [
  "select-tab-1",
  "select-tab-2",
  "select-tab-3",
  "select-tab-4",
  "select-tab-5",
  "select-tab-6",
  "select-tab-7",
  "select-tab-8",
  "select-tab-9",
  "select-tab-0",
] as const;
type ShellShortcut =
  ApplicationShortcut | (typeof TAB_SELECTION_ACTIONS)[number];
function isShellShortcut(action: string): action is ShellShortcut {
  return (
    isApplicationShortcut(action) ||
    TAB_SELECTION_ACTIONS.some((allowed) => allowed === action)
  );
}
type MobileIncoming = IncomingArchiveRequest;
type MobileEntry = {
  requestId: string;
  kind:
    | "library"
    | "search"
    | "new-tab"
    | "get-content"
    | "download-action"
    | "article"
    | "library-notice"
    | "sharing"
    | "voice-search"
    | "bookmarks";
  value: string;
};
type MobileLifecycleState = {
  active: boolean;
  interactive: boolean;
  backgroundRequestId: string | null;
  retirementRevision: number;
  incoming: MobileIncoming[];
  entries: MobileEntry[];
};
type MobileVoiceSearch = {
  privacy: PrivacyContext;
  outcome?: { result: { cancelled?: boolean; query?: string } } | { error: unknown };
};
type MobileManagedImport = {
  privacy: PrivacyContext;
} & ({ kind: "files"; paths: string[] } | { kind: "folder"; path: string } |
  { kind: "registered"; result: ImportResult });
function waitForSessionOperation<T>(pending: Promise<T>): Promise<T> {
  // The ES2022/webview baseline predates Promise.withResolvers; setTimeout is callback-based.
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(
      () =>
        reject(
          new Error(
            "The local service did not finish the session operation within 8 seconds.",
          ),
        ),
      8000,
    );
    pending.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

// Removing an iframe does not run the archive's pagehide handlers. Keep the old
// keyed reader mounted until a real document navigation has handed off its writes.
function ReaderHost({ reader }: { reader: ReactElement | null }) {
  const retained = useRef(reader);
  const desired = useRef(reader);
  const retiring = useRef(false);
  const mustRemount = useRef(false);
  const [epoch, setEpoch] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const [failure, setFailure] = useState("");
  desired.current = reader;
  const sameReader =
    retained.current?.key === reader?.key &&
    Boolean(retained.current) === Boolean(reader);
  if (sameReader && !retiring.current && !mustRemount.current)
    retained.current = reader;
  useLayoutEffect(() => {
    if (
      retiring.current ||
      (!mustRemount.current &&
        retained.current?.key === desired.current?.key &&
        Boolean(retained.current) === Boolean(desired.current))
    )
      return;
    retiring.current = true;
    mustRemount.current = true;
    setFailure("");
    void retireReaderDocuments().then(
      () => {
        retained.current = desired.current;
        retiring.current = false;
        mustRemount.current = false;
        // A rapid A → B → A still retired A's browsing context. Never reuse its
        // blank iframe just because the latest requested React key matches.
        setEpoch((value) => value + 1);
      },
      (error) => {
        retiring.current = false;
        setFailure(errorText(error));
      },
    );
  }, [reader?.key, Boolean(reader), attempt]);
  return (
    <div
      style={
        failure
          ? {
              position: "fixed",
              inset: 0,
              zIndex: 1000,
              display: "flex",
              flexDirection: "column",
              background: "var(--paper)",
            }
          : { display: sameReader ? "contents" : "none" }
      }
    >
      {failure && (
        <ErrorNotice
          message={failure}
          onRetry={() => setAttempt((value) => value + 1)}
        />
      )}
      <div key={epoch} style={{ display: "contents" }} inert={Boolean(failure)}>
        {retained.current}
      </div>
    </div>
  );
}
export default function App() {
  const localeContext = useLocale();
  const { setLocale } = localeContext;
  const resolvedLocale = getLocale();
  // Direction depends only on the resolved locale; the media query list is a
  // stable handle whose `.matches` tracks viewport changes.
  const rtl = useMemo(
    () => localeDirection(resolvedLocale) === "rtl",
    [resolvedLocale],
  );
  const narrowReaderChrome = useMemo(
    () => window.matchMedia("(max-width: 540px)"),
    [],
  );
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [pane, setPane] = useState<Pane>("library");
  const hydrated = useRef(false);
  const startupNavigation = useRef<Pane | null>(null);
  const navigate = useCallback((destination: Pane) => {
    if (!hydrated.current) startupNavigation.current = destination;
    setPane(destination);
  }, []);
  const [nearbyDiscoveryIntent, setNearbyDiscoveryIntent] = useState(0);
  const [webSharingIntent, setWebSharingIntent] = useState(0);
  const webSharingSequence = useRef(0);
  const [privateSyncIntent, setPrivateSyncIntent] = useState(0);
  const privateSyncSequence = useRef(0);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const [closed, setClosed] = useState<Tab[]>([]);
  const [mobileTabsOpen, setMobileTabsOpen] = useState(false);
  const [readerChrome, setReaderChrome] = useState<{ owner: string; hidden: boolean } | null>(null);
  const retainedTabFocus = useRef<{ activeTab: string | null; pane: Pane } | null>(null);
  const [downloadTarget, setDownloadTarget] = useState<DownloadTarget | null>(null);
  const [watchedDownloads, setWatchedDownloads] = useState<{
    target: DownloadTarget;
    privacy: PrivacyContext;
  }[]>([]);
  const [completedDownloads, setCompletedDownloads] = useState<Book[]>([]);
  const focusDownload = useCallback((target: DownloadTarget, admission: DownloadAdmission) => {
    const { privacy, focus } = admission;
    if (!isPrivacyPolicyContextCurrent(privacy) || privacy.privateMode || privacy.readOnly) return;
    const retained = { ...target };
    setWatchedDownloads((previous) => [
      ...previous.filter((entry) => entry.target.kind !== target.kind || entry.target.id !== target.id),
      { target: retained, privacy },
    ]);
    if (focus && isPrivacyContextCurrent(privacy)) {
      setDownloadTarget(retained);
      setPane("downloads");
    }
  }, []);
  const dismissDownloadTarget = useCallback((target: DownloadTarget) => {
    setDownloadTarget((previous) => previous?.kind === target.kind && previous.id === target.id ? null : previous);
    setWatchedDownloads((previous) => previous.filter((entry) =>
      entry.target.kind !== target.kind || entry.target.id !== target.id));
  }, []);
  useEffect(() => {
    const retained = retainedTabFocus.current;
    retainedTabFocus.current = null;
    if (retained && retained.activeTab === activeTab && retained.pane === pane) return;
    setMobileTabsOpen(false);
  }, [activeTab, pane]);
  useEffect(() => {
    if (mobileTabsOpen && window.matchMedia("(max-width: 760px)").matches)
      (document.querySelector<HTMLElement>('#reader-tabs .tab-select[aria-current="page"]') ??
        document.querySelector<HTMLElement>("#reader-tabs .tab-select, #reader-tabs .new-tab"))?.focus();
  }, [mobileTabsOpen]);
  useLayoutEffect(() => {
    if (!mobileTabsOpen && window.matchMedia("(max-width: 760px)").matches &&
        document.activeElement?.closest("#reader-tabs"))
      document.getElementById("main-content")?.focus();
  }, [mobileTabsOpen]);
  useEffect(() => {
    if (!mobileTabsOpen) return;
    const dismissOutside = (event: PointerEvent) => {
      if (event.target instanceof Element &&
          !event.target.closest("#reader-tabs, [data-mobile-tabs-trigger]"))
        setMobileTabsOpen(false);
    };
    document.addEventListener("pointerdown", dismissOutside, true);
    return () => document.removeEventListener("pointerdown", dismissOutside, true);
  }, [mobileTabsOpen]);
  const dismissMobileTabs = () => {
    setMobileTabsOpen(false);
    Array.from(document.querySelectorAll<HTMLElement>('[data-mobile-tabs-trigger="true"]'))
      .find((button) => button.getClientRects().length > 0)?.focus();
  };
  const [mapChoices, setMapChoices] = useState<Record<string, string>>({});
  const [readerGeneration, setReaderGeneration] = useState(0);
  const [readerWorkspaces, setReaderWorkspaces] = useState<{
    epoch: number;
    tabs: Record<string, ReaderWorkspace>;
  }>({ epoch: 0, tabs: {} });
  // Keep only non-sensor workspace data, and reject late writes from an earlier privacy epoch.
  const workspaceBoundary = `${readerGeneration}:${snapshot?.settings.privateMode}`;
  const workspaceContext = useRef({ boundary: workspaceBoundary, epoch: 0 });
  if (workspaceContext.current.boundary !== workspaceBoundary)
    workspaceContext.current = {
      boundary: workspaceBoundary,
      epoch: workspaceContext.current.epoch + 1,
    };
  const workspaceEpoch = workspaceContext.current.epoch;
  // The band's generic prefix frames only a blocking failure. A recoverable
  // diagnostic shown with technical details on stands as its own sentence.
  const [error, setError] = useState<{ message: string; severity: ErrorSeverity } | null>(null);
  const [initialError, setInitialError] = useState("");
  const [startupNoticeDismissed, setStartupNoticeDismissed] = useState(false);
  const [importing, setImporting] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [mobileSuspended, setMobileSuspended] = useState(false);
  const [mobileMediaRetained, setMobileMediaRetained] = useState(false);
  const [audioInterruption, setAudioInterruption] = useState(0);
  const [mobileInteraction, setMobileInteraction] = useState({ interactive: !isMobile, revision: 0 });
  const sessionAutosaveAllowed = useRef(!isMobile);
  sessionAutosaveAllowed.current = !quitting &&
    (!isMobile || (!mobileSuspended && mobileInteraction.interactive));
  const downloadConsentAllowed = useRef(false);
  downloadConsentAllowed.current = !quitting &&
    (!isMobile || (!mobileSuspended && mobileInteraction.interactive));
  const [mobileRecoveryNeeded, setMobileRecoveryNeeded] = useState(false);
  const [mobileIncoming, setMobileIncoming] = useState<MobileIncoming[]>([]);
  const [mobileIncomingBusy, setMobileIncomingBusy] = useState(false);
  const [documentProgress, setDocumentProgress] = useState<number | null>(null);
  const [diskSpaceWarning, setDiskSpaceWarning] = useState<DiskSpaceWarning | null>(null);
  const diskSpaceConfirmation = useRef<{
    privacy: PrivacyContext;
    resolve: (accepted: boolean) => void;
  } | null>(null);
  const dismissedIncoming = useRef(new Set<string>());
  const mobileRetirementRevision = useRef(0);
  // An in-page destination refused for a stale generation is replayed once
  // after the live snapshot confirms the new generation.
  const pendingReaderOpen = useRef<{
    location: TabLocation;
    generation: number;
    tabId: string;
  } | null>(null);
  const mobileReconcile = useRef<(() => void) | null>(null);
  const mobileIntentsAwaitingSnapshot = useRef(false);
  const mobileExternalSelection = useRef<{ book: Book; privacy: PrivacyContext } | null>(null);
  const mobileManagedImport = useRef<MobileManagedImport | null>(null);
  const mobileManagedBusy = useRef(false);
  const mobileBooks = useRef<Book[]>([]);
  const mobileLibraryChanges = useRef({ requested: 0, applied: 0 });
  const mobileWidgetSnapshot = useRef("");
  // Publish only the launcher-visible summary. Restricted sessions clear the
  // archive titles instead of leaving a previous session's labels on screen.
  useEffect(() => {
    if (!isAndroid || !snapshot) return;
    const restricted = snapshot.settings.privateMode || snapshot.settings.readOnly;
    const payload = {
      titles: restricted
        ? []
        : snapshot.books
            .filter((book) => book.available)
            .slice(0, 3)
            .map((book) => book.title),
      privateMode: snapshot.settings.privateMode,
      readOnly: snapshot.settings.readOnly,
    };
    const signature = JSON.stringify(payload);
    if (signature === mobileWidgetSnapshot.current) return;
    const timer = window.setTimeout(() => {
      // Best-effort: the widget keeps its last published view if the native
      // bridge is unavailable, and a later snapshot retries the update.
      void api("mobile_update_search_widget", payload)
        .then(() => {
          mobileWidgetSnapshot.current = signature;
        })
        .catch(() => {});
    }, 500);
    return () => window.clearTimeout(timer);
  }, [snapshot]);
  const mobileReaderShortcut = useRef<((action: string) => void) | null>(null);
  const readerScrollFlush = useRef<(() => void) | null>(null);
  const newTabRef = useRef<(() => void) | null>(null);
  const processedEntries = useRef(new Set<string>());
  // Root/window-owned continuations outlive lifecycle queue revisions, not
  // this shell instance. A foreground-only consumer applies each result once.
  const mobileVoiceSearches = useRef(new Map<string, MobileVoiceSearch>());
  const [downloadNotificationActions, setDownloadNotificationActions] = useState<DownloadNotificationAction[]>([]);
  const downloadNotificationQueue = useRef(new Map<string, DownloadNotificationAction>());
  const runningDownloadNotificationActions = useRef(new Set<string>());
  const completedDownloadNotificationActions = useRef(new Set<string>());
  const beginDownloadNotificationAction = useCallback((requestId: string) => {
    if (!downloadNotificationQueue.current.has(requestId)) return false;
    if (runningDownloadNotificationActions.current.has(requestId) ||
        completedDownloadNotificationActions.current.has(requestId)) return false;
    runningDownloadNotificationActions.current.add(requestId);
    return true;
  }, []);
  const finishDownloadNotificationAction = useCallback((requestId: string, complete: boolean) => {
    if (!downloadNotificationQueue.current.has(requestId) &&
        !runningDownloadNotificationActions.current.has(requestId)) return;
    runningDownloadNotificationActions.current.delete(requestId);
    if (complete) {
      completedDownloadNotificationActions.current.add(requestId);
      downloadNotificationQueue.current.delete(requestId);
      setDownloadNotificationActions([...downloadNotificationQueue.current.values()]);
    }
    mobileReconcile.current?.();
  }, []);
  const [globalSearch, setGlobalSearch] = useState(false);
  // A native menu command that needs the library pane to focus a specific
  // control. The incrementing id makes repeated identical commands distinct.
  const [libraryCommand, setLibraryCommand] = useState<{
    id: number;
    kind: "search" | "scope";
    query?: string;
    record?: boolean;
  } | null>(null);
  const libraryCommandId = useRef(0);
  const requestLibraryCommand = useCallback((kind: "search" | "scope") => {
    navigate("library");
    setLibraryCommand({ id: ++libraryCommandId.current, kind });
  }, [navigate]);
  const requestLibrarySearch = useCallback((query: string, record = true) => {
    setPane("library");
    setLibraryCommand({
      id: ++libraryCommandId.current,
      kind: "scope",
      query: query.trim().slice(0, 4096) || undefined,
      record,
    });
  }, []);
  const clearLibraryCommand = useCallback((id: number) => {
    setLibraryCommand((current) => (current?.id === id ? null : current));
  }, []);
  const [tabDrop, setTabDrop] = useState<{ id: string; after: boolean } | null>(
    null,
  );
  // The drop target lives in a ref so a dragover that lands on the same
  // half of the same tab does not re-render the shell.
  const tabDropRef = useRef<{ id: string; after: boolean } | null>(null);
  // `direction` is inherited and constant per locale; read it once per element.
  const tabDirectionCache = useRef({
    rtl: false,
    directions: new WeakMap<Element, "rtl" | "ltr">(),
  });
  // Only a trusted dragstart on a root-owned tab can establish reorder authority.
  // Never obtain the tab ID from DataTransfer: archive frames can supply arbitrary data.
  const tabDrag = useRef<{ id: string; nonce: string } | null>(null);
  const tabButtons = useRef(new Map<string, HTMLButtonElement>());
  const newTabButton = useRef<HTMLButtonElement>(null);
  const quitDialog = useRef<HTMLDialogElement>(null);
  const mobileRecoveryDialog = useRef<HTMLDialogElement>(null);
  const quitRequest = useRef<{ id: string; resumeStorage?: () => void } | null>(
    null,
  );
  const quitEventSerial = useRef(0);
  const sessionTimer = useRef<number | undefined>(undefined);
  const sessionFlushes = useRef(0);
  const mounted = useRef(true);
  const refreshSerial = useRef(0);
  // A suspended snapshot is retried by the API boundary itself; the resume
  // listeners must not fire a second copy of the same request.
  const snapshotSuspended = useRef(false);
  // A privacy boundary blocks tab mutations and autosave until its snapshot
  // lands. A failed (non-suspended) boundary snapshot is retried with backoff
  // instead of leaving the shell inert until an unrelated event refreshes it.
  const privacyRetry = useRef<{ attempt: number; timer?: number; run?: () => void }>({ attempt: 0 });
  const consumedLaunches = useRef(new Set<string>());
  const launchHistoryPending = useRef(new Set<string>());
  const launchCapacityPending = useRef(false);
  const sessionQueue = useRef(Promise.resolve());
  const sessionGeneration = useRef(0);
  const referenceRevision = useRef(0);
  const referenceHistory = useRef<Snapshot["referenceTransitions"]>([]);
  const privacyPending = useRef(false);
  const observedPrivacy = useRef<{ mode: boolean; generation: number } | null>(
    null,
  );
  const current = tabs.find((tab) => tab.id === activeTab);
  const tabFocusContext = useRef({
    activeTab, pane, workspaceEpoch,
    bookId: current?.bookId,
    path: current?.path,
    privacyGeneration: snapshot?.privacyGeneration,
    readOnly: snapshot?.settings.readOnly,
  });
  const previousTabFocus = tabFocusContext.current;
  if (previousTabFocus.activeTab !== activeTab || previousTabFocus.pane !== pane ||
      previousTabFocus.workspaceEpoch !== workspaceEpoch ||
      previousTabFocus.bookId !== current?.bookId || previousTabFocus.path !== current?.path ||
      previousTabFocus.privacyGeneration !== snapshot?.privacyGeneration ||
      previousTabFocus.readOnly !== snapshot?.settings.readOnly)
    tabFocusContext.current = {
      activeTab, pane, workspaceEpoch,
      bookId: current?.bookId,
      path: current?.path,
      privacyGeneration: snapshot?.privacyGeneration,
      readOnly: snapshot?.settings.readOnly,
    };
  const latestSession = useRef({
    tabs,
    activeTab,
    activeSurface: pane,
    settings: snapshot?.settings,
    privacyGeneration: snapshot?.privacyGeneration,
    referenceRevision: referenceRevision.current,
  });
  latestSession.current = {
    tabs,
    activeTab,
    activeSurface: pane,
    settings: snapshot?.settings,
    privacyGeneration: snapshot?.privacyGeneration,
    referenceRevision: referenceRevision.current,
  };
  const [mediaCheckpoint] = useState(() => new ReaderMediaCheckpoint());
  useEffect(() => { mediaCheckpoint.clear(); },
    [mediaCheckpoint, activeTab, pane, workspaceBoundary, snapshot?.settings.readOnly, quitting]);
  // The media progress path runs several times per second per playing video.
  // Rebuild these indexes only when the root actually replaces its arrays.
  const checkpointTabIndex = useRef<{ source: Tab[] | null; byId: Map<string, Tab> }>(
    { source: null, byId: new Map() },
  );
  const checkpointBookIndex = useRef<{ source: Book[] | null; byId: Map<string, Book> }>(
    { source: null, byId: new Map() },
  );
  const mediaCheckpointOwner = useCallback((tabId: string, authority: MediaCheckpointAuthority): MediaCheckpointOwner | null => {
    const session = latestSession.current;
    const tabIndex = checkpointTabIndex.current;
    if (tabIndex.source !== session.tabs) {
      const byId = new Map<string, Tab>();
      for (const tab of session.tabs) if (!byId.has(tab.id)) byId.set(tab.id, tab);
      tabIndex.source = session.tabs;
      tabIndex.byId = byId;
    }
    const tab = tabIndex.byId.get(tabId);
    const bookIndex = checkpointBookIndex.current;
    if (bookIndex.source !== mobileBooks.current) {
      const byId = new Map<string, Book>();
      for (const book of mobileBooks.current) if (!byId.has(book.id)) byId.set(book.id, book);
      bookIndex.source = mobileBooks.current;
      bookIndex.byId = byId;
    }
    const book = bookIndex.byId.get(authority.bookId);
    if (!mounted.current || privacyPending.current || quitRequest.current ||
        session.activeSurface !== "reader" || session.activeTab !== tabId ||
        !session.settings || session.settings.privateMode || session.settings.readOnly ||
        !tab || tab.bookId !== authority.bookId || tab.path !== authority.article ||
        !book || book.installationId !== authority.installationId ||
        session.privacyGeneration !== authority.privacyGeneration ||
        observedPrivacy.current?.generation !== authority.privacyPolicyGeneration)
      return null;
    return { tabId, bookId: book.id, installationId: book.installationId, article: authority.article,
      sourceIdentity: `${book.path}\n${book.sourceReference?.reference ?? ""}`,
      privacyPolicyGeneration: authority.privacyPolicyGeneration };
  }, []);
  const recordMediaCheckpoint = useCallback((tabId: string, authority: MediaCheckpointAuthority, positions: MediaPlaybackPosition[]) => {
    const owner = mediaCheckpointOwner(tabId, authority);
    if (owner) mediaCheckpoint.record(owner, positions);
  }, [mediaCheckpoint, mediaCheckpointOwner]);
  const readMediaCheckpoint = useCallback((tabId: string, authority: MediaCheckpointAuthority) => {
    const owner = mediaCheckpointOwner(tabId, authority);
    return owner ? mediaCheckpoint.read(owner) : [];
  }, [mediaCheckpoint, mediaCheckpointOwner]);
  // The active reader reports audible/PiP playback; the mobile lifecycle uses
  // it to keep a genuinely playing document alive across suspension, and iOS
  // uses the same report to activate its playback audio session. `release`
  // says the reader surface itself is gone, which is the only thing that drops
  // the session the first audible surface latched.
  const readerMedia = useRef({
    audible: false,
    pictureInPicture: false,
    release: false,
  });
  const reportedReaderMedia = useRef("");
  const reportReaderMedia = useCallback(
    (state: {
      audible: boolean;
      pictureInPicture: boolean;
      release?: boolean;
    }) => {
      const release = state.release === true;
      readerMedia.current = {
        audible: state.audible,
        pictureInPicture: state.pictureInPicture,
        release,
      };
      if (!isIOS) return;
      const key = `${state.audible ? 1 : 0}:${state.pictureInPicture ? 1 : 0}:${release ? 1 : 0}`;
      if (reportedReaderMedia.current === key) return;
      reportedReaderMedia.current = key;
      void api("mobile_media_state", {
        audible: state.audible,
        pictureInPicture: state.pictureInPicture,
        release,
      }).catch(() => {
        // A lost report only removes the background exemption; reading must
        // continue. Clear the dedupe key so the next report retries.
        reportedReaderMedia.current = "";
      });
    },
    [],
  );
  const launchReaderWorkspace = useCallback((id: string): ReaderWorkspace => ({
    ...createReaderWorkspace(),
    historyNavigationPending: launchHistoryPending.current.has(id),
  }), []);
  const updateReaderWorkspace = useCallback(
    (
      id: string,
      epoch: number,
      update: (previous: ReaderWorkspace) => ReaderWorkspace,
    ) => {
      setReaderWorkspaces((previous) => {
        if (
          epoch !== workspaceContext.current.epoch ||
          !latestSession.current.tabs.some((tab) => tab.id === id)
        )
          return previous;
        const workspaces = previous.epoch === epoch ? previous.tabs : {};
        const current = workspaces[id] || launchReaderWorkspace(id);
        const next = update(current);
        return next === current
          ? previous
          : { epoch, tabs: { ...workspaces, [id]: next } };
      });
    },
    [launchReaderWorkspace],
  );
  useEffect(() => {
    for (const id of launchHistoryPending.current)
      if (!tabs.some((tab) => tab.id === id)) launchHistoryPending.current.delete(id);
    setReaderWorkspaces((previous) => {
      if (previous.epoch !== workspaceEpoch)
        return { epoch: workspaceEpoch, tabs: {} };
      const retained = Object.fromEntries(
        Object.entries(previous.tabs).filter(([id]) =>
          tabs.some((tab) => tab.id === id),
        ),
      );
      return Object.keys(retained).length === Object.keys(previous.tabs).length
        ? previous
        : { epoch: workspaceEpoch, tabs: retained };
    });
  }, [tabs, workspaceEpoch]);
  const showTechnicalErrors = useRef(false);
  showTechnicalErrors.current = snapshot?.settings.showTechnicalErrors === true;
  const onError = useCallback(
    (value: unknown, severity: ErrorSeverity = "blocking") => {
      const render = reportError(
        diagnostics,
        value,
        severity,
        showTechnicalErrors.current,
      );
      if (render) setError({ message: errorText(value), severity: normalizeSeverity(severity) });
    },
    [],
  );
  useEffect(() => {
    if (!isIOS) return;
    let cancelled = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const apply = (value: unknown) => {
      const size = value as { category?: unknown; scale?: unknown } | null;
      if (size && typeof size.category === "string" &&
          typeof size.scale === "number" && Number.isFinite(size.scale) && size.scale > 0)
        applySystemTextScale({ category: size.category, scale: size.scale });
    };
    void import("@tauri-apps/api/event").then(async ({ listen }) => {
      const remove = await listen("kiwix-mobile-dynamic-type", (event) => {
        if (!cancelled) { revision++; apply(event.payload); }
      });
      if (cancelled) { remove(); return; }
      unlisten = remove;
      const observed = revision;
      const size = await api("mobile_dynamic_type_state");
      if (!cancelled && observed === revision) apply(size);
    }).catch((value) => { if (!cancelled) onError(value, "recoverable"); });
    return () => { cancelled = true; unlisten?.(); };
  }, [onError]);
  const nativeLocaleQueue = useRef<Promise<void>>(Promise.resolve());
  const previousNativeLocale = useRef<string | null>(null);
  useEffect(() => {
    const locale = snapshot?.settings.locale;
    if (!isAndroid || !locale || mobileSuspended || !mobileInteraction.interactive) return;
    let cancelled = false;
    const preservePlatform = previousNativeLocale.current === null || previousNativeLocale.current === locale;
    previousNativeLocale.current = locale;
    nativeLocaleQueue.current = nativeLocaleQueue.current.catch(() => {}).then(async () => {
      if (cancelled) return;
      const result = await api<{ locale: string; effectiveLocale: string }>("mobile_apply_locale", { preservePlatform });
      if (!cancelled) setSystemLocale(result.locale ? result.effectiveLocale : null);
    }).catch((value) => { if (!cancelled) onError(value, "recoverable"); });
    return () => { cancelled = true; };
  }, [snapshot?.settings.locale, mobileInteraction.revision, mobileSuspended, onError]);
  const finishDiskSpaceConfirmation = useCallback((accepted: boolean) => {
    const pending = diskSpaceConfirmation.current;
    if (!pending) return;
    diskSpaceConfirmation.current = null;
    setDiskSpaceWarning(null);
    pending.resolve(accepted && mounted.current && downloadConsentAllowed.current &&
      !privacyPending.current && !quitRequest.current && isPrivacyContextCurrent(pending.privacy));
  }, []);
  useEffect(() => {
    const unregister = registerDiskSpaceConfirmation((warning) => {
      const privacy = capturePrivacyContext();
      if (!mounted.current || !downloadConsentAllowed.current || diskSpaceConfirmation.current ||
          privacyPending.current || quitRequest.current || !privacy || privacy.privateMode || privacy.readOnly)
        return Promise.resolve(false);
      return new Promise<boolean>((resolve) => {
        diskSpaceConfirmation.current = { privacy, resolve };
        setDiskSpaceWarning(warning);
      });
    });
    return () => {
      unregister();
      const pending = diskSpaceConfirmation.current;
      diskSpaceConfirmation.current = null;
      pending?.resolve(false);
    };
  }, []);
  useEffect(() => {
    const pending = diskSpaceConfirmation.current;
    if (pending && (!downloadConsentAllowed.current || !isPrivacyContextCurrent(pending.privacy)))
      finishDiskSpaceConfirmation(false);
  }, [diskSpaceWarning, quitting, mobileSuspended, mobileInteraction.interactive,
      snapshot?.privacyGeneration, snapshot?.settings.privateMode,
      snapshot?.settings.readOnly, finishDiskSpaceConfirmation]);
  useEffect(() => {
    const failed = (event: Event) => {
      const detail = (event as CustomEvent<unknown>).detail;
      if (
        detail &&
        typeof detail === "object" &&
        "error" in detail
      ) {
        const workflow = detail as { error: unknown; severity?: ErrorSeverity };
        onError(workflow.error, workflow.severity);
        return;
      }
      onError(detail);
    };
    window.addEventListener("kiwix-reader-storage-error", failed);
    window.addEventListener("kiwix-desktop-workflow-error", failed);
    return () => {
      window.removeEventListener("kiwix-reader-storage-error", failed);
      window.removeEventListener("kiwix-desktop-workflow-error", failed);
    };
  }, [onError]);
  const refresh = useCallback(async () => {
    const id = ++refreshSerial.current;
    reportBootPhase("snapshot", "Requesting the startup snapshot");
    const pending = api<Snapshot>("snapshot");
    try {
      const data = await pending;
      if (!mounted.current || id !== refreshSerial.current) return null;
      mobileBooks.current = data.books;
      retireReaderStorageGeneration(data.privacyGeneration);
      const previousPrivacy = observedPrivacy.current;
      const boundaryChanged =
        previousPrivacy !== null &&
        previousPrivacy.generation !== data.privacyPolicyGeneration;
      const modeChanged =
        previousPrivacy !== null &&
        previousPrivacy.mode !== data.settings.privateMode;
      if (boundaryChanged || modeChanged) {
        mediaCheckpoint.clear();
        consumedLaunches.current.clear();
        launchHistoryPending.current.clear();
        launchCapacityPending.current = false;
        sessionGeneration.current++;
        setClosed([]);
        setDownloadTarget(null);
        setWatchedDownloads([]);
        setCompletedDownloads([]);
        setGlobalSearch(false);
        setReaderGeneration((previous) => previous + 1);
        for (const requestId of downloadNotificationQueue.current.keys())
          completedDownloadNotificationActions.current.add(requestId);
        downloadNotificationQueue.current.clear();
        setDownloadNotificationActions([]);
      }
      observedPrivacy.current = {
        mode: data.settings.privateMode,
        generation: data.privacyPolicyGeneration,
      };
      setSnapshot((old) => ({
        ...data,
        settings:
          old && JSON.stringify(old.settings) === JSON.stringify(data.settings)
            ? old.settings
            : data.settings,
      }));
      setInitialError("");
      snapshotSuspended.current = false;
      const initial = !hydrated.current;
      const selectedAtStartup = initial ? startupNavigation.current : null;
      const restore = initial || boundaryChanged || modeChanged;
      if (restore) {
        hydrated.current = true;
        setLocale(data.settings.locale);
      }
      const previous = latestSession.current;
      const transitions = data.referenceTransitions ?? [];
      referenceHistory.current = transitions;
      const referencesChanged = (data.referenceRevision ?? 0) > referenceRevision.current;
      const previousRevision = referenceRevision.current;
      const liveTabs = referencesChanged
        ? translateTabs(previous.tabs, transitions, previousRevision)
        : previous.tabs;
      if (referencesChanged) {
        setClosed((tabs) => translateTabs(tabs, transitions, previousRevision));
        setReaderWorkspaces((previous) => ({
          ...previous,
          tabs: Object.fromEntries(Object.entries(previous.tabs).map(([id, workspace]) => [
            id,
            { ...workspace, bookmarkDrafts: translateBookmarkDrafts(workspace.bookmarkDrafts, transitions, previousRevision) },
          ])),
        }));
        if (pendingReaderOpen.current)
          pendingReaderOpen.current = {
            ...pendingReaderOpen.current,
            location: translateLocation(pendingReaderOpen.current.location, transitions, previousRevision),
          };
        // A selection is not transferable provider/installation authority.
        mobileExternalSelection.current = null;
        referenceRevision.current = data.referenceRevision;
      }
      // Startup restore follows the setting; a window created by the explicit
      // "Restore previous session" command adopts its session regardless.
      const adoptSaved = data.settings.restoreSession || data.sessionRestored === true;
      let next: Tab[];
      if (restore) next = adoptSaved ? data.tabs.slice(0, MAX_TABS) : [];
      else {
        const liveBookIds = new Set(data.books.map((book) => book.id));
        next = liveTabs.filter(
          (tab) => tab.bookId === null || liveBookIds.has(tab.bookId),
        );
      }
      const requestedActive = restore ? data.activeTab : previous.activeTab;
      let active = next.some((tab) => tab.id === requestedActive)
        ? requestedActive
        : next[0]?.id || null;
      const acknowledged: string[] = [];
      let opened = false;
      let capacityBlocked = false;
      for (const location of quitRequest.current
        ? []
        : data.launchLocations || []) {
        if (consumedLaunches.current.has(location.launchId)) {
          acknowledged.push(location.launchId);
          continue;
        }
        // The request ID is also its tab ID, so a repeated snapshot cannot append
        // another tab, even while its acknowledgement is still in flight.
        const existing = next.find((tab) => tab.id === location.launchId);
        if (existing) {
          active = existing.id;
        } else if (next.length < MAX_TABS) {
          next.push({
            id: location.launchId,
            bookId: location.bookId,
            path: location.path,
            title: location.title,
            scroll: location.scroll,
            back: [],
            forward: [],
          });
          active = location.launchId;
        } else {
          capacityBlocked = true;
          continue;
        }
        if (location.bookId) launchHistoryPending.current.add(location.launchId);
        consumedLaunches.current.add(location.launchId);
        acknowledged.push(location.launchId);
        opened = true;
      }
      launchCapacityPending.current =
        capacityBlocked ||
        Boolean(quitRequest.current && data.launchLocations?.length);
      if (capacityBlocked)
        onError(
          new Error(
            t(
              "Close a tab before opening another. This window supports up to 200 tabs.",
            ),
          ),
        );
      const requestedSurface =
        data.activeSurface === "reader" && !active
          ? "library"
          : data.activeSurface || (active ? "reader" : "library");
      const restoredSurface = adoptSaved ? requestedSurface : "library";
      const sessionSurface = restore ? restoredSurface : previous.activeSurface;
      const surface = normalizePane(opened ? "reader" : selectedAtStartup ?? sessionSurface);
      latestSession.current = {
        tabs: next,
        activeTab: active,
        activeSurface: surface,
        settings: data.settings,
        privacyGeneration: data.privacyGeneration,
        referenceRevision: referenceRevision.current,
      };
      if (restore || opened || referencesChanged || next.length !== previous.tabs.length)
        setTabs(next);
      if (restore || active !== previous.activeTab) setActiveTab(active);
      if (restore || opened) {
        if (selectedAtStartup !== "nearby" || opened || data.launchSearch) setNearbyDiscoveryIntent(0);
        setPane(surface);
      }
      // Desktop "Look up in Kiwix". Rust keeps only the latest request, under
      // a fresh ID each time; apply each ID once, after any tab restore so the
      // library search is what the window shows, then acknowledge it.
      const launchSearch = quitRequest.current ? null : data.launchSearch;
      if (launchSearch) {
        if (!consumedLaunches.current.has(launchSearch.launchId)) {
          consumedLaunches.current.add(launchSearch.launchId);
          requestLibrarySearch(launchSearch.query, false);
        }
        void api("ack_launch_search", { id: launchSearch.launchId }).catch(onError);
      }
      if (acknowledged.length)
        void api("ack_launch_locations", {
          ids: acknowledged,
          privacyGeneration: data.privacyGeneration,
        }).catch(onError);
      privacyPending.current = false;
      startupNavigation.current = null;
      if (mobileIntentsAwaitingSnapshot.current) {
        mobileIntentsAwaitingSnapshot.current = false;
        mobileReconcile.current?.();
      }
      window.clearTimeout(privacyRetry.current.timer);
      privacyRetry.current = { ...privacyRetry.current, attempt: 0, timer: undefined };
      reportFrontendReady();
      return true;
    } catch (e) {
      if (!mounted.current || id !== refreshSerial.current) return null;
      reportBootPhase("snapshot-failed", errorText(e));
      snapshotSuspended.current = isSuspendedError(e);
      // A boundary snapshot retried with backoff reports its failure once,
      // not on every attempt.
      if (hydrated.current && !(privacyPending.current && privacyRetry.current.attempt > 0)) onError(e);
      else if (!snapshotSuspended.current) setInitialError(errorText(e));
      // A suspended snapshot is retried by the API boundary on foreground.
      if (hydrated.current && privacyPending.current && !snapshotSuspended.current) {
        const retry = privacyRetry.current;
        window.clearTimeout(retry.timer);
        const delay = Math.min(30_000, 1000 * 2 ** Math.min(retry.attempt, 5));
        retry.attempt += 1;
        retry.timer = window.setTimeout(() => {
          retry.timer = undefined;
          if (mounted.current && privacyPending.current) retry.run?.();
        }, delay);
      }
      return false;
    }
  }, [setLocale, onError, mediaCheckpoint, requestLibrarySearch]);
  // Downloads owns its visible progress polling. Keep only user-started work
  // observed elsewhere so completion never steals the current reading surface.
  useEffect(() => {
    if (!watchedDownloads.length || pane === "downloads" || mobileSuspended || quitting) return;
    let cancelled = false;
    let timer: number | undefined;
    async function poll() {
      const current = watchedDownloads.filter((entry) => isPrivacyPolicyContextCurrent(entry.privacy));
      if (!current.length) {
        timer = window.setTimeout(() => void poll(), 1500);
        return;
      }
      const [downloads, map] = await Promise.allSettled([
        current.some((entry) => entry.target.kind === "download")
          ? api<DownloadJob[]>("downloads") : Promise.resolve([]),
        current.some((entry) => entry.target.kind === "map")
          ? api<MapJob | null>("map_status") : Promise.resolve(null),
      ]);
      if (cancelled) return;
      const downloadJobs = downloads.status === "fulfilled"
        ? new Map(downloads.value.map((job): [string, DownloadJob] => [job.id, job]))
        : null;
      const matched = current.flatMap((entry) => {
        const job = entry.target.kind === "download"
          ? downloadJobs?.get(entry.target.id) ?? null
          : map.status === "fulfilled" && map.value?.id === entry.target.id ? map.value : null;
        return job && isPrivacyPolicyContextCurrent(entry.privacy) ? [{ ...entry, job }] : [];
      });
      if (matched.some(({ job }) => job.importState === "imported")) await refresh();
      if (cancelled) return;
      // One library scan per ready job, reused by both the check and the update.
      const ready: { target: DownloadTarget; books: Book[] }[] = [];
      for (const entry of matched) {
        if (!isPrivacyPolicyContextCurrent(entry.privacy)) continue;
        const books = readyDownloadBooks(entry.job, mobileBooks.current);
        if (books.length) ready.push({ target: entry.target, books });
      }
      if (ready.length) {
        const books = ready.flatMap((entry) => entry.books);
        setCompletedDownloads((previous) => [...previous,
          ...books.filter((book) => !previous.some((existing) => existing.installationId === book.installationId))]);
        setWatchedDownloads((previous) => previous.filter((entry) =>
          !ready.some(({ target }) => target.kind === entry.target.kind && target.id === entry.target.id)));
      }
      if (!cancelled) timer = window.setTimeout(() => void poll(), 1500);
    }
    void poll();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [watchedDownloads, pane, mobileSuspended, quitting, refresh]);
  privacyRetry.current.run = () => void refresh();
  useEffect(() => {
    mounted.current = true;
    void refresh();
    return () => {
      mounted.current = false;
      window.clearTimeout(privacyRetry.current.timer);
    };
  }, [refresh]);
  useEffect(() => {
    if (!snapshot || snapshot.settings.privateMode || snapshot.settings.readOnly ||
        quitting || mobileSuspended || (isMobile && !mobileInteraction.interactive)) return;
    const expectedGeneration = snapshot.privacyGeneration;
    let cancelled = false;
    let reading = false;
    let lastSyncs: string | null = null;
    async function reconcilePrivateSync() {
      if (cancelled || reading || document.visibilityState !== "visible" || !document.hasFocus()) return;
      const privacy = capturePrivacyContext();
      if (!privacy || privacy.privateMode || privacy.readOnly || privacy.generation !== expectedGeneration) return;
      reading = true;
      try {
        // Native owns encrypted sessions and automatic scheduling. This read
        // reopens an enabled listener after lifecycle retirement; it never opts in.
        const status = await privateSyncRequest("private_sync_status", {}, privacy);
        if (cancelled || !isPrivacyContextCurrent(privacy) ||
            document.visibilityState !== "visible" || !document.hasFocus()) return;
        const next = JSON.stringify(status.peers.map((peer) => [peer.id, peer.lastSync ?? null]));
        if (lastSyncs !== null && lastSyncs !== next) void refresh();
        lastSyncs = next;
      } catch (cause) {
        if (!cancelled && isPrivacyContextCurrent(privacy) &&
            document.visibilityState === "visible" && document.hasFocus() &&
            !isSuspendedError(cause) &&
            !String(cause).includes("Return to the app before syncing private reading data"))
          onError(cause, "recoverable");
      } finally { reading = false; }
    }
    void reconcilePrivateSync();
    const timer = window.setInterval(() => void reconcilePrivateSync(), 15000);
    window.addEventListener("focus", reconcilePrivateSync);
    window.addEventListener("pageshow", reconcilePrivateSync);
    document.addEventListener("visibilitychange", reconcilePrivateSync);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", reconcilePrivateSync);
      window.removeEventListener("pageshow", reconcilePrivateSync);
      document.removeEventListener("visibilitychange", reconcilePrivateSync);
    };
  }, [snapshot?.privacyGeneration, snapshot?.settings.privateMode, snapshot?.settings.readOnly,
    quitting, mobileSuspended, mobileInteraction.interactive, refresh, onError]);
  useEffect(() => {
    if (
      launchCapacityPending.current &&
      tabs.length < MAX_TABS &&
      !privacyPending.current &&
      !quitRequest.current
    )
      void refresh();
  }, [tabs.length, quitting, refresh]);
  useEffect(() => {
    if (!snapshot) return;
    document.documentElement.dataset.theme = snapshot.settings.theme;
    try {
      localStorage.setItem(SHELL_THEME_KEY, snapshot.settings.theme);
    } catch {
      // The native boot hint remains authoritative; a blocked mirror is harmless.
    }
    applyWindowTheme(snapshot.settings.theme);
    setLocale(snapshot.settings.locale);
  }, [snapshot?.settings.theme, snapshot?.settings.locale, setLocale]);
  // WebKit paints its own under-page background between frame documents. The
  // native side owns that colour, but only the page hears a system appearance
  // change, which is what a "system" theme and an "original" reader follow.
  useEffect(() => {
    if (!isDesktop) return;
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const sync = () => {
      void api("reader_backdrop", {}).catch(() => undefined);
    };
    query.addEventListener("change", sync);
    return () => query.removeEventListener("change", sync);
  }, []);
  useEffect(() => {
    if (
      !hydrated.current ||
      !snapshot ||
      snapshot.settings.privateMode ||
      snapshot.settings.readOnly ||
      privacyPending.current ||
      quitRequest.current ||
      sessionFlushes.current ||
      !sessionAutosaveAllowed.current
    )
      return;
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.generation !== snapshot.privacyGeneration) return;
    const generation = sessionGeneration.current;
    const privacyGeneration = privacy.generation;
    const timer = window.setTimeout(() => {
      sessionTimer.current = undefined;
      if (quitRequest.current || sessionFlushes.current) return;
      sessionQueue.current = sessionQueue.current
        .then(async () => {
          const current = latestSession.current;
          if (
            generation !== sessionGeneration.current ||
            !sessionAutosaveAllowed.current ||
            !isPrivacyContextCurrent(privacy) ||
            quitRequest.current ||
            sessionFlushes.current ||
            privacyPending.current ||
            current.privacyGeneration !== privacyGeneration ||
            current.settings?.privateMode ||
            current.settings?.readOnly
          )
            return;
          await api<void>("save_session", {
            tabs: current.tabs,
            activeTab: current.activeTab,
            activeSurface: current.activeSurface,
            privacyGeneration,
            referenceRevision: current.referenceRevision,
          });
        })
        .catch((value) => {
          if (
            generation === sessionGeneration.current &&
            sessionAutosaveAllowed.current &&
            isPrivacyContextCurrent(privacy) &&
            !privacyPending.current
          )
            onError(value);
        });
    // The reader publishes scroll at a 250 ms cadence. Keep this debounce above
    // that so continuous scrolling does not enqueue one full save_session per
    // publish; the timer still fires once shortly after scrolling stops.
    }, 600);
    sessionTimer.current = timer;
    return () => {
      window.clearTimeout(timer);
      if (sessionTimer.current === timer) sessionTimer.current = undefined;
    };
  }, [
    tabs,
    activeTab,
    pane,
    snapshot?.privacyGeneration,
    snapshot?.settings.privateMode,
    snapshot?.settings.readOnly,
    quitting,
    mobileSuspended,
    mobileInteraction.interactive,
    onError,
  ]);
  useEffect(() => {
    if (activeTab && !tabs.some((tab) => tab.id === activeTab)) {
      const active = tabs[0]?.id || null;
      latestSession.current = { ...latestSession.current, activeTab: active };
      setActiveTab(active);
    }
    if (!tabs.length && pane === "reader") setPane("library");
  }, [tabs, activeTab, pane]);
  const appendTab = useCallback(
    (tab: Tab, background = false) => {
      if (privacyPending.current || quitRequest.current) return false;
      const current = latestSession.current;
      if (current.tabs.length >= MAX_TABS) {
        onError(
          new Error(
            t(
              "Close a tab before opening another. This window supports up to 200 tabs.",
            ),
          ),
        );
        return false;
      }
      // A stale reopen/launch replay must not open a second tab with the same
      // identity (React key, workspace key, close filter, saved session).
      const next = insertTab(current.tabs, tab, current.activeTab, background);
      if (!next) return false;
      latestSession.current = {
        ...current,
        tabs: next,
        activeTab: background ? current.activeTab : tab.id,
        activeSurface: background ? current.activeSurface : "reader",
      };
      const epoch = workspaceContext.current.epoch;
      setReaderWorkspaces((previous) =>
        epoch !== workspaceContext.current.epoch ? previous : {
          epoch,
          tabs: {
            ...(previous.epoch === epoch ? previous.tabs : {}),
            [tab.id]: {
              ...createReaderWorkspace(),
              historyNavigationPending: !!tab.bookId,
            },
          },
        });
      setTabs(next);
      if (!background) {
        setActiveTab(tab.id);
        setPane("reader");
      }
      return true;
    },
    [onError],
  );
  const selectTab = useCallback((id: string, focus = false) => {
    if (privacyPending.current || quitRequest.current) return;
    const current = latestSession.current;
    if (!current.tabs.some((tab) => tab.id === id)) return;
    latestSession.current = { ...current, activeTab: id, activeSurface: "reader" };
    if (focus && (current.activeTab !== id || current.activeSurface !== "reader"))
      retainedTabFocus.current = { activeTab: id, pane: "reader" };
    const select = () => {
      setActiveTab(id);
      setPane("reader");
      if (focus && window.matchMedia("(max-width: 760px)").matches)
        setMobileTabsOpen(true);
    };
    if (focus) {
      flushSync(select);
      tabButtons.current.get(id)?.focus();
    } else select();
  }, []);
  const reorderTab = useCallback((id: string, index: number) => {
    if (privacyPending.current || quitRequest.current) return;
    const current = latestSession.current;
    const from = current.tabs.findIndex((tab) => tab.id === id);
    const to = Math.max(0, Math.min(current.tabs.length - 1, index));
    if (from < 0 || from === to) return;
    const next = [...current.tabs];
    const [tab] = next.splice(from, 1);
    next.splice(to, 0, tab);
    latestSession.current = { ...current, tabs: next };
    setTabs(next);
  }, []);
  const moveTab = useCallback(
    (id: string, direction: -1 | 1) => {
      const index = latestSession.current.tabs.findIndex(
        (tab) => tab.id === id,
      );
      if (index >= 0) reorderTab(id, index + direction);
    },
    [reorderTab],
  );
  const endTabDrag = useCallback(() => {
    tabDrag.current = null;
    tabDropRef.current = null;
    setTabDrop(null);
    setDragging(false);
  }, []);
  const open = useCallback(
    (location: TabLocation) => {
      appendTab({
        ...location,
        id: crypto.randomUUID(),
        back: [],
        forward: [],
      });
    },
    [appendTab],
  );
  const openExternalSelection = useCallback((book: Book, privacy: PrivacyContext) => {
    if (!isPrivacyPolicyContextCurrent(privacy)) return;
    mobileExternalSelection.current = { book, privacy };
    mobileReconcile.current?.();
  }, []);
  const openInBackground = useCallback(
    (location: TabLocation) => {
      appendTab(
        {
          ...location,
          id: crypto.randomUUID(),
          back: [],
          forward: [],
        },
        true,
      );
    },
    [appendTab],
  );
  const openReaderTab = useCallback(
    (location: TabLocation) => {
      if (latestSession.current.settings?.openNewTabInBackground === false) {
        appendTab(
          {
            ...location,
            id: crypto.randomUUID(),
            back: [],
            forward: [],
          },
          false,
        );
        return;
      }
      openInBackground(location);
    },
    [appendTab, openInBackground],
  );
  useEffect(() => {
    const pending = pendingReaderOpen.current;
    if (!pending || !snapshot || pending.generation === snapshot.privacyGeneration)
      return;
    pendingReaderOpen.current = null;
    // The tab or the window boundary moved on while the snapshot refreshed;
    // the destination is no longer meaningful for this window.
    if (
      quitRequest.current ||
      privacyPending.current ||
      latestSession.current.activeTab !== pending.tabId
    )
      return;
    openReaderTab(pending.location);
  }, [
    snapshot?.privacyGeneration,
    openReaderTab,
    quitting,
    mobileSuspended,
  ]);
  const newTab = useCallback(() => {
    if (!hydrated.current) return;
    appendTab({
      id: crypto.randomUUID(),
      bookId: null,
      path: "",
      title: "",
      scroll: 0,
      back: [],
      forward: [],
    });
  }, [appendTab]);
  newTabRef.current = newTab;
  const closeTab = useCallback(
    async (id: string) => {
      if (privacyPending.current || quitRequest.current) return;
      const focusContext = tabFocusContext.current;
      const focused = document.activeElement;
      const closingTab = tabButtons.current.get(id)?.closest(".reader-tab");
      let ownsFocus = Boolean(focused && (
        closingTab?.contains(focused) ||
        (focusContext.activeTab === id && focusContext.pane === "reader" &&
          document.getElementById("main-content")?.contains(focused))
      ));
      const relinquishFocus = () => { ownsFocus = false; };
      const observeFocus = (event: FocusEvent) => {
        if (event.target !== focused && event.target !== document.body)
          relinquishFocus();
      };
      // Retirement may blur the old frame itself. New interaction, unlike that
      // blur, relinquishes this operation's right to recover the removed focus.
      document.addEventListener("focusin", observeFocus, true);
      document.addEventListener("pointerdown", relinquishFocus, true);
      document.addEventListener("keydown", relinquishFocus, true);
      window.addEventListener("blur", relinquishFocus);
      let resumeStorage: ((restore?: boolean) => void) | undefined;
      let closed = false;
      try {
        readerScrollFlush.current?.();
        if (latestSession.current.activeTab === id)
          resumeStorage = await freezeReaderStorage();
        if (privacyPending.current || quitRequest.current) return;
        const current = latestSession.current;
        const index = current.tabs.findIndex((tab) => tab.id === id);
        const target = current.tabs[index];
        if (!target) return;
        const remaining = current.tabs.filter((tab) => tab.id !== id);
        const active =
          current.activeTab === id
            ? remaining[Math.min(index, remaining.length - 1)]?.id || null
            : current.activeTab;
        const surface = !remaining.length && current.activeSurface === "reader"
          ? "library" : current.activeSurface;
        const restoreFocus = ownsFocus && tabFocusContext.current === focusContext &&
          mounted.current && document.visibilityState !== "hidden" &&
          !document.querySelector(".app-shell[hidden], dialog[open]") &&
          (document.activeElement === focused || document.activeElement === document.body);
        if (restoreFocus &&
            (current.activeTab !== active || current.activeSurface !== surface))
          retainedTabFocus.current = { activeTab: active, pane: surface };
        latestSession.current = {
          ...current,
          tabs: remaining,
          activeTab: active,
          activeSurface: surface,
        };
        flushSync(() => {
          setClosed((old) => [...old.slice(-19), target]);
          setTabs(remaining);
          setActiveTab(active);
          if (!remaining.length)
            setPane((pane) => (pane === "reader" ? "library" : pane));
        });
        closed = true;
        const focusAfterClose = document.activeElement;
        const focusLost = !focusAfterClose || focusAfterClose === document.body ||
          (focusAfterClose === focused && focused &&
            (!focused.isConnected || !focused.getClientRects().length ||
              Boolean(focused.closest("[inert], [hidden]"))));
        if (restoreFocus && focusLost && ownsFocus && mounted.current &&
            workspaceContext.current.epoch === focusContext.workspaceEpoch &&
            !document.querySelector(".app-shell[hidden], dialog[open]")) {
          if (window.matchMedia("(max-width: 760px)").matches)
            flushSync(() => setMobileTabsOpen(true));
          const destination = active ? tabButtons.current.get(active) : newTabButton.current;
          if (destination?.isConnected && destination.getClientRects().length &&
              !destination.closest("[inert], [hidden]"))
            destination.focus();
          else document.getElementById("main-content")?.focus({ preventScroll: true });
        }
      } catch (value) {
        onError(value);
      } finally {
        document.removeEventListener("focusin", observeFocus, true);
        document.removeEventListener("pointerdown", relinquishFocus, true);
        document.removeEventListener("keydown", relinquishFocus, true);
        window.removeEventListener("blur", relinquishFocus);
        resumeStorage?.(!closed);
      }
    },
    [onError],
  );
  const undoClose = useCallback(() => {
    const tab = closed[closed.length - 1];
    if (!tab) return;
    if (latestSession.current.tabs.some((open) => open.id === tab.id)) {
      // Already open (a reopen handled before this render): retire the stale
      // stack entry so older closed tabs stay reachable, and show it.
      setClosed((previous) => removeReopened(previous, tab.id));
      selectTab(tab.id);
      return;
    }
    if (appendTab(tab))
      setClosed((previous) => removeReopened(previous, tab.id));
  }, [closed, appendTab, selectTab]);
  const newWindow = useCallback(async () => {
    if (!isDesktop) return;
    try {
      await api("new_window");
    } catch (e) {
      onError(e);
    }
  }, [onError]);
  // Explicit "Restore previous session": the backend refuses in private or
  // read-only mode and only fills windows that are not already open. When it
  // restores this window, adopt the saved reader tabs and surface directly.
  const adoptRestoredSession = useCallback(
    async (session: {
      tabs: Tab[];
      activeTab: string | null;
      activeSurface: string;
      referenceRevision: number;
    }) => {
      if (session.referenceRevision > referenceRevision.current) {
        if (!await refresh()) throw new Error("Could not reconcile restored archive editions. Retry restoring the session.");
      }
      const restoredSurface = normalizePane(session.activeSurface);
      const current = latestSession.current;
      // A tab opened while native restoration was pending is unsaved user work.
      if (current.tabs.length) return;
      const restoredTabs = translateTabs(session.tabs, referenceHistory.current, session.referenceRevision);
      latestSession.current = {
        ...current,
        tabs: restoredTabs,
        activeTab: session.activeTab,
        activeSurface: restoredSurface,
      };
      flushSync(() => {
        setTabs(restoredTabs);
        setActiveTab(session.activeTab);
        setNearbyDiscoveryIntent(0);
        setPane(restoredSurface);
      });
    },
    [refresh],
  );
  const restorePreviousSession = useCallback(async () => {
    if (!isDesktop) return;
    if (privacyPending.current || quitRequest.current) return;
    try {
      const result = await api<{
        window: {
          tabs: Tab[];
          activeTab: string | null;
          activeSurface: Pane;
          referenceRevision: number;
        } | null;
        restored: string[];
        skipped: string[];
        failed: { window: string; error: string }[];
      }>("restore_session");
      if (privacyPending.current || quitRequest.current) return;
      const restored = result.window;
      if (restored) await adoptRestoredSession(restored);
      // A failed window is a failed user action; skipped windows were left
      // alone on purpose, so they stay quiet and only reach diagnostics.
      if (result.failed?.length) {
        onError(
          new Error(
            t("Some saved windows could not be reopened: {details}", {
              details: result.failed
                .map((failure) => `${failure.window}: ${failure.error}`)
                .join("\n"),
            }),
          ),
        );
      }
      if (result.skipped?.length) {
        onError(
          new Error(
            t(
              "Some saved windows were left unchanged because they are already open.",
            ),
          ),
          "recoverable",
        );
      }
    } catch (e) {
      onError(e);
    }
  }, [adoptRestoredSession, onError]);
  // An already-open window cannot adopt restored tabs from the command result
  // or a startup snapshot, so the backend pushes its saved session here.
  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<{
          tabs: Tab[];
          activeTab: string | null;
          activeSurface: string;
          referenceRevision: number;
        }>("kiwix-session-restored", (event) => {
          const payload = event.payload;
          if (
            !payload ||
            !Array.isArray(payload.tabs) ||
            payload.tabs.length > MAX_TABS ||
            (payload.activeTab !== null &&
              typeof payload.activeTab !== "string") ||
            typeof payload.activeSurface !== "string" ||
            !Number.isSafeInteger(payload.referenceRevision) || payload.referenceRevision < 0
          )
            return;
          void adoptRestoredSession(payload).catch(onError);
        }),
      )
      .then((dispose) => {
        if (cancelled) dispose();
        else unlisten = dispose;
      })
      .catch((value) => onError(value, "recoverable"));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [adoptRestoredSession, onError]);
  const importPaths = useCallback(
    async (paths: string[]) => {
      const context = capturePrivacyContext();
      const generation = sessionGeneration.current;
      if (
        !paths.length ||
        !context ||
        privacyPending.current ||
        quitRequest.current
      )
        return;
      setImporting(true);
      try {
        const result = await api<ImportResult>("import_archives", {
          paths,
          privacyGeneration: context.generation,
        });
        if (
          !mounted.current ||
          generation !== sessionGeneration.current ||
          !isPrivacyContextCurrent(context) ||
          quitRequest.current
        )
          return;
        await refresh();
        if (
          !mounted.current ||
          generation !== sessionGeneration.current ||
          !isPrivacyContextCurrent(context) ||
          quitRequest.current
        )
          return;
        setPane("library");
        if (result.errors.length)
          onError(
            result.errors
              .map((item) => isIOS ? item.error : `${item.path}: ${item.error}`)
              .join("\n"),
          );
      } catch (e) {
        onError(e);
      } finally {
        setImporting(false);
      }
    },
    [refresh, onError],
  );
  const scanPath = useCallback(async (path: string) => {
    const context = capturePrivacyContext();
    const generation = sessionGeneration.current;
    if (!context || privacyPending.current || quitRequest.current) return;
    setImporting(true);
    try {
      const result = await api<ImportResult>("scan_folder", { path, recursive: true });
      if (!mounted.current || generation !== sessionGeneration.current ||
          !isPrivacyContextCurrent(context) || quitRequest.current) return;
      await refresh();
      if (!mounted.current || generation !== sessionGeneration.current ||
          !isPrivacyContextCurrent(context) || quitRequest.current) return;
      setPane("library");
      if (result.errors.length)
        onError(result.errors.map((item) => isIOS ? item.error : `${item.path}: ${item.error}`).join("\n"));
    } catch (e) {
      onError(e);
    } finally {
      setImporting(false);
    }
  }, [refresh, onError]);
  const pickArchives = useCallback(async (folder: boolean, copy: boolean) => {
    const context = capturePrivacyContext();
    const generation = sessionGeneration.current;
    if (!context || privacyPending.current || quitRequest.current) return;
    if (isMobile && mobileManagedBusy.current) return;
    if (isMobile && copy && (context.privateMode || context.readOnly)) return;
    if (isMobile) mobileManagedBusy.current = true;
    let queued = false;
    setImporting(true);
    try {
      if (isMobile && !copy) {
        const result = await api<ImportResult | null>("mobile_add_external_archives", {
          folder, privacyGeneration: context.generation,
        });
        if (!result || !mounted.current || privacyPending.current || quitRequest.current ||
            !isPrivacyPolicyContextCurrent(context)) return;
        // Registration reads originals; resume the shell only after the picker
        // lifecycle has settled. Never feed these books into the copy importer.
        mobileManagedImport.current = { kind: "registered", result, privacy: context };
        queued = true;
        mobileReconcile.current?.();
        return result.books.length > 0 && result.errors.length === 0;
      } else if (folder) {
        const path = isMobile
          ? await api<string | null>("mobile_pick_directory_copy", { privacyGeneration: context.generation })
          : await chooseDirectory();
        if (!path || !mounted.current || privacyPending.current || quitRequest.current) return;
        if (isMobile) {
          if (!isPrivacyPolicyContextCurrent(context)) return;
          mobileManagedImport.current = { kind: "folder", path, privacy: context };
          queued = true;
          mobileReconcile.current?.();
        } else if (generation === sessionGeneration.current && isPrivacyContextCurrent(context)) {
          await scanPath(path);
        }
      } else {
        const paths = isMobile
          ? await api<string[]>("mobile_pick_archive_copies", { privacyGeneration: context.generation })
          : await chooseFiles();
        if (!paths.length || !mounted.current || privacyPending.current || quitRequest.current) return;
        if (isMobile) {
          if (!isPrivacyPolicyContextCurrent(context)) return;
          mobileManagedImport.current = { kind: "files", paths, privacy: context };
          queued = true;
          mobileReconcile.current?.();
        } else if (generation === sessionGeneration.current && isPrivacyContextCurrent(context)) {
          await importPaths(paths);
        }
      }
    } catch (value) {
      onError(value);
    } finally {
      if (!queued) {
        if (isMobile) mobileManagedBusy.current = false;
        setImporting(false);
      }
      setDocumentProgress(null);
    }
  }, [importPaths, scanPath, onError]);
  const importFiles = useCallback(() => pickArchives(false, false), [pickArchives]);
  const scan = useCallback(() => pickArchives(true, false), [pickArchives]);
  const importCopies = useCallback(() => pickArchives(false, true), [pickArchives]);
  const importFolderCopy = useCallback(() => pickArchives(true, true), [pickArchives]);
  useEffect(() => {
    if (!isNative) return;
    const disposers: (() => void)[] = [];
    let cancelled = false;
    function keep(dispose: () => void) {
      if (cancelled) dispose();
      else disposers.push(dispose);
    }
    if (isDesktop) void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (
            tabDrag.current ||
            privacyPending.current ||
            quitRequest.current
          ) {
            setDragging(false);
            return;
          }
          if (event.payload.type === "drop") {
            setDragging(false);
            const dropped = event.payload.paths;
            const context = capturePrivacyContext();
            const generation = sessionGeneration.current;
            if (!context) return;
            void requestFolderAccess(dropped)
              .then(() => {
                if (
                  cancelled ||
                  !mounted.current ||
                  privacyPending.current ||
                  quitRequest.current ||
                  generation !== sessionGeneration.current ||
                  !isPrivacyContextCurrent(context)
                ) return;
                return importPaths(dropped);
              })
              .catch(onError);
          } else
            setDragging(
              event.payload.type === "enter" || event.payload.type === "over",
            );
        }),
      )
      .then(keep)
      .catch(onError);
    import("@tauri-apps/api/event")
      .then(async ({ listen }) => {
        keep(
          await listen("kiwix-open", () => {
            void refresh();
          }),
        );
        // Close the initial-snapshot/listener-registration gap. Events only wake
        // refresh; a queued request remains authoritative until acknowledged.
        void refresh();
        keep(
          await listen("kiwix-library-changed", () => {
            if (isMobile) {
              mobileLibraryChanges.current.requested++;
              mobileReconcile.current?.();
            } else void refresh();
          }),
        );
        keep(
          await listen("kiwix-privacy-changed", () => {
            privacyPending.current = true;
            void refresh();
          }),
        );
      })
      .catch(onError);
    return () => {
      cancelled = true;
      for (const dispose of disposers) dispose();
    };
  }, [importPaths, onError, refresh]);
  const flushSession = useCallback(
    async (request?: { id: string }) => {
      sessionFlushes.current++;
      let active = true;
      const generation = sessionGeneration.current;
      const privacyGeneration = latestSession.current.privacyGeneration;
      if (sessionTimer.current !== undefined) {
        window.clearTimeout(sessionTimer.current);
        sessionTimer.current = undefined;
      }
      const previous = sessionQueue.current;
      const pending = flushPendingPreferences()
        .then(() => previous)
        .then(async () => {
          for (;;) {
            if (privacyPending.current) await refresh();
            if (!active || (request && quitRequest.current !== request))
              throw new Error("Session flush was cancelled.");
            const session = latestSession.current;
            if (
              !hydrated.current ||
              privacyPending.current ||
              !session.settings ||
              session.privacyGeneration === undefined
            )
              throw new Error(
                t("Could not connect to the local library service."),
              );
            if (session.settings.privateMode || session.settings.readOnly)
              return;
            if (
              generation !== sessionGeneration.current ||
              privacyGeneration !== session.privacyGeneration
            )
              throw new Error(
                "Session privacy changed during the session flush. Retry Quit.",
              );
            await api<void>("save_session", {
              tabs: session.tabs,
              activeTab: session.activeTab,
              activeSurface: session.activeSurface,
              privacyGeneration: session.privacyGeneration,
              referenceRevision: session.referenceRevision,
            });
            const latest = latestSession.current;
            if (
              !privacyPending.current &&
              generation === sessionGeneration.current &&
              session.tabs === latest.tabs &&
              session.activeTab === latest.activeTab &&
              session.activeSurface === latest.activeSurface &&
              session.privacyGeneration === latest.privacyGeneration
              && session.referenceRevision === latest.referenceRevision
            )
              return;
          }
        });
      // Keep later writes serial even if this explicit, acknowledged flush fails.
      sessionQueue.current = pending.catch(() => {});
      try {
        await waitForSessionOperation(pending);
      } finally {
        active = false;
        sessionFlushes.current--;
      }
    },
    [refresh],
  );
  useEffect(() => {
    if (!isMobile) return;
    let cancelled = false;
    let revision = 0;
    let paused = false;
    let acknowledgedRequest: string | null = null;
    let resumeStorage: ((restore?: boolean) => void) | undefined;
    let queue = Promise.resolve();
    const disposers: (() => void)[] = [];
    let notificationAckRetry: number | undefined;
    const keep = (dispose: () => void) => {
      if (cancelled) dispose();
      else disposers.push(dispose);
    };
    const reconcile = () => {
      const current = ++revision;
      queue = queue.then(async () => {
        // A foreground burst enqueues several reconciles. Only the newest one
        // may query native state; the rest are already superseded.
        if (cancelled || current !== revision) return;
        // Native events only wake this root-owned state query; their payloads
        // cannot authorize storage retirement, resumption, or document import.
        const state = await api<MobileLifecycleState>("mobile_lifecycle_state");
        if (cancelled || current !== revision) return;
        // The freshest state read on this pass, reused by the final check
        // unless an intervening refresh or import made it stale.
        let latestState = state;
        let needsStateRead = false;
        if (state.retirementRevision !== mobileRetirementRevision.current)
          flushSync(() => setMobileMediaRetained(false));
        if (!state.active || !state.interactive || paused ||
            state.retirementRevision !== mobileRetirementRevision.current) {
          setMobileInteraction((previous) => previous.interactive
            ? { ...previous, interactive: false } : previous);
        }
        setMobileIncoming(state.incoming.filter((item) => !dismissedIncoming.current.has(item.requestId)));
        if (state.active) {
          if (state.retirementRevision !== mobileRetirementRevision.current && !paused) {
            paused = true;
            flushSync(() => setMobileSuspended(true));
            try {
              if (!resumeStorage) {
                const release = await freezeReaderStorage();
                if (cancelled) {
                  release(false);
                  return;
                }
                resumeStorage = release;
              }
            } catch (value) {
              if (!cancelled) {
                onError(value);
                setMobileRecoveryNeeded(true);
              }
            }
            if (cancelled || current !== revision) return;
          }
          if (paused) {
            const refreshed = await refresh();
            if (cancelled || current !== revision) return;
            if (refreshed !== true) {
              // A competing snapshot is not a failed service. Reconcile its
              // replacement rather than leaving a successful resume inert.
              if (refreshed === null) reconcile();
              else if (!snapshotSuspended.current) setMobileRecoveryNeeded(true);
              return;
            }
            // Background retirement has already invalidated reader authority.
            // Refresh keeps live root state for the same policy, but restores
            // the new state at an actual privacy boundary before any writes.
            await flushSession();
            if (cancelled || current !== revision) return;
            const currentState = await api<MobileLifecycleState>("mobile_lifecycle_state");
            if (cancelled || current !== revision) return;
            if (!currentState.active || currentState.retirementRevision !== state.retirementRevision) {
              reconcile();
              return;
            }
            mobileRetirementRevision.current = currentState.retirementRevision;
            latestState = currentState;
            needsStateRead = false;
            resumeStorage?.();
            resumeStorage = undefined;
            paused = false;
            setMobileRecoveryNeeded(false);
            flushSync(() => { setMobileSuspended(false); setMobileMediaRetained(false); });
          }
          const selection = mobileExternalSelection.current;
          const managedImport = mobileManagedImport.current;
          const libraryRevision = mobileLibraryChanges.current.requested;
          if (state.interactive && (selection || managedImport || libraryRevision !== mobileLibraryChanges.current.applied)) {
            // Picker completions and library notifications share the same
            // lifecycle-authorized snapshot handoff.
            const refreshed = await refresh();
            needsStateRead = true;
            if (cancelled || current !== revision) return;
            if (refreshed !== true) {
              if (refreshed === null) reconcile();
              else if (!snapshotSuspended.current) setMobileRecoveryNeeded(true);
              return;
            }
            const confirmed = await api<MobileLifecycleState>("mobile_lifecycle_state");
            if (cancelled || current !== revision) return;
            if (!confirmed.interactive || confirmed.retirementRevision !== mobileRetirementRevision.current) {
              reconcile();
              return;
            }
            latestState = confirmed;
            mobileLibraryChanges.current.applied = libraryRevision;
            if (managedImport && mobileManagedImport.current === managedImport) {
              mobileManagedImport.current = null;
              try {
                if (isPrivacyPolicyContextCurrent(managedImport.privacy) &&
                    !privacyPending.current && !quitRequest.current) {
                  if (managedImport.kind === "registered") {
                    setPane("library");
                    if (managedImport.result.errors.length) onError(managedImport.result.errors
                      .map((item) => isIOS ? item.error : `${item.path}: ${item.error}`).join("\n"));
                  } else if (managedImport.kind === "files") await importPaths(managedImport.paths);
                  else await scanPath(managedImport.path);
                }
              } finally {
                mobileManagedBusy.current = false;
                setImporting(false);
                setDocumentProgress(null);
              }
              if (cancelled || current !== revision) return;
            }
            if (selection && mobileExternalSelection.current === selection) {
              mobileExternalSelection.current = null;
              const book = mobileBooks.current.find((book) =>
                book.id === selection.book.id && book.installationId === selection.book.installationId);
              if (book && isPrivacyPolicyContextCurrent(selection.privacy))
                open({ bookId: book.id, path: book.mainPath, title: book.title, scroll: 0 });
            }
          }
          // The common foreground pass has not awaited anything that can move
          // native state since its last read, so reuse it instead of a third
          // round trip. A refresh or import above sets `needsStateRead`.
          const readyState = needsStateRead
            ? await api<MobileLifecycleState>("mobile_lifecycle_state")
            : latestState;
          if (cancelled || current !== revision) return;
          if (!readyState.active || readyState.retirementRevision !== mobileRetirementRevision.current) {
            reconcile();
            return;
          }
          setMobileInteraction((previous) => ({
            interactive: readyState.interactive,
            revision: previous.revision + (readyState.interactive ? 1 : 0),
          }));
          // PROCESS_TEXT, kiwix:// links and dynamic shortcuts are one-shot
          // intents. Apply each only while the shell is genuinely interactive,
          // then acknowledge the native copy so it is never replayed.
          if (readyState.interactive && document.visibilityState === "visible") {
            // Acknowledge shortcuts only after tab restoration and the first policy snapshot.
            if (!hydrated.current && readyState.entries.length) {
              mobileIntentsAwaitingSnapshot.current = true;
              return;
            }
            // Acknowledgements and the notification-actions state are applied
            // once after the drain instead of per entry: a backlog of N intents
            // must not cost N sequential round trips before the rest apply.
            const acknowledgements: { requestId: string; notification: boolean }[] = [];
            let notificationActionsChanged = false;
            for (const entry of readyState.entries) {
              if (cancelled || current !== revision) return;
              if (entry.kind === "download-action") {
                if (!completedDownloadNotificationActions.current.has(entry.requestId)) {
                  const privacy = capturePrivacyContext();
                  if (!privacy) continue; // Await actual foreground/privacy reconciliation.
                  if (privacy.privateMode || privacy.readOnly) {
                    completedDownloadNotificationActions.current.add(entry.requestId);
                    downloadNotificationQueue.current.delete(entry.requestId);
                    notificationActionsChanged = true;
                  } else if (!processedEntries.current.has(entry.requestId)) {
                    try {
                      const payload = JSON.parse(entry.value) as { id?: unknown; action?: unknown; phase?: unknown };
                      if (typeof payload.id !== "string" ||
                          !["pause", "resume", "cancel", "retry"].includes(String(payload.action)) ||
                          !["active", "paused", "failed"].includes(String(payload.phase))) {
                        completedDownloadNotificationActions.current.add(entry.requestId);
                      } else {
                        processedEntries.current.add(entry.requestId);
                        downloadNotificationQueue.current.set(entry.requestId, {
                          requestId: entry.requestId, id: payload.id as string,
                          action: payload.action as DownloadNotificationAction["action"],
                          phase: payload.phase as DownloadNotificationAction["phase"],
                          privacyGeneration: privacy.generation, privacyEpoch: privacy.epoch,
                        });
                        notificationActionsChanged = true;
                        setPane("downloads");
                      }
                    } catch {
                      completedDownloadNotificationActions.current.add(entry.requestId);
                    }
                  } else {
                    // Background-only retirement preserves intent, never its old
                    // foreground authority. Policy retirement is discarded.
                    const pending = downloadNotificationQueue.current.get(entry.requestId);
                    if (!pending || pending.privacyEpoch !== privacy.epoch) {
                      completedDownloadNotificationActions.current.add(entry.requestId);
                      downloadNotificationQueue.current.delete(entry.requestId);
                    } else {
                      downloadNotificationQueue.current.set(entry.requestId, { ...pending, privacyGeneration: privacy.generation });
                    }
                    notificationActionsChanged = true;
                  }
                }
                if (completedDownloadNotificationActions.current.has(entry.requestId))
                  acknowledgements.push({ requestId: entry.requestId, notification: true });
                continue;
              }
              if (entry.kind === "voice-search") {
                const privacy = capturePrivacyContext();
                if (!privacy || privacyPending.current || quitRequest.current) continue;
                if (!processedEntries.current.has(entry.requestId)) {
                  processedEntries.current.add(entry.requestId);
                  requestLibrarySearch("");
                  if (isAndroid && !privacy.privateMode && !privacy.readOnly) {
                    const pending: MobileVoiceSearch = { privacy };
                    mobileVoiceSearches.current.set(entry.requestId, pending);
                    // Never hold the serialized lifecycle queue across an
                    // external activity: background checkpoints must proceed.
                    void api<{ cancelled?: boolean; query?: string }>(
                      "mobile_voice_search", { language: getLocale() },
                    ).then(
                      (result) => { pending.outcome = { result }; },
                      (error) => { pending.outcome = { error }; },
                    ).then(() => {
                      if (mounted.current && mobileVoiceSearches.current.get(entry.requestId) === pending)
                        mobileReconcile.current?.();
                    });
                  }
                }
                const pending = mobileVoiceSearches.current.get(entry.requestId);
                if (pending) {
                  if (!pending.outcome) continue;
                  // Remove before applying/acknowledging: failed acknowledgments
                  // retry only the ack, never the recognizer or the transcript.
                  mobileVoiceSearches.current.delete(entry.requestId);
                  if (isPrivacyPolicyContextCurrent(pending.privacy)) {
                    if ("error" in pending.outcome) onError(pending.outcome.error);
                    else {
                      const result = pending.outcome.result;
                      if (!result.cancelled && typeof result.query === "string")
                        requestLibrarySearch(result.query.trim().slice(0, 4096));
                    }
                  }
                }
              }
              if (!processedEntries.current.has(entry.requestId)) {
                processedEntries.current.add(entry.requestId);
                if (entry.kind === "new-tab") newTabRef.current?.();
                else if (entry.kind === "get-content") setPane("catalog");
                else if (entry.kind === "library") setPane("library");
                else if (entry.kind === "sharing") {
                  setNearbyDiscoveryIntent((intent) => intent + 1);
                  setPane("nearby");
                }
                else if (entry.kind === "bookmarks") setPane("bookmarks");
                else if (entry.kind === "article") {
                  // Rust canonicalizes the zim:// link to `book-id\nlocation`.
                  const separator = entry.value.indexOf("\n");
                  const bookId = separator < 0 ? "" : entry.value.slice(0, separator);
                  const location = separator < 0 ? "" : entry.value.slice(separator + 1);
                  const book = mobileBooks.current.find((item) => item.id === bookId);
                  if (!book || !book.available) {
                    // A pinned shortcut can outlive its archive. Land on the
                    // library with an actionable notice instead of a dead tab.
                    setPane("library");
                    onError(
                      new Error(
                        t(
                          "This pinned article’s archive is not in your library. Open or re-import it, then try the shortcut again.",
                        ),
                      ),
                      "recoverable",
                    );
                  } else {
                    open({
                      bookId: book.id,
                      path: location || book.mainPath,
                      title: book.title,
                      scroll: 0,
                    });
                  }
                } else if (entry.kind === "library-notice") {
                  setPane("library");
                  onError(new Error(entry.value), "recoverable");
                } else requestLibrarySearch(entry.value);
              }
              acknowledgements.push({ requestId: entry.requestId, notification: false });
            }
            if (notificationActionsChanged)
              setDownloadNotificationActions([...downloadNotificationQueue.current.values()]);
            for (const acknowledgement of acknowledgements) {
              if (cancelled || current !== revision) return;
              try {
                await api("mobile_ack_entry", { requestId: acknowledgement.requestId });
                processedEntries.current.delete(acknowledgement.requestId);
                if (acknowledgement.notification) {
                  completedDownloadNotificationActions.current.delete(acknowledgement.requestId);
                  runningDownloadNotificationActions.current.delete(acknowledgement.requestId);
                }
              } catch (value) {
                if (cancelled || current !== revision) return;
                // Keep the processed marker so a retry acknowledges without
                // repeating the navigation that already happened.
                onError(value);
                if (acknowledgement.notification) {
                  window.clearTimeout(notificationAckRetry);
                  notificationAckRetry = window.setTimeout(reconcile, 500);
                }
              }
            }
          }
          return;
        }
        const requestId = state.backgroundRequestId;
        if (requestId === acknowledgedRequest && requestId !== null) return;
        if (!requestId) {
          // Native retirement has already closed its write window. Do not
          // manufacture an unowned freeze/save that waits for foreground.
          if (state.retirementRevision !== mobileRetirementRevision.current) {
            paused = true;
            flushSync(() => setMobileSuspended(true));
          }
          return;
        }
        await api("mobile_validate_background", { requestId });
        if (cancelled || current !== revision) return;
        const hadHydratedSession = hydrated.current;
        if (!hadHydratedSession) refreshSerial.current++;
        paused = true;
        const keepReaderForMedia = shouldKeepReaderForBackgroundMedia({
          ...readerMedia.current,
          privateMode: Boolean(latestSession.current.settings?.privateMode),
          readOnly: Boolean(latestSession.current.settings?.readOnly),
          retiring: privacyPending.current ||
            state.retirementRevision !== mobileRetirementRevision.current,
          quitting: Boolean(quitRequest.current),
        });
        flushSync(() => {
          setMobileMediaRetained(keepReaderForMedia);
          setMobileSuspended(true);
        });
        await withMobileBackgroundCheckpoint(requestId, async () => {
          let failure: string | undefined;
          try {
            if (!resumeStorage && !keepReaderForMedia) {
              const release = await freezeReaderStorage();
              if (cancelled) { release(false); return; }
              resumeStorage = release;
            }
            if (hadHydratedSession) {
              await flushSession();
            } else {
              const queued = sessionQueue.current;
              await flushPendingPreferences();
              await queued;
            }
          } catch (value) {
            failure = errorText(value);
            if (!cancelled && !isSuspendedError(value)) {
              onError(value);
              setMobileRecoveryNeeded(true);
            }
          }
          if (cancelled || current !== revision) return;
          await api("mobile_background_ack", { requestId, error: failure });
          acknowledgedRequest = requestId;
        });
      }).catch((value) => {
        if (!cancelled && !isSuspendedError(value)) {
          onError(value);
          if (paused) setMobileRecoveryNeeded(true);
        }
      });
    };
    mobileReconcile.current = reconcile;
    void import("@tauri-apps/api/event").then(async ({ listen }) => {
      for (const name of ["kiwix-mobile-background", "kiwix-mobile-active", "kiwix-mobile-incoming", "kiwix-mobile-entry"])
        keep(await listen(name, reconcile));
      // A cold launch can be reported before the OS marks the scene active.
      // A suspension is retried by the API boundary once the native lifecycle
      // is active; other startup failures still get one foreground retry here.
      keep(await listen("kiwix-mobile-active", () => {
        if (!hydrated.current && !snapshotSuspended.current) void refresh();
      }));
      // WebKit handles native interruptions. Keep shell audible bookkeeping in
      // sync without issuing a second pause or taking over its audio session.
      keep(await listen<unknown>("kiwix-mobile-audio", (event) => {
        const value = event.payload as { event?: unknown } | null;
        if (value?.event === "interruption-began" || value?.event === "route-unavailable")
          setAudioInterruption((revision) => revision + 1);
      }));
      keep(await listen<unknown>("kiwix-mobile-document-progress", (event) => {
        const value = event.payload as { bytes?: unknown; done?: unknown } | null;
        if (value?.done === true) {
          setDocumentProgress(null);
          return;
        }
        if (value && typeof value.bytes === "number" && Number.isSafeInteger(value.bytes) && value.bytes >= 0)
          setDocumentProgress(value.bytes);
      }));
      keep(await listen<unknown>("kiwix-mobile-error", (event) => {
        const value = event.payload as { operation?: unknown; error?: unknown } | null;
        if (!value || typeof value.error !== "string") return;
        // Retiring a finished reader source is housekeeping. A late native
        // acknowledgement must not paint a red band over the library.
        if (typeof value.operation === "string" &&
            ["speech-retire", "speech-retirement", "speech-reader-retirement", "speech-spelling-retirement"].includes(value.operation)) {
          console.info(`KIWIX_SPEECH_RETIRE ${value.error}`);
          return;
        }
        onError(value.error);
      }));
      if (!cancelled) reconcile(); // Drain cold-start documents and missed lifecycle events.
    }).catch(onError);
    // WebView visibility/focus can survive a missed native event delivery.
    // These notifications grant no authority; revision/nonce checks above do.
    const resume = () => {
      reconcile();
      if (!hydrated.current && !snapshotSuspended.current) void refresh();
    };
    window.addEventListener("focus", resume);
    window.addEventListener("pageshow", resume);
    document.addEventListener("visibilitychange", resume);
    return () => {
      cancelled = true;
      revision++;
      window.clearTimeout(notificationAckRetry);
      mobileExternalSelection.current = null;
      if (mobileManagedImport.current) {
        mobileManagedImport.current = null;
        mobileManagedBusy.current = false;
        if (mounted.current) setImporting(false);
      }
      if (mobileReconcile.current === reconcile) mobileReconcile.current = null;
      window.removeEventListener("focus", resume);
      window.removeEventListener("pageshow", resume);
      document.removeEventListener("visibilitychange", resume);
      for (const dispose of disposers) dispose();
      window.removeEventListener("focus", reconcile);
      window.removeEventListener("pageshow", reconcile);
      document.removeEventListener("visibilitychange", reconcile);
      resumeStorage?.(false);
    };
  }, [flushSession, refresh, onError, open, importPaths, scanPath, requestLibrarySearch]);
  const dismissMobileIncoming = useCallback(async () => {
    const request = mobileIncoming[0];
    // Dismissal only retires a queued native request; it never mutates the
    // library, so private and read-only mode must be able to clear the queue.
    if (!request || mobileIncomingBusy) return;
    setMobileIncomingBusy(true);
    try {
      await api("mobile_dismiss_incoming", { requestId: request.requestId });
      dismissedIncoming.current.add(request.requestId);
      setMobileIncoming((items) => items.filter((item) => item.requestId !== request.requestId));
    } catch (value) {
      onError(value);
    } finally {
      setMobileIncomingBusy(false);
    }
  }, [mobileIncoming, mobileIncomingBusy, onError]);
  const acceptMobileIncoming = useCallback(async (copy: boolean) => {
    const request = mobileIncoming[0];
    const privacy = capturePrivacyContext();
    if (!request || !privacy || mobileIncomingBusy || mobileManagedBusy.current ||
        privacyPending.current || quitRequest.current ||
        (copy && (privacy.privateMode || privacy.readOnly))) return;
    setMobileIncomingBusy(true);
    mobileManagedBusy.current = true;
    setImporting(true);
    let queued = false;
    try {
      const result = await api<ImportResult>(copy ? "mobile_copy_incoming" : "mobile_open_incoming", {
        requestId: request.requestId, privacyGeneration: privacy.generation,
      });
      if (!mounted.current || !isPrivacyPolicyContextCurrent(privacy)) return;
      const state = await api<MobileLifecycleState>("mobile_lifecycle_state");
      if (!state.incoming.some((item) => item.requestId === request.requestId))
        dismissedIncoming.current.add(request.requestId);
      setMobileIncoming(state.incoming.filter((item) => !dismissedIncoming.current.has(item.requestId)));
      mobileManagedImport.current = { kind: "registered", result, privacy };
      queued = true;
      mobileReconcile.current?.();
    } catch (value) {
      onError(value);
    } finally {
      setMobileIncomingBusy(false);
      if (!queued) {
        mobileManagedBusy.current = false;
        setImporting(false);
      }
      setDocumentProgress(null);
    }
  }, [mobileIncoming, mobileIncomingBusy, onError]);
  useEffect(() => {
    const dialog = quitDialog.current;
    if (!dialog) return;
    try {
      if (quitting && !dialog.open) dialog.showModal();
      else if (!quitting && dialog.open) dialog.close();
    } catch (e) {
      onError(e);
    }
  }, [quitting, onError]);
  useEffect(() => {
    const dialog = mobileRecoveryDialog.current;
    if (!dialog) return;
    try {
      if (isMobile && mobileSuspended && mobileRecoveryNeeded && !dialog.open) dialog.showModal();
      else if ((!mobileSuspended || !mobileRecoveryNeeded) && dialog.open) dialog.close();
    } catch (value) {
      onError(value);
    }
  }, [mobileSuspended, mobileRecoveryNeeded, onError]);
  useEffect(() => {
    if (!isDesktop) return;
    let cancelled = false;
    const disposers: (() => void)[] = [];
    function keep(dispose: () => void) {
      if (cancelled) dispose();
      else disposers.push(dispose);
    }
    Promise.all([
      import("@tauri-apps/api/webviewWindow"),
      import("@tauri-apps/api/core"),
    ])
      .then(async ([{ getCurrentWebviewWindow }, { invoke }]) => {
        const nativeWindow = getCurrentWebviewWindow();
        keep(
          await nativeWindow.listen<unknown>(
            "kiwix-quit-cancelled",
            (event) => {
              const payload = event.payload;
              if (
                !payload ||
                typeof payload !== "object" ||
                !("requestId" in payload) ||
                typeof payload.requestId !== "string" ||
                !("error" in payload) ||
                typeof payload.error !== "string"
              )
                return;
              quitEventSerial.current++;
              if (quitRequest.current?.id === payload.requestId) {
                quitRequest.current.resumeStorage?.();
                quitRequest.current = null;
                setQuitting(false);
              }
              onError(new Error(payload.error));
            },
          ),
        );
        keep(
          await nativeWindow.listen<unknown>("kiwix-quit-flush", (event) => {
            const payload = event.payload;
            if (
              !payload ||
              typeof payload !== "object" ||
              !("requestId" in payload) ||
              typeof payload.requestId !== "string" ||
              payload.requestId.length > 64 ||
              quitRequest.current?.id === payload.requestId
            )
              return;
            const requestId = payload.requestId;
            const serial = ++quitEventSerial.current;
            void (async () => {
              // Events cannot authorize a flush on their own: prove the random request
              // is pending for this native window before freezing or touching session data.
              try {
                await invoke("validate_quit_flush", { requestId });
              } catch {
                return;
              }
              if (
                cancelled ||
                serial !== quitEventSerial.current ||
                quitRequest.current
              )
                return;
              const request: { id: string; resumeStorage?: () => void } = {
                id: requestId,
              };
              quitRequest.current = request;
              // Commit pending React tab changes before taking the final session snapshot.
              flushSync(() => setQuitting(true));
              let failure: string | null = null;
              try {
                const resumeStorage = await freezeReaderStorage();
                if (cancelled || quitRequest.current !== request) {
                  resumeStorage();
                  return;
                }
                request.resumeStorage = resumeStorage;
                await flushSession(request);
              } catch (e) {
                failure = errorText(e);
                if (quitRequest.current === request) onError(e);
              }
              if (cancelled || quitRequest.current !== request) return;
              try {
                await invoke("ack_quit_flush", { requestId, error: failure });
              } catch (e) {
                if (quitRequest.current === request) onError(e);
              }
            })();
          }),
        );
      })
      .catch(onError);
    return () => {
      cancelled = true;
      quitEventSerial.current++;
      for (const dispose of disposers) dispose();
    };
  }, [flushSession, onError]);
  useEffect(() => {
    if (!isDesktop) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    let closing = false;
    import("@tauri-apps/api/window")
      .then(async ({ getCurrentWindow }) => {
        const nativeWindow = getCurrentWindow();
        const unlisten = await nativeWindow.onCloseRequested(async (event) => {
          event.preventDefault();
          if (closing || quitRequest.current) return;
          closing = true;
          const request: { id: string; resumeStorage?: () => void } = {
            id: `close:${crypto.randomUUID()}`,
          };
          quitRequest.current = request;
          flushSync(() => setQuitting(true));
          const cancelClose = () => {
            if (quitRequest.current === request) {
              quitRequest.current = null;
              setQuitting(false);
            }
            closing = false;
          };
          let discard = false;
          let resumeStorage: (() => void) | undefined;
          try {
            resumeStorage = await freezeReaderStorage();
            request.resumeStorage = resumeStorage;
            await flushSession(request);
          } catch (e) {
            onError(e);
            discard = await confirmAction(
              t(
                "The current session could not be saved. Close this window anyway? Unsaved tab changes will be lost.",
              ),
            );
            if (!discard) {
              resumeStorage?.();
              cancelClose();
              return;
            }
          }
          try {
            await waitForSessionOperation(api("close_window"));
          } catch (e) {
            onError(e);
            if (
              !discard &&
              !(await confirmAction(
                t(
                  "The current session could not be saved. Close this window anyway? Unsaved tab changes will be lost.",
                ),
              ))
            ) {
              resumeStorage?.();
              cancelClose();
              return;
            }
          }
          try {
            await nativeWindow.destroy();
          } catch (e) {
            resumeStorage?.();
            cancelClose();
            onError(e);
          }
        });
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch(onError);
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [onError, flushSession]);
  const applicationShortcut = useCallback(
    (action: ShellShortcut) => {
      if (quitRequest.current || document.querySelector(".app-shell[hidden], dialog[open]")) return;
      const selection = TAB_SELECTION_ACTIONS.indexOf(
        action as (typeof TAB_SELECTION_ACTIONS)[number],
      );
      if (selection >= 0) {
        const tab = latestSession.current.tabs[selection];
        if (tab) selectTab(tab.id, true);
        return;
      }
      if (action === "new-tab") newTab();
      else if (action === "reopen-tab") undoClose();
      else if (
        action === "close-tab" &&
        pane === "reader" &&
        latestSession.current.activeTab
      )
        closeTab(latestSession.current.activeTab);
      else if (action === "import") void importFiles();
      else if (action === "reading-list") navigate("bookmarks");
      else if (action === "settings") navigate("settings");
      else if (action === "library-search" || action === "search-scope")
        requestLibraryCommand(action === "library-search" ? "search" : "scope");
      else if (action === "new-window" && isDesktop) void newWindow();
      else if (action === "restore-session" && isDesktop)
        void restorePreviousSession();
      else if (
        (action === "next-tab" || action === "previous-tab") &&
        latestSession.current.tabs.length
      ) {
        const session = latestSession.current;
        const index = session.tabs.findIndex(
          (tab) => tab.id === session.activeTab,
        );
        const next =
          (index + (action === "previous-tab" ? -1 : 1) + session.tabs.length) %
          session.tabs.length;
        selectTab(session.tabs[next].id, true);
      }
    },
    [
      selectTab,
      pane,
      navigate,
      importFiles,
      newTab,
      newWindow,
      restorePreviousSession,
      undoClose,
      closeTab,
      requestLibraryCommand,
    ],
  );
  useEffect(() => {
    if (!isDesktop) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    import("@tauri-apps/api/webviewWindow")
      .then(({ getCurrentWebviewWindow }) =>
        getCurrentWebviewWindow().listen<unknown>("kiwix-shortcut", (event) => {
          if (
            typeof event.payload === "string" &&
            isShellShortcut(event.payload)
          )
            applicationShortcut(event.payload);
        }),
      )
      .then((unlisten) => {
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch(onError);
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [applicationShortcut, onError]);
  useEffect(() => {
    if (!isMobile) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(async ({ listen }) => {
      const unlisten = await listen<unknown>("kiwix-mobile-shortcut", (event) => {
        const payload = event.payload as { action?: unknown } | null;
        if (!payload || typeof payload.action !== "string") return;
        const action = payload.action;
        if (cancelled || document.visibilityState !== "visible" || !document.hasFocus() ||
            document.querySelector(".app-shell[hidden]")) return;
        void (async () => {
          const state = await api<MobileLifecycleState>("mobile_lifecycle_state");
          if (cancelled || document.visibilityState !== "visible" || !document.hasFocus() ||
              document.querySelector(".app-shell[hidden]") || !state.active || !state.interactive) return;
          if (state.retirementRevision !== mobileRetirementRevision.current) {
            mobileReconcile.current?.();
            return;
          }
          const dialogs = document.querySelectorAll<HTMLDialogElement>("dialog[open]");
          if (dialogs.length) {
            if (action === "escape") {
              // Native capture consumes Escape before DOM dispatch. Preserve
              // the current dialog's cancel/busy policy, without also sending
              // Escape to the reader underneath it.
              const focused = document.activeElement?.closest<HTMLDialogElement>("dialog[open]");
              const dialog = focused ?? dialogs[dialogs.length - 1];
              if (dialog.dispatchEvent(new Event("cancel", { cancelable: true })))
                dialog.close();
            }
            return;
          }
          if (isShellShortcut(action) && action !== "new-window")
            applicationShortcut(action);
          else if (action === "search" && pane !== "reader")
            setGlobalSearch(true);
          else if (pane === "reader" && ["search", "find", "print", "save", "bookmark",
            "zoom-in", "zoom-out", "zoom-reset", "back", "forward", "fullscreen", "escape",
            "speech-slower", "speech-faster"].includes(action))
            mobileReaderShortcut.current?.(action);
        })().catch(onError);
      });
      if (cancelled) unlisten();
      else dispose = unlisten;
    }).catch(onError);
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [applicationShortcut, pane, onError]);
  // Native menu labels only change with the resolved locale; rebuilding ~50
  // catalog lookups on every poll/progress render is wasted work.
  const nativeMenuLabels = useMemo(() => ({
    file: t("File"),
    tabs: t("Tabs"),
    newTab: t("New tab"),
    newWindow: t("New window"),
    restoreSession: t("Restore previous session"),
    import: t("Import archives"),
    home: t("Archive home"),
    settings: t("Settings"),
    readingList: t("Reading list"),
    librarySearch: t("Filter your library"),
    searchScope: t("Search options"),
    closeTab: t("Close tab"),
    reopenTab: t("Reopen closed tab"),
    nextTab: t("Next tab"),
    previousTab: t("Previous tab"),
    selectTabs: TAB_SELECTION_ACTIONS.map((_, index) =>
      t("Select tab {number}", { number: formatNumber(index + 1) }),
    ),
    edit: t("Edit"),
    undo: t("Undo"),
    redo: t("Redo"),
    cut: t("Cut"),
    copy: t("Copy"),
    paste: t("Paste"),
    selectAll: t("Select all"),
    quit: t("Quit"),
    reader: {
      search: t("Search books"),
      find: t("Find in article"),
      print: t("Print / save as PDF"),
      save: t("Save article as PDF / original media"),
      bookmark: t("Bookmark and notes"),
      "zoom-in": t("Zoom in"),
      "zoom-out": t("Zoom out"),
      "zoom-reset": t("Reset zoom"),
      back: t("Back"),
      forward: t("Forward"),
      random: t("Random article"),
      toc: t("Table of contents"),
      fullscreen: t("Fullscreen"),
      "speech-slower": t("Read slower"),
      "speech-faster": t("Read faster"),
    },
  }), [localeContext]);
  useEffect(() => {
    if (!isDesktop || !snapshot) return;
    import("@tauri-apps/api/core")
      .then(({ invoke }) =>
        invoke("update_native_menu", { labels: nativeMenuLabels }),
      )
      .catch(onError);
  }, [nativeMenuLabels, !!snapshot, onError]);
  useEffect(() => {
    const showSearch = () => {
      if (
        pane !== "reader" &&
        !quitRequest.current &&
        !document.querySelector("dialog[open]")
      )
        setGlobalSearch(true);
    };
    if (!isDesktop) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    import("@tauri-apps/api/webviewWindow")
      .then(({ getCurrentWebviewWindow }) =>
        getCurrentWebviewWindow().listen<unknown>(
          "kiwix-reader-shortcut",
          (event) => {
            if (event.payload === "search") showSearch();
          },
        ),
      )
      .then((unlisten) => {
        if (cancelled) unlisten();
        else dispose = unlisten;
      })
      .catch(onError);
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [pane, onError]);
  useEffect(() => {
    window.addEventListener("dragend", endTabDrag);
    window.addEventListener("drop", endTabDrag);
    return () => {
      window.removeEventListener("dragend", endTabDrag);
      window.removeEventListener("drop", endTabDrag);
    };
  }, [endTabDrag]);
  useEffect(() => {
    function key(event: KeyboardEvent) {
      if (!event.isTrusted || isDesktop) return; // Desktop accelerators are owned by the native menu, including with iframe focus.
      if (quitRequest.current || document.querySelector("dialog[open]")) return;
      let action: ShellShortcut | undefined;
      if (
        event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.shiftKey &&
        /^Digit[0-9]$/.test(event.code)
      ) {
        const number = Number(event.code.slice(-1));
        action = TAB_SELECTION_ACTIONS[number === 0 ? 9 : number - 1];
      } else if (
        event.ctrlKey &&
        !event.altKey &&
        !event.metaKey &&
        event.key === "Tab"
      )
        action = event.shiftKey ? "previous-tab" : "next-tab";
      else if ((event.metaKey || event.ctrlKey) && !event.altKey) {
        const key = event.key.toLowerCase();
        if (key === "o") action = "import";
        else if (key === "t")
          action = event.shiftKey ? "reopen-tab" : "new-tab";
        else if (key === "w") action = "close-tab";
        else if (key === "n" && event.shiftKey && isDesktop)
          action = "new-window";
        else if (key === "k" && pane !== "reader") {
          event.preventDefault();
          setGlobalSearch(true);
        }
      }
      if (action) {
        event.preventDefault();
        applicationShortcut(action);
      }
    }
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [applicationShortcut, pane]);
  // One index per snapshot replaces per-render library walks.
  const bookIndex = useMemo(() => {
    const byId = new Map<string, Book>();
    const availableInstallations = new Set<string>();
    for (const book of snapshot?.books ?? []) {
      if (!byId.has(book.id)) byId.set(book.id, book);
      if (book.available) availableInstallations.add(book.installationId);
    }
    return { byId, availableInstallations };
  }, [snapshot?.books]);
  const completedVisible = useMemo(
    () =>
      completedDownloads.filter((book) =>
        bookIndex.availableInstallations.has(book.installationId),
      ),
    [completedDownloads, bookIndex],
  );
  const currentBook = current?.bookId
    ? bookIndex.byId.get(current.bookId)
    : undefined;
  // Keep hidden chrome bound to the document, not to lifecycle revisions:
  // a foreground/interaction reconcile after a scroll must not pull it back.
  const readerChromeOwner = `${current?.id || ""}:${currentBook?.installationId || ""}:${workspaceEpoch}`;
  const readerChromeHidden = !isDesktop && narrowReaderChrome.matches &&
    pane === "reader" && !quitting &&
    !mobileSuspended && mobileInteraction.interactive && !mobileTabsOpen &&
    readerChrome?.owner === readerChromeOwner && readerChrome.hidden;
  const navigation: {
    id: Pane;
    label: string;
    icon: typeof LibraryIcon;
    count?: number;
  }[] = [
    {
      id: "library",
      label: t("Library"),
      icon: LibraryIcon,
      count: snapshot?.books.length,
    },
    { id: "catalog", label: t("Discover"), icon: Compass },
    { id: "maps", label: t("Maps"), icon: Globe2 },
    { id: "downloads", label: t("Downloads"), icon: Download },
    { id: "nearby", label: t("Nearby"), icon: Radio },
    {
      id: "bookmarks",
      label: t("Reading list"),
      icon: Bookmark,
      count: snapshot?.bookmarks.length,
    },
  ];
  return (
    <>
      {/* Do not toggle inert above archive frames: older WebViews lose their
          native accessibility subtree. Inline display also overrides the
          shell's author-level flex layout while hidden. */}
      <div
        className={`app-shell${isMobile ? " is-mobile" : ""}${pane === "reader" ? " is-reading" : ""}${readerChromeHidden ? " reader-chrome-hidden" : ""}`}
        hidden={quitting || mobileSuspended}
        style={{ display: quitting || mobileSuspended ? "none" : undefined }}
        aria-busy={quitting || mobileSuspended}
        onDragOver={(event) => {
          event.preventDefault();
          if (tabDrag.current) {
            tabDropRef.current = null;
            setTabDrop(null);
            event.dataTransfer.dropEffect = "move";
            return;
          }
          if (
            quitRequest.current ||
            !event.dataTransfer.types.includes("Files")
          )
            return;
          event.dataTransfer.dropEffect = "copy";
        }}
        onDrop={(event) => {
          event.preventDefault();
          if (tabDrag.current) {
            endTabDrag();
            return;
          }
          if (privacyPending.current || quitRequest.current) return;
          if (isNative) return;
          const paths = Array.from(event.dataTransfer.files)
            .map((file) => (file as File & { path?: string }).path)
            .filter((path): path is string => !!path);
          if (paths.length) void importPaths(paths);
          else if (event.dataTransfer.files.length)
            onError(
              new Error(
                t(
                  "Browser security hides local file paths. Use Import archives to provide the archive path on the host device.",
                ),
              ),
            );
        }}
      >
        <a className="skip-link" href="#main-content">
          {t("Skip to content")}
        </a>
        <aside className="sidebar" inert={readerChromeHidden} aria-hidden={readerChromeHidden || undefined}>
          {usesOverlayTitleBar && (
            <div className="titlebar-drag" data-tauri-drag-region aria-hidden="true" />
          )}
          <button
            className="brand"
            onClick={() => navigate("library")}
            aria-label={t("Kiwix home")}
          >
            <span className="brand-mark">
              <BookOpen size={20} strokeWidth={1.6} />
            </span>
            <span>
              <strong>{t("kiwix")}</strong>
            </span>
          </button>
          <nav className="main-nav" aria-label={t("Main navigation")}>
            {navigation.map((item) => (
              <button
                key={item.id}
                className={pane === item.id ? "active" : ""}
                onClick={() => {
                  if (item.id === "nearby") setNearbyDiscoveryIntent((intent) => intent + 1);
                  navigate(item.id);
                }}
                aria-label={item.label}
                title={item.label}
                aria-current={pane === item.id ? "page" : undefined}
              >
                <item.icon size={19} strokeWidth={1.6} />
                <span>{item.label}</span>
                {!!item.count && <small>{formatNumber(item.count)}</small>}
              </button>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <button
              className={
                pane === "settings"
                  ? "sidebar-settings active"
                  : "sidebar-settings"
              }
              onClick={() => navigate("settings")}
              aria-label={t("Settings")}
              title={t("Settings")}
              aria-current={pane === "settings" ? "page" : undefined}
            >
              <SettingsIcon size={18} />
              <span>{t("Settings")}</span>
            </button>
          </div>
        </aside>
        <div className="main-shell">
          <header
            className="workspace-bar"
            inert={readerChromeHidden}
            aria-hidden={readerChromeHidden || undefined}
            data-tauri-drag-region={usesOverlayTitleBar ? "deep" : undefined}
          >
            <div className="workspace-actions">
              <button
                className="subtle global-search"
                disabled={!snapshot}
                onClick={() => {
                  if (
                    !quitRequest.current &&
                    !document.querySelector("dialog[open]")
                  )
                    setGlobalSearch(true);
                }}
                aria-label={t("Search tabs, reading list and history")}
                title={t("Search tabs, reading list and history")}
              >
                <Search size={17} />
                <span className="global-search-label">{t("Search")}</span>
              </button>
              <button
                className="icon-button mobile-tabs-trigger"
                data-mobile-tabs-trigger="true"
                aria-label={t("Open reader tabs")}
                title={t("Open reader tabs")}
                aria-expanded={mobileTabsOpen}
                aria-controls="reader-tabs"
                onClick={() => setMobileTabsOpen((open) => !open)}
              >
                <PanelsTopLeft size={18} />
                <span>{formatNumber(tabs.length)}</span>
              </button>
              {snapshot?.settings.privateMode && (
                <span className="badge">{t("Private reading")}</span>
              )}
              {snapshot?.settings.readOnly && (
                <span className="badge warning">{t("Read-only")}</span>
              )}
              {isDesktop && (
                <button
                  className="icon-button"
                  onClick={newWindow}
                  aria-label={t("New window")}
                  title={t("New window")}
                >
                  <AppWindow size={16} />
                </button>
              )}
              <button
                className="icon-button"
                disabled={importing || snapshot?.settings.readOnly}
                onClick={importFiles}
                aria-label={t("Add files")}
                title={t("Add files")}
              >
                <Plus size={16} />
              </button>
            </div>
          </header>
          <div
            id="reader-tabs"
            className={`tab-strip${mobileTabsOpen ? " mobile-tabs-open" : ""}`}
            onKeyDown={(event) => {
              if (event.key === "Escape" && mobileTabsOpen) {
                event.preventDefault();
                dismissMobileTabs();
              }
            }}
            role="navigation"
            aria-label={t("Open reader tabs")}
          >
            <div className="mobile-tabs-heading">
              <strong>{t("Open reader tabs")}</strong>
              <button
                className="icon-button"
                aria-label={t("Search tabs, reading list and history")}
                title={t("Search tabs, reading list and history")}
                disabled={!snapshot}
                onClick={() => {
                  setMobileTabsOpen(false);
                  if (!quitRequest.current && !document.querySelector("dialog[open]"))
                    setGlobalSearch(true);
                }}
              >
                <Search size={18} />
              </button>
              <button className="icon-button" aria-label={t("Close")} onClick={dismissMobileTabs}>
                <X size={18} />
              </button>
            </div>
            <div className="tab-scroll">
              {tabs.map((tab) => (
                <div
                  key={tab.id}
                  className={`reader-tab ${activeTab === tab.id && pane === "reader" ? "active" : ""} ${tabDrop?.id === tab.id ? (tabDrop.after ? "tab-drop-after" : "tab-drop-before") : ""}`}
                  onMouseDown={(event) => {
                    if (event.button === 1) event.preventDefault();
                  }}
                  onAuxClick={(event) => {
                    if (event.button === 1 && event.isTrusted) {
                      event.preventDefault();
                      closeTab(tab.id);
                    }
                  }}
                  onDragOver={(event) => {
                    if (
                      !event.isTrusted ||
                      !tabDrag.current ||
                      quitRequest.current
                    )
                      return;
                    event.preventDefault();
                    event.stopPropagation();
                    event.dataTransfer.dropEffect = "move";
                    const bounds = event.currentTarget.getBoundingClientRect();
                    const rightHalf =
                      event.clientX > bounds.left + bounds.width / 2;
                    const cache = tabDirectionCache.current;
                    if (cache.rtl !== rtl) {
                      cache.rtl = rtl;
                      cache.directions = new WeakMap<Element, "rtl" | "ltr">();
                    }
                    let direction = cache.directions.get(event.currentTarget);
                    if (!direction) {
                      direction =
                        getComputedStyle(event.currentTarget).direction === "rtl"
                          ? "rtl"
                          : "ltr";
                      cache.directions.set(event.currentTarget, direction);
                    }
                    const after =
                      direction === "rtl" ? !rightHalf : rightHalf;
                    if (
                      tabDropRef.current?.id === tab.id &&
                      tabDropRef.current.after === after
                    )
                      return;
                    tabDropRef.current = { id: tab.id, after };
                    setTabDrop({ id: tab.id, after });
                  }}
                  onDrop={(event) => {
                    if (!tabDrag.current) return;
                    event.preventDefault();
                    event.stopPropagation();
                    const owner = tabDrag.current;
                    const valid =
                      event.isTrusted &&
                      event.dataTransfer.getData(TAB_DRAG_TYPE) === owner.nonce;
                    endTabDrag();
                    if (!valid) return;
                    const session = latestSession.current;
                    const from = session.tabs.findIndex(
                      (item) => item.id === owner.id,
                    );
                    const target = session.tabs.findIndex(
                      (item) => item.id === tab.id,
                    );
                    if (from < 0 || target < 0) return;
                    const bounds = event.currentTarget.getBoundingClientRect();
                    const rightHalf =
                      event.clientX > bounds.left + bounds.width / 2;
                    const after =
                      getComputedStyle(event.currentTarget).direction === "rtl"
                        ? !rightHalf
                        : rightHalf;
                    const insertion = target + (after ? 1 : 0);
                    reorderTab(
                      owner.id,
                      insertion - (from < insertion ? 1 : 0),
                    );
                  }}
                >
                  <button
                    className="tab-select"
                    ref={(button) => {
                      if (button) tabButtons.current.set(tab.id, button);
                      else tabButtons.current.delete(tab.id);
                    }}
                    draggable
                    onDragStart={(event) => {
                      if (
                        !event.isTrusted ||
                        privacyPending.current ||
                        quitRequest.current
                      ) {
                        event.preventDefault();
                        return;
                      }
                      const nonce = crypto.randomUUID();
                      tabDrag.current = { id: tab.id, nonce };
                      setDragging(false);
                      event.dataTransfer.effectAllowed = "move";
                      event.dataTransfer.setData(TAB_DRAG_TYPE, nonce);
                    }}
                    onDragEnd={endTabDrag}
                    onClick={() => {
                      setMobileTabsOpen(false);
                      selectTab(tab.id);
                    }}
                    onKeyDown={(event) => {
                      if (
                        !event.isTrusted ||
                        quitRequest.current ||
                        document.querySelector("dialog[open]")
                      )
                        return;
                      const session = latestSession.current;
                      const position = session.tabs.findIndex(
                        (item) => item.id === tab.id,
                      );
                      if (position < 0) return;
                      const horizontalStep =
                        (event.key === "ArrowLeft") ===
                        (getComputedStyle(event.currentTarget).direction ===
                          "rtl")
                          ? 1
                          : -1;
                      if (
                        event.ctrlKey &&
                        event.shiftKey &&
                        !event.altKey &&
                        !event.metaKey &&
                        (event.key === "ArrowLeft" ||
                          event.key === "ArrowRight")
                      ) {
                        event.preventDefault();
                        moveTab(tab.id, horizontalStep);
                      } else if (
                        !event.ctrlKey &&
                        !event.altKey &&
                        !event.metaKey &&
                        ["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                          event.key,
                        )
                      ) {
                        event.preventDefault();
                        const destination =
                          event.key === "Home"
                            ? 0
                            : event.key === "End"
                              ? session.tabs.length - 1
                              : (position +
                                  horizontalStep +
                                  session.tabs.length) %
                                session.tabs.length;
                        selectTab(session.tabs[destination].id, true);
                      }
                    }}
                    aria-current={
                      activeTab === tab.id && pane === "reader"
                        ? "page"
                        : undefined
                    }
                  >
                    <BookOpen size={14} />
                    <span>{tab.title || t("New tab")}</span>
                  </button>
                  <button
                    className="tab-close"
                    onClick={() => closeTab(tab.id)}
                    aria-label={t("Close {title}", {
                      title: tab.title || t("New tab"),
                    })}
                  >
                    <X size={13} />
                  </button>
                </div>
              ))}
            </div>
            <button
              className="icon-button"
              disabled={
                !activeTab ||
                (rtl
                  ? tabs.findIndex((tab) => tab.id === activeTab) >=
                    tabs.length - 1
                  : tabs.findIndex((tab) => tab.id === activeTab) <= 0)
              }
              onClick={() => activeTab && moveTab(activeTab, rtl ? 1 : -1)}
              aria-label={t("Move tab left")}
              title={t("Move tab left")}
            >
              <ChevronLeft size={15} />
            </button>
            <button
              className="icon-button"
              disabled={
                !activeTab ||
                (rtl
                  ? tabs.findIndex((tab) => tab.id === activeTab) <= 0
                  : tabs.findIndex((tab) => tab.id === activeTab) >=
                    tabs.length - 1)
              }
              onClick={() => activeTab && moveTab(activeTab, rtl ? -1 : 1)}
              aria-label={t("Move tab right")}
              title={t("Move tab right")}
            >
              <ChevronRight size={15} />
            </button>
            <button
              className="icon-button new-tab"
              ref={newTabButton}
              disabled={!snapshot}
              onClick={newTab}
              aria-label={t("New tab")}
              title={t("New tab")}
            >
              <Plus size={17} />
            </button>
            {!!closed.length && (
              <button
                className="icon-button"
                onClick={undoClose}
                aria-label={t("Reopen closed tab")}
                title={t("Reopen closed tab")}
              >
                <RotateCcw size={15} />
              </button>
            )}
          </div>
          {error && (
            <div className="global-error">
              <ErrorNotice
                message={
                  error.severity === "blocking"
                    ? t("The operation could not be completed.") + "\n" + error.message
                    : error.message
                }
              />
              <button
                className="icon-button"
                onClick={() => setError(null)}
                aria-label={t("Dismiss error")}
              >
                <X size={17} />
              </button>
            </div>
          )}
          {!!snapshot?.startupWarnings?.length && !startupNoticeDismissed && (
            <div className="startup-notice" role="status">
              <div>
                <strong>{t("Some saved items need attention.")}</strong>
                <p>{snapshot.startupWarnings.join("\n")}</p>
              </div>
              <button
                className="icon-button"
                onClick={() => setStartupNoticeDismissed(true)}
                aria-label={t("Dismiss")}
              >
                <X size={17} />
              </button>
            </div>
          )}
          {importing && (
            <div className="import-progress" role="status">
              <Upload size={16} />
              {t("Importing archives… Your original files stay in place.")}
            </div>
          )}
          {pane !== "downloads" && !mobileSuspended && completedVisible.length > 0 && (
            <aside className="download-completion" aria-label={t("Downloads")}>
              <div role="status">{t("Completed")}</div>
              <div className="download-completion-actions">
                {completedVisible.map((book) => (
                  <button key={book.installationId} className="primary" onClick={() => {
                    setCompletedDownloads((previous) => previous.filter((entry) => entry.installationId !== book.installationId));
                    open({ bookId: book.id, path: "", title: book.title, scroll: 0 });
                  }}>
                    <BookOpen size={20} />{t("Open {title}", { title: book.title })}
                  </button>
                ))}
                <button className="icon-button" aria-label={t("Close")} onClick={() => setCompletedDownloads([])}><X size={20} /></button>
              </div>
            </aside>
          )}
          <main
            id="main-content"
            tabIndex={-1}
            className={
              pane === "reader" ? "main-content reader-content" : "main-content"
            }
          >
            {!snapshot ? (
              initialError ? (
                <div className="startup-state">
                  <ErrorNotice
                    message={
                      t("Could not connect to the local library service.") +
                      "\n" +
                      initialError
                    }
                    onRetry={refresh}
                  />
                </div>
              ) : (
                <Loading label={t("Opening your workbench…")} />
              )
            ) : (
              <>
                {pane === "library" && (
                  <Library
                    {...{
                      books: snapshot.books,
                      settings: snapshot.settings,
                      onOpen: open,
                      onRefresh: refresh,
                      onError,
                    }}
                    importing={importing}
                    onImport={importFiles}
                    onScan={scan}
                    onCopyImport={importCopies}
                    onCopyFolder={importFolderCopy}
                    onExternalSelected={openExternalSelection}
                    onDiscover={() => setPane("catalog")}
                    onHistory={() => setPane("history")}
                    onNearby={() => {
                      setNearbyDiscoveryIntent((intent) => intent + 1);
                      setPane("nearby");
                    }}
                    command={libraryCommand}
                    onCommandHandled={clearLibraryCommand}
                  />
                )}
                <Suspense fallback={<Loading label={t("Please wait…")} />}>
                {pane === "catalog" && (
                  <Catalog
                    settings={snapshot.settings}
                    privacyGeneration={snapshot.privacyGeneration}
                    books={snapshot.books}
                    onError={onError}
                    onDownload={focusDownload}
                    onOpenBook={(book) =>
                      open({
                        bookId: book.id,
                        path: book.mainPath,
                        title: book.title,
                        scroll: 0,
                      })
                    }
                  />
                )}
                {pane === "downloads" && (
                  <Downloads
                    settings={snapshot.settings}
                    books={snapshot.books}
                    onError={onError}
                    onRefresh={refresh}
                    focusTarget={downloadTarget}
                    notificationAction={downloadNotificationActions[0] ?? null}
                    notificationInteractive={!quitting && !mobileSuspended && mobileInteraction.interactive}
                    notificationInteractionRevision={mobileInteraction.revision}
                    onBeginNotificationAction={beginDownloadNotificationAction}
                    onFinishNotificationAction={finishDownloadNotificationAction}
                    onDownload={focusDownload}
                    onDismissTarget={dismissDownloadTarget}
                    onMaps={() => setPane("maps")}
                    onOpen={(location, target) => {
                      setWatchedDownloads((previous) => previous.filter((entry) =>
                        entry.target.kind !== target.kind || entry.target.id !== target.id));
                      const installed = snapshot.books.find((book) => book.id === location.bookId);
                      if (installed) setCompletedDownloads((previous) => previous.filter((book) =>
                        book.installationId !== installed.installationId));
                      open(location);
                    }}
                  />
                )}
                {pane === "nearby" && (
                  <section className="page nearby-page">
                    <LocalSwarmControls
                      books={snapshot.books}
                      settings={snapshot.settings}
                      discoveryIntent={nearbyDiscoveryIntent}
                      onDownload={focusDownload}
                      onOpen={open}
                      onLibrary={() => setPane("library")}
                      onChange={refresh}
                      onWebSharing={() => {
                        setWebSharingIntent(++webSharingSequence.current);
                        setPane("settings");
                      }}
                    />
                  </section>
                )}
                {pane === "maps" && (
                  <Maps
                    {...{
                      books: snapshot.books,
                      settings: snapshot.settings,
                      privacyGeneration: snapshot.privacyGeneration,
                      onOpen: open,
                      onRefresh: refresh,
                      onError,
                      onDownload: focusDownload,
                      onDismissTarget: dismissDownloadTarget,
                    }}
                    choices={mapChoices}
                    onChoice={(region, language) => {
                      setMapChoices((previous) => ({
                        ...previous,
                        [region]: language,
                      }));
                      // Keep tracking the accepted job, but do not reuse a map
                      // decision after its selection has changed.
                      setDownloadTarget((previous) =>
                        previous?.kind === "map" && previous.request?.regionId === region
                          ? { kind: "map", id: previous.id }
                          : previous);
                      setWatchedDownloads((previous) => previous.map((entry) =>
                        entry.target.kind === "map" && entry.target.request?.regionId === region
                          ? { ...entry, target: { kind: "map", id: entry.target.id } }
                          : entry));
                    }}
                  />
                )}
                {(pane === "bookmarks" || pane === "history") && (
                  <Collections
                    key={pane}
                    kind={pane}
                    readOnly={snapshot.settings.readOnly}
                    privateMode={snapshot.settings.privateMode}
                    privacyGeneration={snapshot.privacyGeneration}
                    onConnect={() => {
                      setPrivateSyncIntent(++privateSyncSequence.current);
                      setPane("settings");
                    }}
                    books={snapshot.books}
                    bookmarks={snapshot.bookmarks}
                    history={snapshot.history}
                    onOpen={open}
                    onRefresh={refresh}
                    onError={onError}
                  />
                )}
                {pane === "settings" && (
                  <Settings
                    settings={snapshot.settings}
                    books={snapshot.books}
                    sharing={snapshot.sharing}
                    notifications={snapshot.notifications}
                    privacyGeneration={snapshot.privacyGeneration}
                    onOpen={open}
                    privateSyncIntent={privateSyncIntent}
                    onPrivateSyncShown={() => setPrivateSyncIntent((current) =>
                      current === privateSyncIntent ? 0 : current)}
                    webSharingIntent={webSharingIntent}
                    onWebSharingShown={() => setWebSharingIntent((current) =>
                      current === webSharingIntent ? 0 : current)}
                    onRefresh={refresh}
                    onError={onError}
                    onDownload={focusDownload}
                  />
                )}
                </Suspense>
                <ReaderHost
                  reader={
                    pane === "reader" && current && !quitting ? (
                      <Reader
                        key={`${current.id}:${currentBook?.installationId || "missing"}:${currentBook?.path || ""}:${readerGeneration}:${snapshot.settings.privateMode}:${snapshot.settings.readOnly}`}
                        tab={current}
                        mobileTabs={{
                          count: tabs.length,
                          expanded: mobileTabsOpen,
                          onToggle: () => setMobileTabsOpen((open) => !open),
                        }}
                        books={snapshot.books}
                        bookmarks={snapshot.bookmarks}
                        settings={snapshot.settings}
                        recentSearches={snapshot.recentSearches}
                        privacyGeneration={snapshot.privacyGeneration}
                        privacyPolicyGeneration={snapshot.privacyPolicyGeneration}
                        suspended={mobileSuspended}
                        mediaRetained={mobileMediaRetained}
                        audioInterruption={audioInterruption}
                        interactive={mobileInteraction.interactive}
                        interactionRevision={mobileInteraction.revision}
                        onReaderChromeHiddenChange={(hidden) => setReaderChrome({ owner: readerChromeOwner, hidden })}
                        onShortcutHandler={(handler) => { mobileReaderShortcut.current = handler; }}
                        onScrollFlushHandler={(handler) => { readerScrollFlush.current = handler; }}
                        onMediaState={reportReaderMedia}
                        onMediaCheckpoint={(authority, positions) => recordMediaCheckpoint(current.id, authority, positions)}
                        readMediaCheckpoint={(authority) => readMediaCheckpoint(current.id, authority)}
                        clearMediaCheckpoint={() => mediaCheckpoint.clear()}
                        workspace={
                          readerWorkspaces.epoch === workspaceEpoch
                            ? readerWorkspaces.tabs[current.id] ||
                              launchReaderWorkspace(current.id)
                            : launchReaderWorkspace(current.id)
                        }
                        onWorkspaceChange={(update) =>
                          updateReaderWorkspace(
                            current.id,
                            workspaceEpoch,
                            update,
                          )
                        }
                        onTabChange={(tab) => {
                          if (
                            privacyPending.current ||
                            workspaceEpoch !== workspaceContext.current.epoch
                          )
                            return;
                          const session = latestSession.current;
                          const existing = session.tabs.find((item) => item.id === tab.id);
                          if (!existing || tab.id !== current.id) return;
                          if (session.activeTab !== current.id || quitRequest.current) {
                            // The reader may flush its trailing scroll after a
                            // tab switch has moved activeTab or after quit has
                            // begun. Accept only a scroll-only final update for
                            // this tab; navigation/history changes still
                            // require an active, non-quitting reader.
                            const scrollOnly =
                              tab.bookId === existing.bookId &&
                              tab.path === existing.path &&
                              tab.title === existing.title &&
                              tab.restricted === existing.restricted &&
                              JSON.stringify(tab.back) === JSON.stringify(existing.back) &&
                              JSON.stringify(tab.forward) === JSON.stringify(existing.forward);
                            if (!scrollOnly) return;
                            const next = session.tabs.map((item) =>
                              item.id === tab.id ? { ...item, scroll: tab.scroll } : item,
                            );
                            latestSession.current = { ...session, tabs: next };
                            setTabs(next);
                            return;
                          }
                          const next = session.tabs.map((item) =>
                            item.id === tab.id ? tab : item,
                          );
                          latestSession.current = { ...session, tabs: next };
                          setTabs(next);
                        }}
                        onRefresh={refresh}
                        onOpenInNewTab={(location, generation) => {
                          if (
                            quitRequest.current ||
                            privacyPending.current ||
                            workspaceEpoch !== workspaceContext.current.epoch
                          )
                            return;
                          if (generation !== snapshot.privacyGeneration) {
                            // The request came from a document that predates
                            // this window's privacy generation. Re-establish
                            // the snapshot so the reader re-bootstraps, then
                            // replay this destination once against the new
                            // generation instead of failing silently.
                            if (pendingReaderOpen.current?.generation !== generation)
                              pendingReaderOpen.current = {
                                location,
                                generation,
                                tabId: current.id,
                              };
                            void refresh();
                            return;
                          }
                          if (latestSession.current.activeTab !== current.id)
                            return;
                          openReaderTab(location);
                        }}
                        onError={(value, severity) => {
                          // Reader owns a visible reconnect/reload notice for
                          // temporary absence of this exact retained source.
                          if (!isProviderUnavailableFor(value, currentBook)) onError(value, severity);
                        }}
                        onOpenLibrary={() => setPane("library")}
                      />
                    ) : null
                  }
                />
              </>
            )}
          </main>
          <footer className="status-bar" inert={readerChromeHidden} aria-hidden={readerChromeHidden || undefined}>
            <span>
              {snapshot
                ? t("{count} archives", { count: snapshot.books.length })
                : t("Connecting to local service…")}
            </span>
            {isMobile && documentProgress !== null && (
              <span role="status">{t("{count} bytes copied", { count: documentProgress })}</span>
            )}
          </footer>
        </div>
        {dragging && (
          <div className="drop-overlay">
            <div>
              <FolderOpen size={46} />
              <h2>{t("Drop archives to add them")}</h2>
              <p>
                {t("Your original files will stay exactly where they are.")}
              </p>
            </div>
          </div>
        )}
        {globalSearch && snapshot && !quitting && (
          <UnifiedSearch
            books={snapshot.books}
            tabs={tabs}
            bookmarks={snapshot.bookmarks}
            history={snapshot.history}
            onSelectTab={selectTab}
            onOpen={open}
            onClose={() => setGlobalSearch(false)}
          />
        )}
      </div>
      {snapshot && !quitting && !mobileSuspended && (
        <CrashReportPrompt
          books={snapshot.books}
          readOnly={snapshot.settings.readOnly}
          onError={onError}
        />
      )}
      <SettingsPolicyConfirmation available={!quitting && !mobileSuspended &&
        (!isMobile || mobileInteraction.interactive)} />
      <SettingsDirectoryMigration available={!quitting && !mobileSuspended &&
        (!isMobile || mobileInteraction.interactive)} />
      {diskSpaceWarning && downloadConsentAllowed.current && (
        <Modal title={t("Review low storage warning")} onClose={() => finishDiskSpaceConfirmation(false)}>
          <p className="disk-space-message">{t(
            "This download would leave less than 1 GB of free space in {directory}.\nAvailable: {available}. Remaining download: {remaining}.\nContinue anyway?",
            {
              directory: isIOS ? t("Kiwix storage") : diskSpaceWarning.directory,
              available: formatBytes(diskSpaceWarning.availableBytes),
              remaining: formatBytes(diskSpaceWarning.remainingBytes),
            },
          )}</p>
          <div className="modal-actions">
            <button autoFocus onClick={() => finishDiskSpaceConfirmation(false)}>{t("Cancel")}</button>
            <button className="primary" onClick={(event) => {
              if (event.isTrusted) finishDiskSpaceConfirmation(true);
            }}>{t("Continue")}</button>
          </div>
        </Modal>
      )}
      {isMobile && (
        // Opened only after a failed mobile resume (see the effect that calls
        // showModal); desktop never shows it, so it is not mounted there.
        <dialog
          ref={mobileRecoveryDialog}
          className="modal"
          aria-label={t("Try again")}
          onCancel={(event) => event.preventDefault()}
        >
          <ErrorNotice message={error?.message || initialError || t("Could not connect to the local library service.")} onRetry={() => {
            retryFailedSettings();
            mobileReconcile.current?.();
          }} />
        </dialog>
      )}
      {isMobile && !mobileSuspended && snapshot && mobileIncoming[0] && (
        <IncomingArchiveDialog
          key={mobileIncoming[0].requestId}
          request={mobileIncoming[0]}
          sessionOnly={snapshot.settings.privateMode || snapshot.settings.readOnly}
          busy={mobileIncomingBusy || importing}
          copiedBytes={documentProgress}
          onCancel={() => void dismissMobileIncoming()}
          onOpenOriginals={() => void acceptMobileIncoming(false)}
          onSaveCopy={() => void acceptMobileIncoming(true)}
          onChooseFiles={() => {
            void pickArchives(false, false).then((opened) => {
              if (opened) void dismissMobileIncoming();
            });
          }}
        />
      )}
      <dialog
        ref={quitDialog}
        className="modal"
        aria-label={t("Quit")}
        onCancel={(event) => event.preventDefault()}
      >
        <Loading label={t("Please wait…")} />
      </dialog>
    </>
  );
}
