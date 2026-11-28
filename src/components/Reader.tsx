import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ButtonHTMLAttributes,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ArrowUpRight,
  Bookmark as BookmarkIcon,
  BookOpen,
  Camera,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleHelp,
  Download,
  Ellipsis,
  ExternalLink,
  FileText,
  Home,
  Library as LibraryIcon,
  List,
  LocateFixed,
  LockKeyhole,
  Maximize,
  Minimize,
  Minus,
  Pause,
  Pin,
  Play,
  Plus,
  Printer,
  Search,
  Share2,
  ShieldAlert,
  ShieldCheck,
  Shuffle,
  Square,
  Type,
  Volume2,
  X,
} from "lucide-react";
import {
  api,
  capturePrivacyContext,
  createSearchRequest,
  getReaderPreferences,
  isDesktop,
  isMacos,
  isMobile,
  isNative,
  isWindows,
  isAndroid,
  isIOS,
  isPrivacyContextCurrent,
  saveMhtml,
  saveArticlePdf,
  printArticlePdf,
  shareArticlePdf,
  setReaderRestricted,
  canShareArticlePdf,
  canPrintArticlePdf,
  saveBookZoom,
  saveReaderPreferences,
  saveSpeechPreference,
  saveResource,
  isPrivacyTransitionError,
  subscribeReaderPreferences,
  type ReaderAppearance,
  type PrivacyContext,
  type SearchRequest,
} from "../api";
import { formatNumber, getLocale, languageName, normalizeLocale, t, useLocale } from "../i18n";
import { searchStatusMessage } from "../reader-search-status";
import {
  adjustedSpeechRate, archiveSpeechLanguages, matchingSpeechVoices,
  selectedSpeechVoice, speechPreference,
} from "../speech-preferences";
import { filterSpeechVoices } from "../speech-voice-filter";
import KokoroDownloadPrompt from "./KokoroDownloadPrompt";
import { useNativeSpeech } from "../native-speech";
import {
  sensorDecisionWriteOutcome,
  sensorRequestDisposition,
  type SensorDecision,
} from "../sensor-policy";
import {
  canNavigateReaderStorage,
  discardQuarantinedReaderStorage,
  quarantineReaderStorage,
  registerReaderStorage,
  retireReaderDocuments,
  updateReaderStorageLocation,
  waitForReaderStorageNavigation,
} from "../reader-storage";
import type {
  Book,
  Bookmark,
  Highlight,
  ReaderLocation,
  SearchHit,
  SearchResults,
  Settings,
  SpellingIndexStatus,
  SpeechPreference,
  Tab,
  TabLocation,
} from "../types";
import {
  ERROR_SITES,
  isSuspendedError,
  isProviderUnavailableFor,
  shouldRenderError,
  type ErrorSeverity,
  type ProviderDocumentError,
} from "../error-visibility";
import {
  validMediaPlaybackPositions,
  type MediaCheckpointAuthority,
  type MediaPlaybackPosition,
} from "../reader-media-checkpoint";
import { errorText } from "./UI";
import "./reader.css";

export type ReaderProps = {
  tab: Tab;
  books: Book[];
  bookmarks: Bookmark[];
  settings: Settings;
  recentSearches: string[];
  privacyGeneration: number;
  privacyPolicyGeneration: number;
  interactive: boolean;
  interactionRevision: number;
  suspended?: boolean;
  /** Root-owned background exemption; never authorizes IPC while suspended. */
  mediaRetained?: boolean;
  /** Native audio interruption/route-loss revision; each bump stops all media. */
  audioInterruption?: number;
  mobileTabs?: { count: number; expanded: boolean; onToggle: () => void };
  /** Compact reading chrome only; never the More dialog or document lifecycle. */
  onReaderChromeHiddenChange?: (hidden: boolean) => void;
  workspace: ReaderWorkspace;
  onWorkspaceChange: (
    update: (previous: ReaderWorkspace) => ReaderWorkspace,
  ) => void;
  onTabChange: (tab: Tab) => void;
  onRefresh: () => void | Promise<unknown>;
  // The frame's privacy generation travels with the request so the root can
  // refuse or re-establish stale authority instead of trusting a session copy.
  onOpenInNewTab: (location: TabLocation, generation: number) => void;
  onError: (error: unknown, severity?: ErrorSeverity, showGlobally?: boolean) => void;
  onOpenLibrary?: () => void;
  onShortcutHandler?: (handler: ((action: string) => void) | null) => void;
  /** The root can flush the trailing scroll before closing or snapshotting the tab. */
  onScrollFlushHandler?: (handler: (() => void) | null) => void;
  // The root may keep a mobile-suspended reader alive for audible media or PiP.
  onMediaState?: (state: {
    audible: boolean;
    pictureInPicture: boolean;
    /** The reader surface is gone; drop the iOS session it latched. */
    release?: boolean;
  }) => void;
  onMediaCheckpoint: (authority: MediaCheckpointAuthority, positions: MediaPlaybackPosition[]) => void;
  readMediaCheckpoint: (authority: MediaCheckpointAuthority) => MediaPlaybackPosition[];
  clearMediaCheckpoint: () => void;
};

type Sensor = "camera" | "location";
/** An archive's request for a photo, bound to the frame that made it. */
type PhotoRequest = {
  frameId: string;
  sessionId: string;
  bookId: string | null;
  origin: string;
  privacyGeneration: number;
  requestId: string;
  accept: string;
  capture: "user" | "environment";
};
/** A picked or captured image, as Rust hands it over (base64 bytes). */
type PickedPhoto = { name: string; mime: string; lastModified?: number | null; data: string };
/** The camera returns JPEG or PNG; only offer it when the input accepts one. */
function cameraAccepts(accept: string) {
  const parts = accept.split(",").map((part) => part.trim().toLowerCase()).filter(Boolean);
  if (!parts.length) return false;
  return parts.some((part) =>
    part === "image/*" ||
    ["image/jpeg", "image/jpg", "image/png", ".jpg", ".jpeg", ".jfif", ".png"].includes(part));
}
type SensorStatus = {
  camera: SensorDecision;
  location: SensorDecision;
  nativeLocation: boolean;
};
type FrameSession = {
  location: ReaderLocation;
  sessionId: string;
  frameId: string;
  contextId: string;
  privacyGeneration: number;
  installationId: string | null;
  status: SensorStatus;
};
type TocEntry = { id: string; title: string; level: number };
type Voice = { name: string; lang: string; voiceURI: string; default: boolean; quality?: number };
type SensorRequest = {
  requestId: string;
  sensor: Sensor;
  options?: {
    enableHighAccuracy?: boolean;
    timeout?: number;
    maximumAge?: number;
  };
  sessionId: string;
};
type PendingSensor = SensorRequest & { nativeId: number; cancelled: boolean };
type Panel =
  "search" | "contents" | "appearance" | "speech" | "bookmark" | null;
export type ApplicationShortcut =
  | "new-tab"
  | "close-tab"
  | "import"
  | "next-tab"
  | "previous-tab"
  | "reopen-tab"
  | "new-window"
  | "restore-session"
  | "reading-list"
  | "settings"
  | "library-search"
  | "search-scope";
export function isApplicationShortcut(
  action: string,
): action is ApplicationShortcut {
  return [
    "new-tab",
    "close-tab",
    "import",
    "next-tab",
    "previous-tab",
    "reopen-tab",
    "new-window",
    "restore-session",
    "reading-list",
    "settings",
    "library-search",
    "search-scope",
  ].includes(action);
}
// Only unsaved edits enter this map; reopening a panel never resets a draft.
type BookmarkDraft = { title: string; note: string; tags: string };
export type ReaderWorkspace = {
  panel: Panel;
  // Only real navigation may create history; pane/document restoration cannot.
  historyNavigationPending: boolean;
  query: string;
  searchMode: "title" | "fulltext";
  searchBooks: string[];
  searchScopeInitialized: boolean;
  searchResults: SearchResults | null;
  searchOffset: number;
  searchError: string;
  /** True while searchError is the empty-query validation sentence. */
  searchValidation: boolean;
  bookmarkDrafts: Partial<Record<string, BookmarkDraft>>;
  // Strict read-only edits stay in this tab's ephemeral, privacy-scoped workspace.
  appearancePatch: Partial<ReaderAppearance>;
  bookZoom: Record<string, number | null>;
  speechLanguages: Record<string, string>;
  searchedQuery: string;
};
export function createReaderWorkspace(): ReaderWorkspace {
  return {
    panel: null,
    historyNavigationPending: false,
    query: "",
    searchMode: "title",
    searchBooks: [],
    searchScopeInitialized: false,
    searchResults: null,
    searchOffset: 0,
    searchError: "",
    searchValidation: false,
    bookmarkDrafts: {},
    appearancePatch: {},
    bookZoom: {},
    speechLanguages: {},
    searchedQuery: "",
  };
}
function bookmarkKey(tab: Pick<Tab, "bookId" | "path">): string {
  return JSON.stringify([tab.bookId, passageDocumentKey(tab.path)]);
}
type PassageAnchor = Pick<
  Highlight,
  "exact" | "prefix" | "suffix" | "start" | "end"
>;
type PassageSelection = PassageAnchor & {
  documentPath: string;
  url: string;
  sessionId: string;
  bookId: string;
};
type PassageDocument = Pick<
  PassageSelection,
  "documentPath" | "url" | "sessionId" | "bookId"
>;
type PassageStatus = "found" | "ambiguous" | "unavailable";
type HighlightChange =
  | { type: "add"; highlight: Highlight }
  | { type: "comment"; id: string; comment: string }
  | { type: "remove"; id: string };
// Annotation operations address the latest native bookmark, never replace a
// cached metadata/annotation snapshot. New bookmarks alone supply a title.
type BookmarkSave = Pick<Bookmark, "bookId" | "path"> &
  (
    | {
        id?: string;
        title: string;
        note: string;
        tags: string[];
        highlightChange?: never;
      }
    | (({ id: string; title?: never } | { id?: never; title: string }) & {
        highlightChange: HighlightChange;
        note?: never;
        tags?: never;
      })
  );
const MAX_DOCUMENT_HIGHLIGHTS = 1000;
const HIGHLIGHT_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
function passageAnchor(value: unknown): value is PassageAnchor {
  return (
    object(value) &&
    text(value.exact, 16_384) &&
    value.exact.length > 0 &&
    text(value.prefix, 128) &&
    text(value.suffix, 128) &&
    typeof value.start === "number" &&
    Number.isInteger(value.start) &&
    value.start >= 0 &&
    typeof value.end === "number" &&
    Number.isInteger(value.end) &&
    value.end <= 100_000_000 &&
    value.end > value.start &&
    value.end - value.start === value.exact.length
  );
}
// Split delimiters before decoding: an encoded literal ?/# is archive path text,
// not a query/route. Decode only once so %252F and %2F remain distinct.
function passageDocumentKey(path: string): string {
  const split = path.search(/[?#]/);
  const pathname = split < 0 ? path : path.slice(0, split);
  const suffix = split < 0 ? "" : path.slice(split);
  try {
    return JSON.stringify([decodeURIComponent(pathname), suffix]);
  } catch {
    return JSON.stringify([pathname, suffix]);
  }
}
// Locations are opaque archive-relative values: a percent-encoded libzim path
// plus an optional archive-authored query/fragment. Canonicalize the path part
// exactly once (decode then re-encode is idempotent for an encoded path), so a
// literal ?/# stays %3F/%23 and never becomes a URL delimiter. Matching native
// encode_path/NON_ALPHANUMERIC keeps bridge, tab and server identities equal.
function canonicalLocation(location: string): string {
  const split = location.search(/[?#]/);
  const pathname = split < 0 ? location : location.slice(0, split);
  const suffix = split < 0 ? "" : location.slice(split);
  try {
    const decoded = decodeURIComponent(
      pathname.replace(/%(?![a-f\d]{2})/gi, "%25"),
    );
    const leading = decoded.match(/^\/+/)?.[0].length ?? 0;
    const encoded = decoded
      .slice(leading)
      .split("/")
      .map((part) =>
        encodeURIComponent(part).replace(
          /[!'()*._~-]/g,
          (character) =>
            `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
        ),
      )
      .join("/");
    return "%2F".repeat(leading) + encoded + suffix;
  } catch {
    return location;
  }
}
// The archive server answers a document navigation to an absent entry with a
// redirect to the fallback page plus this marker. The shell shows a quiet
// notice and keeps the marker out of the tab's archive location.
const MISS_MARKER = "kiwix-miss";
function readerDocumentUrl(value: string): { url: string; persistedUrl: string; missed: boolean } {
  try {
    const parsed = new URL(value);
    if (!parsed.searchParams.has(MISS_MARKER))
      return { url: value, persistedUrl: value, missed: false };
    // Persistence drops the recovery marker, never live document authority:
    // selections, link context and highlights compare against location.href.
    parsed.searchParams.delete(MISS_MARKER);
    return { url: value, persistedUrl: parsed.toString(), missed: true };
  } catch {
    return { url: value, persistedUrl: value, missed: false };
  }
}
function passageDocumentPath(path: string, headingFragment: boolean): string {
  // A heading fragment is document navigation, not part of the document identity.
  const documentPath =
    headingFragment && path.includes("#")
      ? path.slice(0, path.indexOf("#"))
      : path;
  return canonicalLocation(documentPath);
}
function applicablePassages(bookmarks: Bookmark[], document: PassageDocument) {
  const key = passageDocumentKey(document.documentPath);
  const result: { bookmark: Bookmark; highlight: Highlight }[] = [];
  for (const bookmark of bookmarks) {
    if (bookmark.bookId !== document.bookId) continue;
    for (const highlight of bookmark.highlights ?? []) {
      if (passageDocumentKey(highlight.documentPath) !== key) continue;
      result.push({ bookmark, highlight });
      // One extra entry makes the aggregate-limit explanation visible.
      if (result.length > MAX_DOCUMENT_HIGHLIGHTS) return result;
    }
  }
  return result;
}
type LiveArticleSnapshot = { html: string; width: number; height: number; documentUrl: string };
type ReaderCommand =
  | { command: "history-traverse"; requestId: string; url: string; direction: "back" | "forward"; destinations: string[]; y: number }
  | { command: "find"; query: string; requestId: string; url: string; backwards?: boolean }
  | { command: "scroll"; x: number; y: number }
  | { command: "reading-viewport"; deltaTop: number; url: string }
  | { command: "reading-controls"; enabled: boolean; minimumScroll: number }
  | { command: "expand-sections" }
  | { command: "link-previews"; enabled: boolean }
  | { command: "zoom"; value: number }
  | {
      command: "appearance";
      theme: Settings["readerTheme"];
      fontFamily: Settings["fontFamily"];
      fontSize: number;
      width: Settings["readerWidth"];
    }
  | { command: "speak"; text?: string; start?: number; end?: number; rate: number; voice: string; lang: string }
  | { command: "speech-preferences"; rate: number; voice: string; lang: string }
  | { command: "speech-highlight"; offset?: number; clear?: boolean }
  | { command: "speech-highlight-selection"; text: string; start?: number; end?: number }
  | { command: "speech-text"; requestId: string }
  | { command: "export-snapshot"; requestId: string; url: string }
  | { command: "restore-media"; requestId: string; url: string; positions: MediaPlaybackPosition[] }
  | { command: "stop-speech" | "print" }
  | { command: "pause-media" }
  | { command: "pause-video" }
  | { command: "highlights-begin"; url: string; documentPath: string }
  | {
      command: "highlights";
      url: string;
      highlights: (Pick<
        Highlight,
        "id" | "documentPath" | "exact" | "prefix" | "suffix" | "start" | "end"
      > & { bookmarkId: string })[];
    }
  | { command: "highlights-end"; url: string }
  | { command: "highlight-jump"; id: string; bookmarkId: string; url: string }
  | { command: "toc"; id: string };
type HistoryTraversal = {
  requestId: string;
  session: FrameSession;
  source: Tab;
  next: Tab;
  timer?: number;
};

const MAX_TEXT = 120_000;
const MAX_PREVIEW_THUMBNAIL_BYTES = 256 * 1024;
// Mirrors the service's outcome for an archive with no readable document at
// all. That is a limit of the archive, not a failed user action.
const NO_ARTICLES = "This archive has no articles to open.";
type ArticlePreview = {
  title: string;
  path: string;
  excerpt: string;
  mime: string;
  thumbnail: string | null;
};
let nextNativeRequest = 1;
let sensorLifecycle: Promise<unknown> = Promise.resolve();
function lifecycle<T>(operation: () => Promise<T>): Promise<T> {
  const result = sensorLifecycle.catch(() => undefined).then(operation);
  sensorLifecycle = result.catch(() => undefined);
  return result;
}
function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function sensorKey(
  request: Pick<SensorRequest, "sessionId" | "requestId">,
): string {
  return `${request.sessionId}:${request.requestId}`;
}
function sensorDecisionKey(
  installationId: string | null,
  sensor: Sensor,
): string | null {
  return installationId ? `${installationId}:${sensor}` : null;
}
function text(value: unknown, max = 8192): value is string {
  return typeof value === "string" && value.length <= max;
}
function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
function currentLocation(tab: Tab): TabLocation | null {
  return tab.bookId
    ? {
        bookId: tab.bookId,
        path: tab.path,
        title: tab.title,
        scroll: tab.scroll,
      }
    : null;
}
function lastLocationIndex(
  stack: TabLocation[],
  bookId: string | null,
  path: string,
): number {
  for (let index = stack.length - 1; index >= 0; index--)
    if (stack[index].bookId === bookId && stack[index].path === path)
      return index;
  return -1;
}
function sameHistoryStack(left: TabLocation[], right: TabLocation[]): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index++) {
    const a = left[index], b = right[index];
    if (a.bookId !== b.bookId || a.path !== b.path || a.title !== b.title || a.scroll !== b.scroll)
      return false;
  }
  return true;
}
// A tab's back and forward stacks are saved with every session write and
// rendered as one menu entry each; keep the nearest entries only. The bridge
// refuses longer single traversals as well.
const MAX_TAB_HISTORY = 200;
function boundedHistory(stack: TabLocation[]): TabLocation[] {
  return stack.length > MAX_TAB_HISTORY ? stack.slice(-MAX_TAB_HISTORY) : stack;
}
// How long the bridge of an unchanged document may take to prove itself after
// a frame load that no new document announced. Unproven loads lose the session.
const DOCUMENT_PROOF_TIMEOUT = 1000;
// How long a page stays painted while its replacement re-sessions. A real
// bridge answers within the handshake; anything slower is hidden again.
const HELD_PAINT_LIMIT = 3000;
function validExternal(value: string): boolean {
  try {
    return ["http:", "https:", "mailto:"].includes(new URL(value).protocol);
  } catch {
    return false;
  }
}
function isHtml(mime: string): boolean {
  const mediaType = mime.split(";", 1)[0].trim().toLowerCase();
  return mediaType === "text/html" || mediaType === "application/xhtml+xml";
}
function exportName(title: string, mime: string): string {
  const extensions: Record<string, string> = {
    "text/html": "html",
    "application/xhtml+xml": "html",
    "application/pdf": "pdf",
    "text/plain": "txt",
    "text/css": "css",
    "application/json": "json",
    "application/xml": "xml",
    "text/xml": "xml",
    "image/png": "png",
    "image/jpeg": "jpg",
    "image/gif": "gif",
    "image/webp": "webp",
    "image/svg+xml": "svg",
    "image/avif": "avif",
    "image/bmp": "bmp",
    "image/tiff": "tiff",
    "image/x-icon": "ico",
    "audio/mpeg": "mp3",
    "audio/ogg": "ogg",
    "audio/wav": "wav",
    "video/mp4": "mp4",
    "video/webm": "webm",
    "application/epub+zip": "epub",
    "application/zip": "zip",
    "application/octet-stream": "bin",
  };
  const extension =
    extensions[mime.split(";", 1)[0].trim().toLowerCase()] ?? "bin";
  const stem =
    (title || t("Article"))
      .replace(/[<>:"\/\\|?*\x00-\x1f]/g, "_")
      .slice(0, 120)
      .replace(/[. ]+$/, "") || t("Article");
  return stem.toLowerCase().endsWith(`.${extension}`)
    ? stem
    : `${stem}.${extension}`;
}
function articlePath(url: string, location: ReaderLocation): string | null {
  try {
    const target = new URL(url);
    const base = new URL(location.baseUrl);
    if (
      target.origin !== location.origin ||
      base.origin !== location.origin ||
      target.username ||
      target.password ||
      !target.pathname.startsWith(base.pathname)
    )
      return null;
    // The browser serializes the URL path; canonicalize it once so every
    // location entering the shell matches native encode_path, including a
    // literal ?/# that the archive encoded as %3F/%23.
    return canonicalLocation(
      target.pathname.slice(base.pathname.length) + target.search + target.hash,
    );
  } catch {
    return null;
  }
}

function IconButton({
  label,
  children,
  active,
  ...props
}: { label: string; children: ReactNode; active?: boolean } & Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "title" | "aria-label"
>) {
  return (
    <button
      type="button"
      className={`rw-icon${active ? " is-active" : ""}`}
      title={label}
      aria-label={label}
      aria-pressed={active === undefined ? undefined : active}
      {...props}
    >
      <span className="rw-icon-label">{label}</span>
      {children}
    </button>
  );
}

function HistoryDisclosure({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const detailsRef = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const details = detailsRef.current;
      if (details && event.target instanceof Node && !details.contains(event.target)) {
        details.open = false;
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || document.querySelector("dialog[open]")) return;
      const details = detailsRef.current;
      if (details?.open) {
        event.preventDefault();
        event.stopPropagation();
        details.open = false;
        details.querySelector("summary")?.focus();
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);
  return (
    <details
      ref={detailsRef}
      className="rw-history-menu"
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary
        role="button"
        tabIndex={0}
        aria-expanded={open}
        aria-label={label}
      >
        <ChevronDown size={12} />
        <span className="rw-icon-label">{label}</span>
      </summary>
      {children}
    </details>
  );
}
function Dialog({
  title,
  children,
  onClose,
  canRestoreFocus,
  className = "",
  busy = false,
  dismissOnBackdrop = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  canRestoreFocus: () => boolean;
  className?: string;
  busy?: boolean;
  dismissOnBackdrop?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  // Keep the ownership check from this dialog's mount, including its reader
  // tab/interaction revision, rather than adopting a later destination.
  const focusOwner = useRef(canRestoreFocus);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const owner = dialog.closest(".reader-workspace");
    const target = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!dialog.open) dialog.showModal();
    return () => {
      const focused = document.activeElement;
      if (dialog.open) dialog.close();
      if (!owner?.isConnected || !focusOwner.current() ||
          !target?.isConnected || !owner.contains(target) ||
          target.closest("[inert], [hidden]") || target.matches(":disabled") ||
          !target.getClientRects().length ||
          (focused !== document.body && focused !== target && !dialog.contains(focused)) ||
          [...document.querySelectorAll('dialog[open], [role="dialog"]')].some(
            (other) => other !== dialog && other.getClientRects().length,
          )) return;
      if (focused !== target) target.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={`rw-dialog ${className}`}
      aria-label={title}
      aria-busy={busy}
      onClick={(event) => {
        if (dismissOnBackdrop && event.target === event.currentTarget && !busy) {
          const bounds = event.currentTarget.getBoundingClientRect();
          if (event.clientX < bounds.left || event.clientX > bounds.right ||
              event.clientY < bounds.top || event.clientY > bounds.bottom)
            onClose();
        }
      }}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <header>
        <h2>{title}</h2>
        <IconButton label={t("Close dialog")} disabled={busy} onClick={onClose}>
          <X size={18} />
        </IconButton>
      </header>
      {children}
    </dialog>
  );
}

export default function Reader(props: ReaderProps) {
  useLocale();
  const { tab, books, bookmarks, settings } = props;
  const latest = useRef(props);
  latest.current = props;
  const tabRef = useRef(tab);
  tabRef.current = tab;
  const alive = useRef(true);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const findRef = useRef<HTMLInputElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const findBarRef = useRef<HTMLFormElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const findInvoker = useRef<HTMLElement | null>(null);
  const panelInvoker = useRef<HTMLElement | null>(null);
  const activeFrame = useRef<FrameSession | null>(null);
  const retainedActivation = useRef<FrameSession | null>(null);
  const restoredMediaDocument = useRef("");
  const resolvedKey = useRef("");
  const initializedFrame = useRef("");
  // The frame that produced the connection watchdog band, so a late `ready`
  // from that same frame can clear it instead of leaving a false failure.
  const watchdogFrame = useRef("");
  const freshOrigin = useRef(false);
  const openSequence = useRef(0);
  const searchSequence = useRef(0);
  const searchDebounce = useRef<number | undefined>(undefined);
  const searchRequest = useRef<SearchRequest | null>(null);
  const navigationSequence = useRef(0);
  const previewSequence = useRef(0);
  const previewGate = useRef({
    at: 0,
    pending: false,
    timer: undefined as number | undefined,
  });
  const previewQueued = useRef<{
    session: FrameSession;
    path: string;
    sequence: number;
  } | null>(null);
  const pendingSensors = useRef(new Map<string, PendingSensor>());
  // The last explicit choice per installation, kept across a decision reload so
  // a write that failed cannot let the store's old grant auto-allow requests.
  const sensorDecisions = useRef(new Map<string, SensorDecision>());
  // One accepted in-page destination survives a generation change and is
  // replayed exactly once against the re-established generation.
  const pendingNavigation = useRef<{
    generation: number;
    location: TabLocation;
    newTab: boolean;
  } | null>(null);
  // Re-establishment is bounded to one snapshot refresh per observed
  // generation; a refusal never retries itself in a loop.
  const authorityRefresh = useRef({ generation: -1, pending: false });
  const scrollTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const lastVisit = useRef<{ location: TabLocation; privacy: PrivacyContext } | null>(null);
  const historyNavigationPending = useRef(props.workspace.historyNavigationPending);
  const pendingHistoryTraversal = useRef<HistoryTraversal | null>(null);
  const clearHistoryTraversal = useCallback(() => {
    const pending = pendingHistoryTraversal.current;
    if (pending) clearTimeout(pending.timer);
    pendingHistoryTraversal.current = null;
  }, []);
  const scrollPosition = useRef(tab.scroll);
  const restoredDocument = useRef("");
  const [frame, setFrame] = useState<FrameSession | null>(null);
  const displayedFrame = useRef<FrameSession | null>(null);
  displayedFrame.current = frame;
  const storageRegistration = useRef<(() => void) | null>(null);
  const frameNavigation = useRef<{
    node: HTMLIFrameElement;
    frameId: string;
    session: FrameSession;
  } | null>(null);
  // A stable ref is essential: changing reader sessions must not detach the
  // WindowProxy's outgoing storage binding before its real pagehide runs.
  const bindFrame = useCallback((node: HTMLIFrameElement | null) => {
    if (frameRef.current === node) return;
    storageRegistration.current?.();
    storageRegistration.current = null;
    frameNavigation.current = null;
    frameRef.current = node;
  }, []);
  const [reload, setReload] = useState(0);
  const [loading, setLoading] = useState(false);
  const [frameReady, setFrameReady] = useState(false);
  const [error, setError] = useState("");
  const [providerUnavailable, setProviderUnavailable] = useState<ProviderDocumentError | null>(null);
  const { workspace } = props;
  const {
    panel,
    query,
    searchMode,
    searchBooks,
    searchResults,
    searchOffset,
    searchError,
    searchValidation,
  } = workspace;
  const updateWorkspace = useCallback((patch: Partial<ReaderWorkspace>) => {
    latest.current.onWorkspaceChange((previous) => ({ ...previous, ...patch }));
  }, []);
  const setPanel = useCallback(
    (value: Panel | ((previous: Panel) => Panel)) => {
      const previousPanel = latest.current.workspace.panel;
      const nextPanel = typeof value === "function" ? value(previousPanel) : value;
      if (nextPanel && nextPanel !== previousPanel) {
        const target = document.activeElement;
        panelInvoker.current =
          target instanceof HTMLElement && workspaceRef.current?.contains(target) &&
          !panelRef.current?.contains(target) && !findBarRef.current?.contains(target)
            ? target : null;
      }
      latest.current.onWorkspaceChange((previous) => ({
        ...previous,
        panel: typeof value === "function" ? value(previous.panel) : value,
      }));
    },
    [],
  );
  const [toc, setToc] = useState<TocEntry[]>([]);
  const [selection, setSelection] = useState("");
  const [passageSelection, setPassageSelection] =
    useState<PassageSelection | null>(null);
  const [passageSelectionUnavailable, setPassageSelectionUnavailable] =
    useState(false);
  const [passageDocument, setPassageDocument] =
    useState<PassageDocument | null>(null);
  const passageDocumentRef = useRef<PassageDocument | null>(null);
  // Only fresh bounded UUID tokens cross into archives. Legacy bookmark IDs
  // remain shell-owned, and late statuses cannot affect a newer render.
  const passageWire = useRef({
    toWire: new Map<string, string>(),
    fromWire: new Map<string, string>(),
    permitted: new Set<string>(),
    url: "",
    sessionId: "",
  });
  const [passageStatuses, setPassageStatuses] = useState<
    Record<string, PassageStatus>
  >({});
  const [commentDrafts, setCommentDrafts] = useState<Record<string, string>>(
    {},
  );
  const bookmarkWrite = useRef(false);
  // Mobile: every voice comes from the native engine. Desktop: system voices
  // come from the archive frame's speechSynthesis and Kokoro neural voices from
  // the native engine; the reader sees one merged list.
  const [platformVoices, setVoices] = useState<Voice[]>([]);
  const [neuralVoices, setNeuralVoices] = useState<Voice[]>([]);
  const voices = useMemo(
    () => isMobile ? platformVoices : [...platformVoices, ...neuralVoices],
    [platformVoices, neuralVoices],
  );
  const [speechAvailable, setSpeechAvailable] = useState(!isMobile);
  const [speechAvailabilityError, setSpeechAvailabilityError] = useState("");
  const [speaking, setSpeaking] = useState(false);
  // Desktop only: the native (Kokoro) engine's state, separate from the
  // frame's speechSynthesis state so stop/pause always reach the active engine.
  const [nativeSpeaking, setNativeSpeaking] = useState(false);
  const frameMedia = useRef({ audible: false, pictureInPicture: false });
  const nativeSpeechStop = useRef<(() => Promise<void>) | null>(null);
  const [findOpen, setFindOpen] = useState(false);
  const [compact, setCompact] = useState(
    () => window.matchMedia("(max-width: 760px)").matches,
  );
  const [controlsOpen, setControlsOpen] = useState(false);
  const [panHidden, setPanHidden] = useState(false);
  // The token a session's bridge bound while answering reader-init, and the
  // frame it was bound in. An empty token means two different tokens were
  // claimed for the session: neither is trusted.
  const panAuthority = useRef<{
    sessionId: string;
    token: string;
    sequence: number;
    contextId: string;
    origin: string;
  } | null>(null);
  // The bridge announces its private token before archive code runs. Only that
  // first announcement of a document may authorize gestures.
  const gestureHello = useRef<{ frameId: string; token: string } | null>(null);
  // Desktop photo chooser for an archive's camera file input (`capture`).
  // A request is bound to the exact frame, session, archive and privacy
  // generation that made it; any change drops it.
  const photoRequestRef = useRef<PhotoRequest | null>(null);
  const [photoRequest, setPhotoRequest] = useState<PhotoRequest | null>(null);
  const [photoStage, setPhotoStage] = useState<"choose" | "picking" | "camera">("choose");
  const [photoError, setPhotoError] = useState("");
  const [cameraSupported, setCameraSupported] = useState(false);
  const cameraAvailable = cameraSupported && photoRequest !== null && cameraAccepts(photoRequest.accept);
  const photoCancelButton = useRef<HTMLButtonElement>(null);
  // One secret per reader session, sent only in reader-init; a photo request
  // must carry it. The bridge reads reader-init through natives captured before
  // archive scripts ran, removes the secret from the message and stops the
  // event, so archive script cannot read it while the bridge's listener exists.
  // reader-init is therefore only posted to a document a bridge vouched for.
  const photoSecrets = useRef(new Map<string, string>());
  const photoSecretFor = useCallback((sessionId: string) => {
    // Only the session being initialised can still ask; earlier ones are gone.
    for (const known of [...photoSecrets.current.keys()])
      if (known !== sessionId) photoSecrets.current.delete(known);
    let secret = photoSecrets.current.get(sessionId);
    if (!secret) {
      secret = crypto.randomUUID();
      photoSecrets.current.set(sessionId, secret);
    }
    return secret;
  }, []);
  const awaitingGestureHello = useRef(true);
  // WebKit fires the frame's load event for same-document (fragment)
  // navigations too, so a load alone is not a new document. Every document's
  // bridge announces a private token at bootstrap and binds that same token to
  // the session it is handed. The latest announced token therefore names the
  // newest document: when it is not the session's own, the session's document
  // was replaced. Announcement and load arrive in either order, so a load that
  // no other document has announced keeps the session only once the bridge
  // holding it proves, with that token, that it is still listening.
  //
  // What this does and does not establish: it tells a same-document load from
  // a replaced document for pages whose bridge is intact. It is not an
  // authority boundary against archive script. Script in the frame can forge
  // an announcement, and a page that destroys its own bridge listener and then
  // announces itself is indistinguishable from a new document.
  const documentAnnounced = useRef<string | null>(null);
  // A bridge has vouched for the frame's document since reader-init was last
  // posted: it announced itself, or the bridge already initialised in this
  // frame proved it is still listening. reader-init, which carries the session
  // id and the photo secret, is never posted without it.
  const bridgeVouched = useRef(false);
  // The session whose reader-init is withheld until a bridge vouches.
  const pendingInit = useRef("");
  // `nonce` null: the session has no bound token yet, so nothing is asked until
  // its bridge binds one. `kept` false: the probed session already lost
  // admission (or belongs to an earlier open); its proof only vouches.
  const documentProbe = useRef<{
    nonce: string | null;
    sessionId: string;
    frameId: string;
    node: HTMLIFrameElement;
    painted: boolean;
    kept: boolean;
    timer?: number;
  } | null>(null);
  const heldPaintTimer = useRef<number | undefined>(undefined);
  // The frame whose paint is kept while a child navigation re-sessions: the
  // page was already on screen under the previous session, so hiding it for
  // the handshake would only blink. Admission and controls stay withheld.
  const [heldPaint, setHeldPaint] = useState("");
  const readingViewport = useRef<{ sessionId: string; url: string; top: number; hidden: boolean } | null>(null);
  const panCanHide = useRef(false);
  const revealReadingControls = useCallback(() => setPanHidden(false), []);
  // A reader document is on screen in this window: WebKit's under-page
  // background must then match the reader paper, or the gap while a frame's
  // document is replaced flashes the platform's light grey in dark mode. It is
  // declared before the article request so the native side has the surface
  // registered before the first frame can navigate.
  useEffect(() => {
    if (!isDesktop) return;
    void api("reader_backdrop", { reader: true }).catch(() => undefined);
    return () => {
      void api("reader_backdrop", { reader: false }).catch(() => undefined);
    };
  }, []);
  useEffect(() => {
    const media = window.matchMedia("(max-width: 760px)");
    const changed = () => setCompact(media.matches);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);
  useEffect(() => {
    setControlsOpen(false);
  }, [compact, tab.id, props.suspended]);
  const [findQuery, setFindQuery] = useState("");
  const [findFound, setFindFound] = useState<boolean | null>(null);
  const pendingFind = useRef<{
    requestId: string;
    query: string;
    url: string;
    sessionId: string;
    frameId: string;
  } | null>(null);
  const resetFind = useCallback(() => {
    pendingFind.current = null;
    setFindFound(null);
  }, []);
  useLayoutEffect(() => {
    resetFind();
  }, [tab.id, tab.bookId, tab.path, frame?.sessionId, frame?.location.url,
    props.privacyGeneration, props.suspended, props.interactive, reload, resetFind]);
  const canRestoreDialogFocus = useCallback(() => {
    const owner = workspaceRef.current;
    return alive.current && !!owner?.isConnected &&
      latest.current.tab.id === tab.id &&
      latest.current.interactionRevision === props.interactionRevision &&
      !latest.current.suspended && latest.current.interactive &&
      !!owner.getClientRects().length && !owner.closest("[inert], [hidden]");
  }, [tab.id, props.interactionRevision]);
  const restoreReaderFocus = useCallback((invoker: HTMLElement | null, control: string) => {
    const owner = workspaceRef.current;
    if (!alive.current || !owner?.isConnected || latest.current.suspended ||
        !latest.current.interactive || !owner.getClientRects().length ||
        owner.closest("[inert], [hidden]") ||
        document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')) return;
    const usable = (target: HTMLElement | null): target is HTMLElement =>
      !!target?.isConnected && owner.contains(target) &&
      !target.closest("[inert], [hidden]") &&
      !target.matches(":disabled") && !!target.getClientRects().length &&
      getComputedStyle(target).visibility === "visible";
    const target = [invoker,
      ...owner.querySelectorAll<HTMLElement>(`[data-reader-control="${control}"]`),
      frameRef.current,
      ...owner.querySelectorAll<HTMLElement>('[data-reader-control="search"], [data-reader-control="navigation"]'),
    ].find(usable);
    target?.focus();
  }, []);
  const dismissFind = useCallback(() => {
    restoreReaderFocus(findInvoker.current, "find");
    resetFind();
    setFindOpen(false);
  }, [resetFind, restoreReaderFocus]);
  const dismissPanel = useCallback(() => {
    const currentPanel = latest.current.workspace.panel;
    if (currentPanel)
      restoreReaderFocus(
        currentPanel === "search" ? null : panelInvoker.current,
        currentPanel,
      );
    setPanel(null);
  }, [restoreReaderFocus, setPanel]);
  const previousPanel = useRef(panel);
  useEffect(() => {
    const previous = previousPanel.current;
    previousPanel.current = panel;
    if (previous !== "search" || panel === "search" || panel === null) return;
    // Clicking another control keeps its own focus; a keyboard panel switch
    // unmounts the focused search field, so move focus to the panel that
    // opened (its toolbar trigger stands in for the panel content).
    const active = document.activeElement;
    if (active && active !== document.body) return;
    requestAnimationFrame(() => {
      const focused = document.activeElement;
      if (!focused || focused === document.body)
        restoreReaderFocus(null, panel);
    });
  }, [panel, restoreReaderFocus]);
  const dismissReaderChrome = useCallback(() => {
    // Escape inside the article must not pull focus out of that document.
    const target = document.activeElement;
    if (target && findBarRef.current?.contains(target)) dismissFind();
    else {
      resetFind();
      setFindOpen(false);
    }
    if (target && panelRef.current?.contains(target)) dismissPanel();
    else setPanel(null);
  }, [dismissFind, dismissPanel, resetFind, setPanel]);
  const setSearchBooks = (
    value: string[] | ((previous: string[]) => string[]),
  ) => {
    latest.current.onWorkspaceChange((previous) => ({
      ...previous,
      searchScopeInitialized: true,
      searchBooks:
        typeof value === "function" ? value(previous.searchBooks) : value,
    }));
  };
  const [searchBusy, setSearchBusy] = useState(false);
  const [snippetSaving, setSnippetSaving] = useState(false);
  const snippetPreference = useRef(settings.showSearchSnippets);
  const stopSearch = useCallback(() => {
    searchSequence.current++;
    void searchRequest.current?.cancel().catch((value) =>
      latest.current.onError(value, ERROR_SITES["reader.search-cancel"]),
    );
  }, []);
  useEffect(() => {
    const request = createSearchRequest();
    searchRequest.current = request;
    const unsubscribe = subscribeReaderPreferences(() => {
      if (!capturePrivacyContext()) {
        stopSearch();
        setSearchBusy(false);
      }
    });
    return () => {
      unsubscribe();
      if (searchRequest.current === request) searchRequest.current = null;
      void request.dispose().catch((value) =>
        latest.current.onError(value, ERROR_SITES["reader.search-dispose"]),
      );
    };
  }, [stopSearch]);
  useEffect(() => {
    stopSearch();
    setSearchBusy(false);
  }, [props.privacyGeneration, props.suspended, tab.id, stopSearch]);
  useEffect(() => {
    if (snippetPreference.current === settings.showSearchSnippets) return;
    snippetPreference.current = settings.showSearchSnippets;
    stopSearch();
    setSearchBusy(false);
    updateWorkspace({
      searchResults: null,
      searchError: "",
      searchValidation: false,
      searchOffset: 0,
    });
  }, [settings.showSearchSnippets, stopSearch, updateWorkspace]);
  const changeSearchSnippets = async (showSearchSnippets: boolean) => {
    stopSearch();
    setSearchBusy(false);
    updateWorkspace({
      searchResults: null,
      searchError: "",
      searchValidation: false,
      searchOffset: 0,
    });
    setSnippetSaving(true);
    try {
      await api("save_settings", { settings: { showSearchSnippets } });
      latest.current.onRefresh();
    } catch (value) {
      latest.current.onError(value);
    } finally {
      if (alive.current) setSnippetSaving(false);
    }
  };
  const [bookmarkBusy, setBookmarkBusy] = useState(false);
  const [exportBusy, setExportBusy] = useState(false);
  const pendingExport = useRef<{
    requestId: string; session: FrameSession;
    resolve: (snapshot: LiveArticleSnapshot) => void;
    reject: (error: Error) => void;
  } | null>(null);
  useEffect(() => () => {
    pendingExport.current?.reject(new Error(t("The article changed before export completed.")));
    pendingExport.current = null;
  }, [frame?.sessionId, frame?.location.url, props.privacyGeneration, props.suspended]);
  const [external, setExternal] = useState<{
    url: string;
  } | null>(null);
  const externalLinksBlocked = settings.externalLinks === "deny";
  const [linkContext, setLinkContext] = useState<{
    resource: boolean;
    url: string;
    documentUrl: string;
    sessionId: string;
    frameId: string;
    interactionRevision: number;
    path: string | null;
  } | null>(null);
  const [externalBusy, setExternalBusy] = useState(false);
  const [consent, setConsent] = useState<SensorRequest | null>(null);
  const consentRef = useRef<SensorRequest | null>(null);
  consentRef.current = consent;
  const [consentBusy, setConsentBusy] = useState(false);
  const [shortcuts, setShortcuts] = useState(false);
  const [permissionsOpen, setPermissionsOpen] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [notice, setNotice] = useState("");
  const [preview, setPreview] = useState<{
    bookId: string;
    requestedPath: string;
    article: ArticlePreview | null;
  } | null>(null);
  const [spellingBusy, setSpellingBusy] = useState<string | null>(null);
  const [attachment, setAttachment] = useState<ReaderLocation | null>(null);
  const [attachmentBusy, setAttachmentBusy] = useState(false);
  const [attachmentError, setAttachmentError] = useState("");
  const canHideReadingControls = compact && !isDesktop && frameReady && !loading &&
    !props.suspended && props.interactive && !props.mobileTabs?.expanded &&
    !panel && !findOpen && !controlsOpen && !consent && !external && !attachment &&
    !linkContext && !preview && !shortcuts && !permissionsOpen && !selection;
  panCanHide.current = canHideReadingControls;
  const readingControlsHidden = panHidden && canHideReadingControls;
  useLayoutEffect(() => {
    // Only a new document resets the compact reader's pan state. Do not reset
    // it for lifecycle, focus or unrelated interaction updates: those arrive
    // after scrolling settles and must not pull the controls back into view.
    setPanHidden(false);
  }, [compact, tab.id, tab.bookId, tab.path, frame?.sessionId, frame?.location.url]);
  useLayoutEffect(() => {
    latest.current.onReaderChromeHiddenChange?.(readingControlsHidden);
  }, [readingControlsHidden]);
  useLayoutEffect(() => {
    const session = activeFrame.current;
    const node = frameRef.current;
    if (!session || !node || !frameReady || props.suspended) {
      readingViewport.current = null;
      return;
    }
    const previous = readingViewport.current;
    const top = node.getBoundingClientRect().top;
    readingViewport.current = { sessionId: session.sessionId, url: session.location.url,
      top, hidden: readingControlsHidden };
    if (previous?.sessionId === session.sessionId && previous.url === session.location.url &&
        previous.hidden !== readingControlsHidden && top !== previous.top)
      send({ command: "reading-viewport", deltaTop: top - previous.top, url: session.location.url });
  }, [readingControlsHidden, frameReady, frame?.sessionId, frame?.location.url, props.suspended]);
  useLayoutEffect(() => () => {
    latest.current.onReaderChromeHiddenChange?.(false);
  }, []);
  useEffect(() => {
    if (!compact || isDesktop) return;
    document.addEventListener("keydown", revealReadingControls, true);
    return () => {
      document.removeEventListener("keydown", revealReadingControls, true);
    };
  }, [compact, revealReadingControls]);
  useEffect(() => {
    setLinkContext(null);
  }, [
    tab.id,
    props.suspended,
    props.interactive,
    props.interactionRevision,
    props.privacyGeneration,
    props.privacyPolicyGeneration,
    consent,
    external,
    attachment,
    controlsOpen,
    shortcuts,
    permissionsOpen,
  ]);
  const currentLinkContext = () => {
    const session = activeFrame.current;
    if (
      !linkContext ||
      !session ||
      session.sessionId !== linkContext.sessionId ||
      session.frameId !== linkContext.frameId ||
      session.location.url !== linkContext.documentUrl ||
      session.privacyGeneration !== latest.current.privacyGeneration ||
      latest.current.interactionRevision !== linkContext.interactionRevision ||
      latest.current.suspended ||
      !latest.current.interactive ||
      !canNavigateReaderStorage() ||
      !workspaceRef.current?.getClientRects().length
    )
      return null;
    const path = articlePath(linkContext.url, session.location);
    if (path !== linkContext.path || (path === null && !validExternal(linkContext.url)))
      return null;
    return { session, path, url: linkContext.url };
  };
  const preferences = useSyncExternalStore(
    subscribeReaderPreferences,
    getReaderPreferences,
  );
  const sharedAppearance =
    preferences.privateMode === settings.privateMode &&
    preferences.readOnly === settings.readOnly
      ? { ...preferences.confirmed, ...preferences.pending }
      : {};
  const readerSettings = {
    ...settings,
    ...(settings.readOnly ? workspace.appearancePatch : sharedAppearance),
  };
  const preferencesMatch =
    preferences.privateMode === settings.privateMode &&
    preferences.readOnly === settings.readOnly;
  const overrides = preferencesMatch ? preferences.bookZoom : settings.bookZoom;
  const pendingZoom = preferencesMatch ? preferences.pendingBookZoom : {};
  const bookId = tab.bookId;
  const recentSearches = useMemo(() => {
    if (!bookId) {
      return props.recentSearches.filter((entry) => !entry.includes("\u001f"));
    }
    const prefix = `${bookId}\u001f`;
    return props.recentSearches
      .filter((entry) => entry.startsWith(prefix))
      .map((entry) => entry.slice(prefix.length));
  }, [props.recentSearches, bookId]);
  const zoomOverride = bookId
    ? settings.readOnly && Object.hasOwn(workspace.bookZoom, bookId)
      ? workspace.bookZoom[bookId]
      : Object.hasOwn(pendingZoom, bookId)
        ? pendingZoom[bookId]
        : overrides[bookId]
    : null;
  readerSettings.zoom = Math.max(
    0.25,
    Math.min(5, zoomOverride ?? readerSettings.zoom),
  );
  const settingsRef = useRef(readerSettings);
  settingsRef.current = readerSettings;
  const book = useMemo(
    () => books.find((item) => item.id === tab.bookId),
    [books, tab.bookId],
  );
  const archiveLanguages = archiveSpeechLanguages(book?.language ?? "");
  const speechLanguage = workspace.speechLanguages[bookId ?? ""] ?? archiveLanguages[0] ?? getLocale();
  const speech = speechPreference(readerSettings.speechPreferences, speechLanguage);
  const matchingVoices = useMemo(
    () => matchingSpeechVoices(voices, speechLanguage),
    [voices, speechLanguage],
  );
  const matchingVoiceUris = useMemo(
    () => new Set(matchingVoices.map((voice) => voice.voiceURI)),
    [matchingVoices],
  );
  const otherVoices = useMemo(
    () => voices.filter((voice) => !matchingVoiceUris.has(voice.voiceURI) &&
      !(voice.voiceURI.startsWith("kokoro:") && !speechLanguage.toLowerCase().startsWith("en"))),
    [voices, matchingVoiceUris, speechLanguage],
  );
  const effectiveVoice = useMemo(
    () => selectedSpeechVoice(voices, speechLanguage, speech.voice),
    [voices, speech.voice, speechLanguage],
  );
  const speechContext = useRef({ language: speechLanguage, voices });
  speechContext.current = { language: speechLanguage, voices };
  // The key canonicalizes the archive path, so recompute it only when the
  // address changes, never per bookmark or per scroll-driven render.
  const draftKey = useMemo(
    () => bookmarkKey(tab),
    [tab.bookId, tab.path],
  );
  const savedBookmark = useMemo(
    () => bookmarks.find((item) => bookmarkKey(item) === draftKey),
    [bookmarks, draftKey],
  );
  const passages = useMemo(
    () => (passageDocument ? applicablePassages(bookmarks, passageDocument) : []),
    [bookmarks, passageDocument],
  );
  const bookmarkDraft = workspace.bookmarkDrafts[draftKey];
  const bookmarkTitle =
    bookmarkDraft?.title ?? savedBookmark?.title ?? tab.title;
  const bookmarkNote = bookmarkDraft?.note ?? savedBookmark?.note ?? "";
  const bookmarkTags =
    bookmarkDraft?.tags ?? savedBookmark?.tags.join(", ") ?? "";
  const editBookmark = (patch: Partial<BookmarkDraft>) => {
    latest.current.onWorkspaceChange((previous) => ({
      ...previous,
      bookmarkDrafts: {
        ...previous.bookmarkDrafts,
        [draftKey]: {
          title: bookmarkTitle,
          note: bookmarkNote,
          tags: bookmarkTags,
          ...previous.bookmarkDrafts[draftKey],
          ...patch,
        },
      },
    }));
  };
  const clearBookmarkDraft = (
    key: string,
    expected: BookmarkDraft | undefined,
  ) => {
    latest.current.onWorkspaceChange((previous) => {
      if (previous.bookmarkDrafts[key] !== expected) return previous;
      const bookmarkDrafts = { ...previous.bookmarkDrafts };
      delete bookmarkDrafts[key];
      return { ...previous, bookmarkDrafts };
    });
  };
  const availableBooks = useMemo(
    () => books.filter((item) => item.available),
    [books],
  );
  const availableBookIds = useMemo(
    () => new Set(availableBooks.map((item) => item.id)),
    [availableBooks],
  );
  // One pass over the reading list; the empty page reuses this for both its
  // visibility check and its first six cards.
  const availableBookmarks = useMemo(
    () => bookmarks.filter((item) => availableBookIds.has(item.bookId)),
    [bookmarks, availableBookIds],
  );

  // A mounted document keeps the generation it was bootstrapped with. When the
  // window's authority has moved on, ask the root for the live snapshot once:
  // the new generation re-bootstraps this reader with the same tab, article and
  // scroll. Never navigate or write on behalf of the retired document.
  const reestablishAuthority = useCallback(() => {
    const state = authorityRefresh.current;
    if (state.pending || state.generation === latest.current.privacyGeneration)
      return;
    authorityRefresh.current = {
      generation: latest.current.privacyGeneration,
      pending: true,
    };
    if (alive.current)
      setNotice(t("Restoring archive access after a privacy change…"));
    void Promise.resolve()
      .then(() => latest.current.onRefresh())
      .then((result) => {
        // The root reports a failed snapshot as `false` instead of throwing.
        // Forget the attempted generation so the next stale-frame interaction
        // asks again rather than being latched out until another refresh.
        if (result === false)
          authorityRefresh.current = { ...authorityRefresh.current, generation: -1 };
      })
      .catch((value) => {
        if (!alive.current) return;
        authorityRefresh.current = { ...authorityRefresh.current, generation: -1 };
        // The API boundary retries a suspended refresh on resume; it is not a
        // user-visible failure even if the retry is refused again.
        if (!isSuspendedError(value)) setError(message(value));
        latest.current.onError(value, undefined, false);
      })
      .finally(() => {
        authorityRefresh.current = {
          ...authorityRefresh.current,
          pending: false,
        };
        if (alive.current) setNotice("");
      });
  }, []);
  const report = useCallback((value: unknown, severity: ErrorSeverity = "blocking") => {
    if (!alive.current) return;
    const book = latest.current.books.find((item) => item.id === tabRef.current.bookId);
    if (isProviderUnavailableFor(value, book)) {
      // The root suppresses only this exact source-scoped transient: the
      // reader owns its visible reconnect/reload notice until genuine ready.
      setProviderUnavailable(value);
      latest.current.onError(value, severity);
      return;
    }
    // The root records the failure; the active reader owns its visible notice.
    latest.current.onError(value, severity, false);
    if (shouldRenderError(severity, latest.current.settings.showTechnicalErrors))
      setError(message(value));
    // A refused generation is not a user error and not a dead end: the reader
    // re-establishes authority so the article reloads without a manual Reload.
    if (isPrivacyTransitionError(value)) reestablishAuthority();
  }, [reestablishAuthority]);
  // The bridge reports a scroll position every animation frame. Keep the live
  // value in the tab ref so history and visit logic stay exact, but publish to
  // the root at most every 250 ms; the trailing timer always publishes the
  // final position.
  const scrollPublish = useRef<{
    timer?: ReturnType<typeof setTimeout>;
    pending: number | null;
  }>({ pending: null });
  const clearScrollPublish = useCallback(() => {
    const state = scrollPublish.current;
    clearTimeout(state.timer);
    state.timer = undefined;
    state.pending = null;
  }, []);
  const flushScrollPublish = useCallback(() => {
    const state = scrollPublish.current;
    clearTimeout(state.timer);
    state.timer = undefined;
    const scroll = state.pending;
    state.pending = null;
    if (scroll === null) return;
    // Also reached from unmount cleanup, after `alive` is false: the root
    // revalidates the tab before it accepts the update.
    const next = { ...tabRef.current, scroll };
    tabRef.current = next;
    latest.current.onTabChange(next);
  }, []);
  const publishTab = useCallback((next: Tab) => {
    if (!alive.current) return;
    // An authoritative tab publish supersedes a queued scroll-only one, but a
    // reload/restricted publish for the same article must not discard the last
    // live position. Carry it into this publish instead.
    const state = scrollPublish.current;
    if (
      state.pending !== null &&
      next.scroll === tabRef.current.scroll &&
      next.id === tabRef.current.id &&
      next.bookId === tabRef.current.bookId &&
      next.path === tabRef.current.path
    ) {
      next = { ...next, scroll: state.pending };
    }
    clearScrollPublish();
    tabRef.current = next;
    latest.current.onTabChange(next);
  }, [clearScrollPublish]);
  const publishScroll = useCallback((scroll: number) => {
    scrollPosition.current = scroll;
    tabRef.current = { ...tabRef.current, scroll };
    const state = scrollPublish.current;
    state.pending = scroll;
    if (state.timer !== undefined) return;
    state.timer = setTimeout(() => {
      state.timer = undefined;
      const value = state.pending;
      state.pending = null;
      if (value === null || !alive.current) return;
      publishTab({ ...tabRef.current, scroll: value });
    }, 250);
  }, [publishTab]);
  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") flushScrollPublish();
    };
    document.addEventListener("visibilitychange", hidden);
    window.addEventListener("pagehide", flushScrollPublish);
    latest.current.onScrollFlushHandler?.(flushScrollPublish);
    return () => {
      document.removeEventListener("visibilitychange", hidden);
      window.removeEventListener("pagehide", flushScrollPublish);
      latest.current.onScrollFlushHandler?.(null);
      // Unmount: never drop the last observed position.
      flushScrollPublish();
    };
  }, [flushScrollPublish]);
  const send = useCallback((command: ReaderCommand) => {
    if (command.command === "stop-speech") {
      // Mobile reads only natively; desktop may be reading with either engine,
      // so stop the native one and still tell the frame.
      void nativeSpeechStop.current?.();
      if (isMobile) return;
    }
    const session = activeFrame.current;
    if (!session) return;
    frameRef.current?.contentWindow?.postMessage(
      {
        channel: "kiwix-reader",
        version: 1,
        type: "reader-command",
        sessionId: session.sessionId,
        ...command,
      },
      session.location.origin,
    );
  }, []);
  useEffect(() => {
    if (!frameReady || readingControlsHidden) return;
    const bar = workspaceRef.current?.querySelector<HTMLElement>(".rw-mobile-bar");
    const update = () => send({ command: "reading-controls", enabled: canHideReadingControls,
      minimumScroll: (bar?.getBoundingClientRect().height || 52) + 2 });
    update();
    if (!bar) return;
    const observer = new ResizeObserver(update);
    observer.observe(bar);
    return () => observer.disconnect();
  }, [frameReady, canHideReadingControls, readingControlsHidden, send]);
  // Mobile suspension retires storage shortly after this event, but audio may
  // legitimately continue while the process is alive. Pause only video that is
  // not the active PiP element; teardown and privacy retirement pause all media.
  // A transient suspension flag (a gated native call while the user taps play)
  // must not stop a video that is just starting, so the pause waits until the
  // suspension has actually lasted.
  useEffect(() => {
    if (!props.suspended) return;
    const timer = setTimeout(() => send({ command: "pause-video" }), 1500);
    return () => clearTimeout(timer);
  }, [props.suspended, send]);
  // A native audio interruption or output-route loss is a system stop: WebKit
  // already pauses the element itself. The shell only clears its own media
  // report here. It must NOT send `pause-media`: doing so raced the user's next
  // play and, on this device, an interruption loop paused the video a fraction
  // of a second after every tap.
  const handledInterruption = useRef(props.audioInterruption ?? 0);
  useEffect(() => {
    const revision = props.audioInterruption ?? 0;
    if (!revision || handledInterruption.current === revision) return;
    handledInterruption.current = revision;
    latest.current.onMediaState?.({ audible: false, pictureInPicture: false });
  }, [props.audioInterruption]);
  const speechIdentity = useCallback(() => {
    const session = activeFrame.current;
    if (!session || session.privacyGeneration !== props.privacyGeneration ||
        !frameReady || !canNavigateReaderStorage()) return null;
    return {
      sessionId: session.sessionId, archiveId: session.location.bookId,
      origin: session.location.origin, url: session.location.url,
      privacyGeneration: session.privacyGeneration,
    };
  }, [frameReady, props.privacyGeneration]);
  const nativeSpeech = useNativeSpeech({
    context: () => (props.suspended || !props.interactive ? null : speechIdentity()),
    identity: speechIdentity,
    contextKey: `${frame?.sessionId ?? ""}:${frame?.location.url ?? ""}:${props.privacyGeneration}`,
    privacyGeneration: props.privacyGeneration,
    interactive: props.interactive,
    sourceReady: frameReady && !props.suspended && frame?.privacyGeneration === props.privacyGeneration,
    interactionRevision: props.interactionRevision,
    extract: (requestId) => send({ command: "speech-text", requestId }),
    onVoices: (installed, available, failure) => {
      if (!isMobile) {
        // Desktop lists only neural voices here; the frame owns availability
        // and system voices, so a failed neural listing just hides Kokoro.
        setNeuralVoices(failure ? [] : filterSpeechVoices(installed));
        return;
      }
      setVoices(filterSpeechVoices(installed));
      setSpeechAvailable(available);
      setSpeechAvailabilityError(failure ? message(failure) : "");
      // Capability discovery owns the speech panel, never article-load failure.
      if (failure) latest.current.onError(failure, "recoverable");
    },
    onSpeaking: isMobile ? setSpeaking : setNativeSpeaking,
    onProgress: (offset) => {
      if (offset === null) send({ command: "speech-highlight", clear: true });
      else send({ command: "speech-highlight", offset });
    },
    onError: report,
  });
  nativeSpeechStop.current = nativeSpeech.stop;
  // The native path reads on mobile always, and on desktop for Kokoro voices.
  const nativeReading = isMobile
    ? speaking || nativeSpeech.pending || nativeSpeech.paused
    : nativeSpeaking || nativeSpeech.pending || nativeSpeech.paused;
  const readingSpeech = nativeReading || (!isMobile && speaking);
  const neuralSelected = !isMobile && (effectiveVoice?.voiceURI.startsWith("kokoro:") ?? false) &&
    matchingVoiceUris.has(effectiveVoice?.voiceURI ?? "");
  const nativeReadingRef = useRef(false);
  nativeReadingRef.current = nativeReading;
  const toggleReading = useCallback((event: React.MouseEvent) => {
    const spokenSelection = passageSelection?.exact ?? selection;
    if (readingSpeech && !nativeReading) { send({ command: "stop-speech" }); return; }
    if (isMobile || nativeReading || neuralSelected) {
      if (!event.isTrusted) return;
      if (readingSpeech) {
        if (nativeSpeech.pending) void nativeSpeech.stop();
        else if (nativeSpeech.paused) void nativeSpeech.resume();
        else void nativeSpeech.pause();
        return;
      }
      if ((isMobile && !speechAvailable) || !frameReady) return;
      if (spokenSelection)
        send({
          command: "speech-highlight-selection",
          text: spokenSelection,
          ...(passageSelection
            ? { start: passageSelection.start, end: passageSelection.end }
            : {}),
        });
      void nativeSpeech.begin({
        rate: speech.rate, lang: speechLanguage,
        title: tab.title || book?.title || "",
        voice: matchingVoiceUris.has(effectiveVoice?.voiceURI ?? "")
          ? effectiveVoice?.voiceURI ?? "" : "",
      }, spokenSelection || null);
    } else {
      if (!speechAvailable || !frameReady) return;
      send({
        command: "speak", ...(spokenSelection ? { text: spokenSelection } : {}),
        ...(passageSelection
          ? { start: passageSelection.start, end: passageSelection.end }
          : {}),
        rate: speech.rate, voice: effectiveVoice?.voiceURI ?? "", lang: speechLanguage,
      });
    }
  }, [book?.title, effectiveVoice, frameReady, isMobile, matchingVoiceUris, nativeReading, nativeSpeech,
      neuralSelected, passageSelection, readingSpeech, selection, send, speech.rate, speechAvailable,
      speechLanguage, tab.title]);
  const stopReading = useCallback(() => {
    if (isMobile) void nativeSpeech.stop();
    // Desktop: stop-speech reaches both engines (see send).
    else send({ command: "stop-speech" });
  }, [isMobile, nativeSpeech, send]);
  // The root decides whether a suspension may retain this document. Native
  // read-aloud is an audible surface even though no media element exists.
  const publishMedia = useCallback(() => {
    latest.current.onMediaState?.({
      // Rust retains an exact started capability. Android has no archive-media
      // exemption, so only an admitted speech start keeps its reader; iOS also
      // retains pending speech and archive media through its native exemption.
      audible: speaking || nativeSpeech.paused ||
        (isAndroid && nativeSpeech.startAdmitted) ||
        (isIOS && (frameMedia.current.audible || nativeSpeech.pending || nativeSpeech.startAdmitted)),
      pictureInPicture: isIOS && frameMedia.current.pictureInPicture,
    });
  }, [speaking, nativeSpeech.pending, nativeSpeech.paused, nativeSpeech.startAdmitted]);
  useEffect(() => { publishMedia(); }, [publishMedia]);
  // Retire the shell's audible exemption when the reader leaves the screen.
  // WebKit owns archive audio activation and release; this is bookkeeping,
  // not an instruction to activate or deactivate its native audio session.
  useEffect(
    () => () => {
      latest.current.onMediaState?.({
        audible: false,
        pictureInPicture: false,
        release: true,
      });
    },
    [],
  );
  const sendHighlights = useCallback(
    (document: PassageDocument) => {
      const session = activeFrame.current;
      if (
        !session ||
        session.sessionId !== document.sessionId ||
        session.privacyGeneration !== latest.current.privacyGeneration ||
        session.location.bookId !== document.bookId ||
        session.location.url !== document.url
      )
        return;
      const wire = {
        toWire: new Map<string, string>(),
        fromWire: new Map<string, string>(),
        permitted: new Set<string>(),
        url: document.url,
        sessionId: document.sessionId,
      };
      const highlights = applicablePassages(latest.current.bookmarks, document)
        .slice(0, MAX_DOCUMENT_HIGHLIGHTS)
        .map(({ highlight, bookmark }) => {
          let token = wire.toWire.get(bookmark.id);
          if (!token) {
            token = crypto.randomUUID();
            wire.toWire.set(bookmark.id, token);
            wire.fromWire.set(token, bookmark.id);
          }
          wire.permitted.add(`${bookmark.id}:${highlight.id}`);
          // Archive scripts may inspect messages; personal comments stay in the shell.
          return {
            id: highlight.id,
            bookmarkId: token,
            documentPath: highlight.documentPath,
            exact: highlight.exact,
            prefix: highlight.prefix,
            suffix: highlight.suffix,
            start: highlight.start,
            end: highlight.end,
          };
        });
      passageWire.current = wire;
      setPassageStatuses({});
      send({
        command: "highlights-begin",
        url: document.url,
        documentPath: document.documentPath,
      });
      // Each authenticated command stays bounded to one bookmark's maximum. Large
      // imports spanning several bookmarks do not silently lose later passages.
      for (let offset = 0; offset < highlights.length; offset += 200)
        send({
          command: "highlights",
          url: document.url,
          highlights: highlights.slice(offset, offset + 200),
        });
      send({ command: "highlights-end", url: document.url });
    },
    [send],
  );
  // Derive a content-addressed key for only the highlights belonging to the
  // active passage document so unrelated snapshot refreshes never tear down
  // and re-anchor highlights inside the article iframe.
  const passageHighlightsKey = useMemo(() => {
    if (!passageDocument) return "";
    return JSON.stringify(
      applicablePassages(bookmarks, passageDocument)
        .slice(0, MAX_DOCUMENT_HIGHLIGHTS)
        .map(({ bookmark, highlight }) => [
          bookmark.id,
          highlight.id,
          highlight.documentPath,
          highlight.exact,
          highlight.prefix,
          highlight.suffix,
          highlight.start,
          highlight.end,
        ]),
    );
  }, [bookmarks, passageDocument]);
  useEffect(() => {
    if (frameReady && passageDocument) sendHighlights(passageDocument);
  }, [frameReady, passageDocument, passageHighlightsKey, sendHighlights]);
  useEffect(() => {
    setCommentDrafts({});
    setPassageSelection(null);
    setPassageSelectionUnavailable(false);
    setPassageStatuses({});
    passageDocumentRef.current = null;
    setPassageDocument(null);
  }, [props.privacyPolicyGeneration]);
  const replySensor = useCallback(
    (request: SensorRequest, data: Record<string, unknown>) => {
      const session = activeFrame.current;
      if (!session || session.sessionId !== request.sessionId) return;
      frameRef.current?.contentWindow?.postMessage(
        {
          channel: "kiwix-reader",
          version: 1,
          type: "sensor-response",
          sessionId: session.sessionId,
          requestId: request.requestId,
          sensorStatus: session.status,
          ...data,
        },
        session.location.origin,
      );
    },
    [],
  );
  const rememberVisit = useCallback((location: TabLocation, frameGeneration: number) => {
    const privacy = capturePrivacyContext();
    lastVisit.current = privacy && privacy.generation === frameGeneration &&
      !privacy.privateMode && !privacy.readOnly && latest.current.settings.historyEnabled
      ? { location, privacy } : null;
  }, []);
  const persistVisit = useCallback((create = false) => {
    const visit = lastVisit.current;
    if (
      !visit ||
      !isPrivacyContextCurrent(visit.privacy) ||
      latest.current.suspended ||
      !latest.current.interactive ||
      !canNavigateReaderStorage() ||
      !latest.current.settings.historyEnabled ||
      latest.current.settings.privateMode ||
      latest.current.settings.readOnly
    )
      return;
    void api("record_visit", { ...visit.location, create, privacyGeneration: visit.privacy.generation }).catch((value) => {
      if (
        !isPrivacyContextCurrent(visit.privacy) ||
        latest.current.suspended ||
        !latest.current.interactive ||
        !canNavigateReaderStorage()
      )
        return;
      if (alive.current)
        report(value, ERROR_SITES["reader.record-visit"]);
      else
        latest.current.onError(value, ERROR_SITES["reader.record-visit"]);
    });
  }, [report]);
  const cancelSensors = useCallback((sessionId: string) => {
    const session = activeFrame.current;
    if (session?.sessionId === sessionId) {
      for (const sensor of ["camera", "location"]) {
        frameRef.current?.contentWindow?.postMessage(
          {
            channel: "kiwix-reader",
            version: 1,
            type: "sensor-revoke",
            sessionId,
            sensor,
          },
          session.location.origin,
        );
      }
    }
    for (const request of pendingSensors.current.values()) {
      if (request.sessionId !== sessionId) continue;
      request.cancelled = true;
      void api("sensor_cancel", {
        sessionId,
        requestId: request.nativeId,
      }).catch((value) =>
        latest.current.onError(value, ERROR_SITES["reader.sensor-deactivate"]),
      );
      pendingSensors.current.delete(sensorKey(request));
    }
    consentRef.current = null;
    if (alive.current) {
      setConsent(null);
      setConsentBusy(false);
    }
  }, []);
  const clearDocumentProbe = useCallback(() => {
    const probe = documentProbe.current;
    if (probe) clearTimeout(probe.timer);
    documentProbe.current = null;
  }, []);
  const deactivateFrame = useCallback((): Promise<void> => {
    clearHistoryTraversal();
    clearDocumentProbe();
    navigationSequence.current++;
    openSequence.current++;
    const session = activeFrame.current;
    if (alive.current) {
      setLinkContext(null);
      setHeldPaint("");
    }
    resetFind();
    if (!session) return Promise.resolve();
    send({ command: "stop-speech" });
    send({ command: "pause-media" });
    latest.current.onMediaState?.({ audible: false, pictureInPicture: false });
    cancelSensors(session.sessionId);
    activeFrame.current = null;
    if (alive.current) setFrameReady(false);
    return lifecycle(() =>
      api("sensor_deactivate", { sessionId: session.sessionId }),
    ).then(() => undefined);
  }, [cancelSensors, clearDocumentProbe, clearHistoryTraversal, resetFind, send]);
  // The one place a session and its photo secret are handed to the frame. The
  // bridge keeps both from archive scripts for as long as its listener exists;
  // callers therefore reach this only through admitDocument.
  const initializeDocument = useCallback((session: FrameSession) => {
    bridgeVouched.current = false;
    pendingInit.current = "";
    frameRef.current?.contentWindow?.postMessage(
      {
        channel: "kiwix-reader",
        version: 1,
        type: "reader-init",
        sessionId: session.sessionId,
        sensorStatus: session.status,
        linkPreviews: latest.current.settings.linkPreviews,
        photoChooser: isWindows && session.location.restricted !== true,
        photoSecret: photoSecretFor(session.sessionId),
        mediaNotice: {
          text: t(
            "This video uses a format this device cannot play smoothly. It may stutter. The same video plays smoothly in the desktop app.",
          ),
          dismiss: t("Dismiss"),
        },
      },
      session.location.origin,
    );
  }, [photoSecretFor]);
  // A document that no bridge has vouched for is never initialised: a page
  // without a bridge, or one whose bridge listener is gone, would read the
  // session id and the photo secret directly. It stays unadmitted and hidden
  // until a bridge vouches, and otherwise ends in the connection watchdog.
  const admitDocument = useCallback((session: FrameSession) => {
    if (bridgeVouched.current) initializeDocument(session);
    else pendingInit.current = session.sessionId;
  }, [initializeDocument]);
  const settlePendingInit = useCallback(() => {
    const active = activeFrame.current;
    const node = frameRef.current;
    if (
      !alive.current ||
      !bridgeVouched.current ||
      !active ||
      !node ||
      pendingInit.current !== active.sessionId ||
      displayedFrame.current?.frameId !== active.frameId ||
      !canNavigateReaderStorage() ||
      active.privacyGeneration !== latest.current.privacyGeneration ||
      frameNavigation.current?.node !== node ||
      frameNavigation.current.frameId !== active.frameId
    )
      return;
    initializeDocument(active);
  }, [initializeDocument]);
  // Another document's bridge announced itself after the one this session is
  // bound to (or the binding is contested). Archive code can forge that, which
  // only costs it a re-session, but cannot learn the bound token.
  const documentReplaced = useCallback((session: FrameSession) => {
    const authority = panAuthority.current;
    return (
      documentAnnounced.current !== null &&
      authority?.sessionId === session.sessionId &&
      authority.token !== documentAnnounced.current
    );
  }, []);
  // The session's document was replaced, or nothing proved otherwise: retire
  // that session and prepare a new one for whichever document vouches.
  const resessionFrame = useCallback(
    (node: HTMLIFrameElement, active: FrameSession, painted: boolean) => {
      setLinkContext(null);
      latest.current.clearMediaCheckpoint();
      retainedActivation.current = null;
      cancelSensors(active.sessionId);
      const sequence = ++openSequence.current;
      activeFrame.current = null;
      // The outgoing page was on screen and usable until now. When a bridge in
      // this frame announced the replacement, keep it painted through the
      // handshake, for a bounded time; anything else hides it again.
      setHeldPaint(painted ? active.frameId : "");
      const releasePaint = () => {
        if (alive.current)
          setHeldPaint((held) => (held === active.frameId ? "" : held));
      };
      clearTimeout(heldPaintTimer.current);
      heldPaintTimer.current = painted
        ? window.setTimeout(releasePaint, HELD_PAINT_LIMIT)
        : undefined;
      setFrameReady(false);
      setLoading(true);
      void lifecycle(async () => {
        await api("sensor_deactivate", {
          sessionId: active.sessionId,
        });
        if (
          !alive.current ||
          !canNavigateReaderStorage() ||
          openSequence.current !== sequence ||
          frameRef.current !== node ||
          node.dataset.frameId !== active.frameId ||
          latest.current.privacyGeneration !==
            active.privacyGeneration
        ) {
          releasePaint();
          return;
        }
        const sessionId = crypto.randomUUID();
        const status = await api<SensorStatus>("sensor_activate", {
          sessionId,
          archiveId: active.location.bookId,
          origin: active.location.origin,
        }).catch((value) => {
          if (
            !alive.current ||
            openSequence.current !== sequence ||
            frameRef.current !== node ||
            node.dataset.frameId !== active.frameId
          )
            throw value;
          report(value, ERROR_SITES["reader.sensor-activate"]);
          return {
            camera: "notAsked",
            location: "notAsked",
            nativeLocation: false,
          } satisfies SensorStatus;
        });
        if (
          !alive.current ||
          !canNavigateReaderStorage() ||
          latest.current.privacyGeneration !==
            active.privacyGeneration ||
          openSequence.current !== sequence ||
          frameRef.current !== node ||
          node.dataset.frameId !== active.frameId
        ) {
          releasePaint();
          await api("sensor_deactivate", { sessionId });
          return;
        }
        const updated = { ...active, sessionId, status };
        activeFrame.current = updated;
        setFrame(updated);
        admitDocument(updated);
      }).catch((value) => {
        releasePaint();
        if (alive.current && openSequence.current === sequence) {
          setLoading(false);
          report(value);
        }
      });
    },
    [admitDocument, cancelSensors, report],
  );
  // The session a probe was opened for loses admission, provided the frame is
  // still the one that was probed.
  const resessionUnproven = useCallback(
    (probe: NonNullable<typeof documentProbe.current>, painted: boolean) => {
      const active = activeFrame.current;
      if (
        !alive.current ||
        !active ||
        active.sessionId !== probe.sessionId ||
        active.frameId !== probe.frameId ||
        displayedFrame.current?.frameId !== active.frameId ||
        !canNavigateReaderStorage() ||
        active.privacyGeneration !== latest.current.privacyGeneration ||
        frameRef.current !== probe.node ||
        frameNavigation.current?.node !== probe.node ||
        frameNavigation.current.frameId !== active.frameId
      )
        return;
      resessionFrame(probe.node, active, painted);
    },
    [resessionFrame],
  );
  // Ask the bridge bound to `sessionId` to prove it is still listening. Only a
  // session that already has a bound token is ever named to the frame.
  const postDocumentProbe = useCallback(
    (probe: NonNullable<typeof documentProbe.current>, origin: string) => {
      const nonce = crypto.randomUUID();
      probe.nonce = nonce;
      if (probe.kept)
        probe.timer = window.setTimeout(() => {
          const current = documentProbe.current;
          if (current !== probe || current.nonce !== nonce) return;
          // Unproven: the session loses admission, and the page is hidden
          // because nothing shows it is still the document that was admitted.
          // A late answer no longer keeps the session; it can only vouch that
          // the bridge is alive, so the replacement session reaches it.
          probe.kept = false;
          probe.timer = undefined;
          resessionUnproven(probe, false);
        }, DOCUMENT_PROOF_TIMEOUT);
      probe.node.contentWindow?.postMessage(
        {
          channel: "kiwix-reader",
          version: 1,
          type: "document-probe",
          sessionId: probe.sessionId,
          nonce,
        },
        origin,
      );
    },
    [resessionUnproven],
  );
  const probeDocument = useCallback(
    (node: HTMLIFrameElement, active: FrameSession, painted: boolean) => {
      clearDocumentProbe();
      const probe: NonNullable<typeof documentProbe.current> = {
        nonce: null,
        sessionId: active.sessionId,
        frameId: active.frameId,
        node,
        painted,
        kept: true,
      };
      if (panAuthority.current?.sessionId === active.sessionId) {
        documentProbe.current = probe;
        postDocumentProbe(probe, active.location.origin);
        return;
      }
      // No bridge has bound a token to this session yet, so there is nothing
      // to prove it with, and its id is not disclosed to whichever document
      // reads the probe. A document that announced itself since reader-init
      // went out gets a new session; otherwise wait for the binding.
      if (bridgeVouched.current) resessionFrame(node, active, painted);
      else documentProbe.current = probe;
    },
    [clearDocumentProbe, postDocumentProbe, resessionFrame],
  );
  // The first load of a frame the shell navigated. Initialisation waits for a
  // bridge to vouch for the document. A same-document reopen makes no new
  // announcement, so the bridge initialised earlier in this frame is asked to
  // prove that it is still the one listening.
  const admitLoadedDocument = useCallback(
    (node: HTMLIFrameElement, active: FrameSession) => {
      clearDocumentProbe();
      admitDocument(active);
      const authority = panAuthority.current;
      if (
        pendingInit.current !== active.sessionId ||
        !authority ||
        !authority.token ||
        authority.sessionId === active.sessionId ||
        authority.contextId !== active.contextId ||
        authority.origin !== active.location.origin
      )
        return;
      const probe: NonNullable<typeof documentProbe.current> = {
        nonce: null,
        sessionId: authority.sessionId,
        frameId: active.frameId,
        node,
        painted: false,
        kept: false,
      };
      documentProbe.current = probe;
      postDocumentProbe(probe, active.location.origin);
    },
    [admitDocument, clearDocumentProbe, postDocumentProbe],
  );
  const restoreFrame = useCallback(
    (url: string) => {
      const previous = displayedFrame.current;
      if (latest.current.suspended) return;
      if (!alive.current || !previous) return;
      const path = articlePath(url, previous.location);
      if (path === null)
        throw new Error(
          t("The archive server returned an unsafe reader address."),
        );
      if (path !== tabRef.current.path) publishTab({ ...tabRef.current, path });
      // Reopen through activation in the existing context: an unfinished handoff
      // can still contain the archive and must navigate, not lose pagehide on removal.
      resolvedKey.current = "";
      setLoading(true);
      setReload((value) => value + 1);
    },
    [publishTab],
  );

  const preferenceHandlers = useRef({
    onSaved: () => {
      latest.current.onRefresh();
    },
    onError: (value: unknown) => {
      if (alive.current) report(value);
      else latest.current.onError(value);
    },
  });
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      openSequence.current++;
      clearHistoryTraversal();
      stopSearch();
      navigationSequence.current++;
      previewSequence.current++;
      clearTimeout(previewGate.current.timer);
      previewGate.current.timer = undefined;
      previewQueued.current = null;
      clearTimeout(scrollTimer.current);
      persistVisit();
      const session = activeFrame.current;
      if (session) {
        for (const command of ["stop-speech", "pause-media"] as const)
          frameRef.current?.contentWindow?.postMessage(
            {
              channel: "kiwix-reader",
              version: 1,
              type: "reader-command",
              sessionId: session.sessionId,
              command,
            },
            session.location.origin,
          );
        cancelSensors(session.sessionId);
        void lifecycle(() =>
          api("sensor_deactivate", { sessionId: session.sessionId }),
        ).catch((value) =>
          latest.current.onError(value, ERROR_SITES["reader.sensor-deactivate"]),
        );
      }
      latest.current.onMediaState?.({ audible: false, pictureInPicture: false });
      activeFrame.current = null;
      clearDocumentProbe();
    };
  }, [cancelSensors, clearDocumentProbe, clearHistoryTraversal, persistVisit, stopSearch]);

  useLayoutEffect(() => {
    const session = displayedFrame.current;
    const node = frameRef.current;
    if (!session || !node?.contentWindow ||
        frameNavigation.current?.frameId === session.frameId) return;
    const admission = new AbortController();
    void waitForReaderStorageNavigation(admission.signal).then(() => {
      if (admission.signal.aborted || !alive.current || !node.isConnected ||
          frameRef.current !== node || displayedFrame.current?.frameId !== session.frameId ||
          latest.current.suspended || latest.current.privacyGeneration !== session.privacyGeneration ||
          !latest.current.books.some((book) => book.id === session.location.bookId &&
            book.installationId === session.installationId)) return;
      // Admission and registration are one synchronous boundary after the
      // outgoing retirement actually drains; no timer or optimistic bypass.
      const registration = registerReaderStorage(node.contentWindow!, {
        origin: session.location.origin,
        bookId: session.location.bookId,
        privacyGeneration: session.privacyGeneration,
        retirementUrl: session.location.retirementUrl,
        retirementProof: session.location.retirementProof,
      }, node, deactivateFrame, restoreFrame);
      if (!registration) {
        if (activeFrame.current?.frameId === session.frameId) {
          cancelSensors(session.sessionId);
          activeFrame.current = null;
          void lifecycle(() => api("sensor_deactivate", { sessionId: session.sessionId }))
            .catch((value) => report(value, ERROR_SITES["reader.sensor-deactivate"]));
        }
        setFrame(frameNavigation.current?.node === node ? frameNavigation.current.session : null);
        setFrameReady(false);
        setLoading(false);
        setError(t("Archive storage is waiting for persistence acknowledgement"));
        return;
      }
      const previousRegistration = storageRegistration.current;
      storageRegistration.current = registration;
      frameNavigation.current = { node, frameId: session.frameId, session };
      previousRegistration?.();
      updateReaderStorageLocation(node.contentWindow!, {
        origin: session.location.origin,
        privacyGeneration: session.privacyGeneration,
        url: session.location.url,
      });
      // Whatever vouched before this navigation vouched for another document.
      bridgeVouched.current = false;
      // Reusing the context still runs the outgoing document's real pagehide.
      node.src = session.location.url;
    }).catch((value) => {
      if (!admission.signal.aborted && alive.current &&
          displayedFrame.current?.frameId === session.frameId) report(value);
    });
    return () => admission.abort();
  }, [frame?.frameId, cancelSensors, report, deactivateFrame, restoreFrame]);

  useEffect(() => {
    if (props.suspended) {
      clearHistoryTraversal();
      // Suspension always gates late article/sensor responses and actions.
      // Retained native media keeps only the existing document identity; an
      // actual retirement still clears it before any foreground activation.
      openSequence.current++;
      retainedActivation.current = props.mediaRetained ? activeFrame.current : null;
      if (retainedActivation.current) setFrameReady(false);
      if (!props.mediaRetained) {
        resolvedKey.current = "";
        freshOrigin.current = true;
      }
      return;
    }
    const key = `${tab.id}\n${tab.bookId ?? ""}\n${tab.path}\n${reload}\n${props.privacyGeneration}`;
    if (resolvedKey.current === key) {
      const retained = retainedActivation.current;
      const node = frameRef.current;
      const source = node?.contentWindow;
      if (!retained || !node || !source || !props.interactive) return;
      let cancelled = false;
      const current = () => !cancelled && alive.current &&
        retainedActivation.current === retained && activeFrame.current === retained &&
        displayedFrame.current?.frameId === retained.frameId &&
        frameRef.current === node && node.contentWindow === source &&
        latest.current.privacyGeneration === retained.privacyGeneration &&
        !latest.current.suspended && latest.current.interactive &&
        !latest.current.settings.privateMode && !latest.current.settings.readOnly &&
        latest.current.books.some((book) => book.id === retained.location.bookId &&
          book.installationId === retained.installationId) && canNavigateReaderStorage();
      // A late reply can arrive after suspension/interactivity changes but
      // while this exact document is still the retained owner. Deactivating
      // that session would retire valid background media; only a replaced
      // document or policy retirement releases it.
      const retainedOwner = () => alive.current &&
        retainedActivation.current === retained &&
        activeFrame.current === retained &&
        displayedFrame.current?.frameId === retained.frameId &&
        frameRef.current === node && node.contentWindow === source &&
        latest.current.privacyGeneration === retained.privacyGeneration &&
        !latest.current.settings.privateMode && !latest.current.settings.readOnly &&
        latest.current.books.some((book) => book.id === retained.location.bookId &&
          book.installationId === retained.installationId);
      // Native background handling deactivates sensors even when media keeps
      // the DOM alive. Restore that exact owner, never a fresh reader session.
      void lifecycle(async () => {
        if (!current()) return;
        const status = await api<SensorStatus>("mobile_speech_restore_reader", {
          sessionId: retained.sessionId, archiveId: retained.location.bookId,
          installationId: retained.installationId,
          origin: retained.location.origin, privacyGeneration: retained.privacyGeneration,
        }, current);
        // Native restoration is fenced by the exact retained session. A late
        // reply cannot deactivate a newer foreground owner.
        if (!current()) {
          if (!retainedOwner())
            void lifecycle(() => api("sensor_deactivate", { sessionId: retained.sessionId }))
              .catch((value) => latest.current.onError(value, ERROR_SITES["reader.sensor-deactivate"]));
          return;
        }
        retained.status = status;
        retainedActivation.current = null;
        setFrame((previous) => previous?.frameId === retained.frameId ? { ...previous, status } : previous);
        setFrameReady(true);
      }).catch((value) => {
        if (!current()) {
          // A failed restore after retirement/privacy still needs to release
          // any native owner the old session retained. A background media
          // handoff keeps that owner for the next foreground.
          if (!retainedOwner())
            void lifecycle(() => api("sensor_deactivate", { sessionId: retained.sessionId }))
              .catch((error) => latest.current.onError(error, ERROR_SITES["reader.sensor-deactivate"]));
          return;
        }
        // The retained owner is gone (or Rust never retained it). Drop the
        // stale activation and run a normal fresh open instead of leaving the
        // frame permanently unready.
        retainedActivation.current = null;
        resolvedKey.current = "";
        setFrame(null);
        setFrameReady(false);
        setReload((value) => value + 1);
        report(value);
      });
      return () => { cancelled = true; };
    }
    // A destination accepted while the document was stale is replayed once,
    // now that the re-established generation is live. Same-tab navigation
    // republishes the path so this effect opens it instead of the old one.
    const pending = pendingNavigation.current;
    if (pending && pending.generation !== props.privacyGeneration) {
      pendingNavigation.current = null;
      if (pending.newTab)
        latest.current.onOpenInNewTab(
          pending.location,
          props.privacyGeneration,
        );
      else if (
        pending.location.bookId === tabRef.current.bookId &&
        pending.location.path !== tabRef.current.path
      ) {
        publishTab({ ...tabRef.current, ...pending.location, scroll: 0 });
        return;
      }
    }
    clearHistoryTraversal();
    retainedActivation.current = null;
    const sequence = ++openSequence.current;
    previewSequence.current++;
    setPreview(null);
    setLinkContext(null);
    const previous = activeFrame.current;
    if (previous) {
      send({ command: "stop-speech" });
      send({ command: "pause-media" });
      latest.current.onMediaState?.({ audible: false, pictureInPicture: false });
      cancelSensors(previous.sessionId);
      void lifecycle(() =>
        api("sensor_deactivate", { sessionId: previous.sessionId }),
      ).catch((value) => report(value, ERROR_SITES["reader.sensor-deactivate"]));
    }
    clearTimeout(scrollTimer.current);
    persistVisit();
    lastVisit.current = null;
    activeFrame.current = null;
    resetFind();
    const retained = displayedFrame.current;
    const reuseContext =
      retained !== null &&
      retained.location.bookId === tab.bookId &&
      retained.privacyGeneration === props.privacyGeneration &&
      !freshOrigin.current;
    if (!reuseContext) setFrame(null);
    clearDocumentProbe();
    setHeldPaint("");
    setFrameReady(false);
    setToc([]);
    setSelection("");
    setPassageSelection(null);
    setPassageSelectionUnavailable(false);
    setPassageStatuses({});
    passageDocumentRef.current = null;
    setPassageDocument(null);
    setSpeaking(false);
    setExternal(null);
    setAttachment(null);
    if (
      previous &&
      bookmarkKey({
        bookId: previous.location.bookId,
        path: previous.location.path,
      }) !== bookmarkKey(tab)
    )
      setPanel((value) => (value === "bookmark" ? null : value));
    setBookmarkBusy(bookmarkWrite.current);
    setExportBusy(false);
    setError("");
    restoredDocument.current = "";
    if (!tab.bookId) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const requestedTab = tab;
    const requestedPrivacy = props.privacyGeneration;
    const requestedInstallation = props.books.find((book) => book.id === tab.bookId)?.installationId ?? null;
    let cancelled = false;
    const admission = new AbortController();
    void (async () => {
      await waitForReaderStorageNavigation(admission.signal);
      if (cancelled || !alive.current || openSequence.current !== sequence ||
          latest.current.privacyGeneration !== requestedPrivacy) return;
      const useFreshOrigin = freshOrigin.current;
      const location = await api<ReaderLocation>("open_article", {
        bookId: requestedTab.bookId,
        path: requestedTab.path || undefined,
        freshOrigin: useFreshOrigin,
        ...(requestedTab.restricted ? { restricted: true } : {}),
      });
      if (cancelled || !alive.current || openSequence.current !== sequence)
        return;
      const parsed = new URL(location.url);
      const retirement = new URL(location.retirementUrl);
      if (
        parsed.origin !== location.origin ||
        !["http:", "https:"].includes(parsed.protocol) ||
        parsed.username ||
        parsed.password ||
        location.bookId !== requestedTab.bookId ||
        location.origin === window.location.origin ||
        retirement.origin !== location.origin ||
        retirement.username ||
        retirement.password ||
        retirement.search ||
        retirement.hash ||
        !/^\/\.kiwix\/[a-f0-9]{32}\/retire\.html$/.test(retirement.pathname) ||
        !/^[a-f0-9]{32}$/.test(location.retirementProof) ||
        articlePath(location.url, location) === null
      )
        throw new Error(
          t("The archive server returned an unsafe reader address."),
        );
      const sessionId = crypto.randomUUID();
      let status: SensorStatus | null | undefined;
      for (;;) {
        // Never wait for retirement while occupying the sensor queue: its
        // outgoing owner may need that same queue to finish deactivation.
        await waitForReaderStorageNavigation(admission.signal);
        status = await lifecycle(async () => {
          if (cancelled || !alive.current || openSequence.current !== sequence ||
              latest.current.privacyGeneration !== requestedPrivacy) return null;
          // A checkpoint can begin while this operation waits its queue turn.
          // No native activation has run; defer admission, not the request.
          if (!canNavigateReaderStorage()) return undefined;
          const activate = () => api<SensorStatus>("sensor_activate", {
            sessionId, archiveId: location.bookId, origin: location.origin,
          });
          try {
            return await activate();
          } catch (value) {
            if (cancelled || !alive.current || openSequence.current !== sequence ||
                latest.current.privacyGeneration !== requestedPrivacy) throw value;
            // Opening a fresh origin can briefly replace the native listener
            // while the previous reader is retiring. Retry that handoff once;
            // if it still fails, keep reading and let the next explicit sensor
            // request report the real error instead of showing a loading banner.
            if (String(value).includes("Sensor origin is not an active archive listener")) {
              await new Promise((resolve) => setTimeout(resolve, 120));
              if (cancelled || !alive.current || openSequence.current !== sequence ||
                  latest.current.privacyGeneration !== requestedPrivacy) throw value;
              try {
                return await activate();
              } catch (retry) {
                if (String(retry).includes("Sensor origin is not an active archive listener"))
                  return {
                    camera: "notAsked", location: "notAsked", nativeLocation: false,
                  } satisfies SensorStatus;
                report(retry, ERROR_SITES["reader.sensor-activate"]);
                return {
                  camera: "notAsked", location: "notAsked", nativeLocation: false,
                } satisfies SensorStatus;
              }
            }
            report(value, ERROR_SITES["reader.sensor-activate"]);
            return {
              camera: "notAsked", location: "notAsked", nativeLocation: false,
            } satisfies SensorStatus;
          }
        });
        if (status !== undefined) break;
      }
      if (!status) return;
      try {
        await waitForReaderStorageNavigation(admission.signal);
      } catch (error) {
        void lifecycle(() => api("sensor_deactivate", { sessionId })).catch(
          (value) => latest.current.onError(value, ERROR_SITES["reader.sensor-deactivate"]),
        );
        throw error;
      }
      if (
        cancelled ||
        !alive.current ||
        openSequence.current !== sequence ||
        latest.current.privacyGeneration !== requestedPrivacy ||
        latest.current.suspended
      ) {
        void lifecycle(() => api("sensor_deactivate", { sessionId })).catch(
          (value) =>
            latest.current.onError(
              value,
              ERROR_SITES["reader.sensor-deactivate"],
            ),
        );
        return;
      }
      const session: FrameSession = {
        location,
        sessionId,
        frameId: sessionId,
        contextId:
          reuseContext &&
          retained &&
          retained.location.origin === location.origin
            ? retained.contextId
            : sessionId,
        privacyGeneration: requestedPrivacy,
        installationId: requestedInstallation,
        status,
      };
      freshOrigin.current = false;
      activeFrame.current = session;
      // An archive without a main page opens a random article instead of
      // failing; say so quietly and keep the tab on the page on screen.
      if (location.noMainPage === true) {
        const fallback = new Error(
          t("This archive has no main page. Showing a random article."),
        );
        setNotice(fallback.message);
        report(fallback, ERROR_SITES["reader.navigation-missing"]);
      } else if (location.missing === true) {
        // The archive answered with its main document because the requested
        // entry is not in it. That is a navigation miss, not a failure: say so
        // quietly and keep the tab on the page that is actually on screen.
        const missing = new Error(
          t("That article is not in this archive. Showing its main page."),
        );
        setNotice(missing.message);
        report(missing, ERROR_SITES["reader.navigation-missing"]);
      }
      const next = {
        ...tabRef.current,
        path: location.path,
        title: location.title || tabRef.current.title,
        restricted: location.restricted === true,
      };
      scrollPosition.current = next.scroll;
      resolvedKey.current = `${next.id}\n${next.bookId ?? ""}\n${next.path}\n${reload}\n${requestedPrivacy}`;
      publishTab(next);
      setFrame(session);
    })().catch((value) => {
      if (!cancelled && alive.current && openSequence.current === sequence) {
        setLoading(false);
        if (message(value) === NO_ARTICLES) {
          setNotice(t("This archive has no articles to open."));
          report(value, ERROR_SITES["reader.navigation-missing"]);
        } else {
          report(value);
        }
      }
    });
    return () => {
      cancelled = true;
      admission.abort();
    };
  }, [
    tab.id,
    tab.bookId,
    tab.path,
    props.privacyGeneration,
    props.suspended,
    props.mediaRetained,
    props.interactive,
    reload,
    cancelSensors,
    clearDocumentProbe,
    clearHistoryTraversal,
    persistVisit,
    publishTab,
    report,
    resetFind,
    send,
  ]);

  const leaveDocument = useCallback(() => {
    latest.current.clearMediaCheckpoint();
    setAttachment(null);
    previewSequence.current++;
    setPreview(null);
    setPanel((value) => (value === "bookmark" ? null : value));
    void deactivateFrame().catch(report);
  }, [deactivateFrame, report, setPanel]);
  const navigate = useCallback(
    (location: TabLocation) => {
      const current = tabRef.current;
      updateWorkspace({ historyNavigationPending: true });
      historyNavigationPending.current = true;
      const previous = currentLocation({
        ...current,
        scroll: scrollPosition.current,
      });
      if (
        previous?.bookId === location.bookId &&
        previous.path === location.path
      ) {
        if (!activeFrame.current) {
          openSequence.current++;
          resolvedKey.current = "";
          setReload((value) => value + 1);
          return;
        }
        scrollPosition.current = location.scroll;
        send({ command: "scroll", x: 0, y: location.scroll });
        historyNavigationPending.current = false;
        updateWorkspace({ historyNavigationPending: false });
        rememberVisit(location, activeFrame.current.privacyGeneration);
        persistVisit(true);
        publishTab({ ...current, scroll: location.scroll });
        return;
      }
      leaveDocument();
      scrollPosition.current = location.scroll;
      publishTab({
        ...current,
        ...location,
        back: previous ? boundedHistory([...current.back, previous]) : current.back,
        forward: [],
      });
    },
    [leaveDocument, publishTab, send, updateWorkspace, rememberVisit, persistVisit],
  );
  // Shell and in-page destinations share one authority check: a destination
  // produced by a document that already lost its generation is queued and
  // replayed once after the reader re-bootstraps, never executed on stale
  // authority and never silently discarded.
  const openDestination = useCallback(
    (location: TabLocation, newTab: boolean) => {
      const session = activeFrame.current;
      if (
        session &&
        session.privacyGeneration !== latest.current.privacyGeneration
      ) {
        pendingNavigation.current = {
          generation: session.privacyGeneration,
          location,
          newTab,
        };
        reestablishAuthority();
        return;
      }
      if (newTab)
        latest.current.onOpenInNewTab(
          location,
          latest.current.privacyGeneration,
        );
      else navigate(location);
    },
    [navigate, reestablishAuthority],
  );
  const historySourceCurrent = useCallback((pending: HistoryTraversal) => {
    const current = tabRef.current;
    return alive.current && !latest.current.suspended && latest.current.interactive &&
      latest.current.privacyGeneration === pending.session.privacyGeneration &&
      displayedFrame.current?.frameId === pending.session.frameId &&
      current.id === pending.source.id && current.bookId === pending.source.bookId &&
      current.path === pending.source.path && sameHistoryStack(current.back, pending.source.back) &&
      sameHistoryStack(current.forward, pending.source.forward) &&
      latest.current.books.some((book) => book.id === current.bookId &&
        book.installationId === pending.session.installationId);
  }, []);
  const reopenHistoryTraversal = useCallback((pending: HistoryTraversal, timedOut = false) => {
    if (pendingHistoryTraversal.current !== pending) return;
    const current = historySourceCurrent(pending) &&
      activeFrame.current?.sessionId === pending.session.sessionId;
    clearHistoryTraversal();
    if (!current) return;
    const reopen = () => {
      leaveDocument();
      scrollPosition.current = pending.next.scroll;
      publishTab(pending.next);
    };
    if (!timedOut) { reopen(); return; }
    // A queued history.go() cannot be cancelled. Destroy its producer through
    // the existing genuine-pagehide/storage handoff before the reload fallback.
    const source = frameRef.current?.contentWindow;
    const retirement = retireReaderDocuments({ restoreOnFailure: false });
    const sequence = openSequence.current;
    void retirement.then(() => {
      if (openSequence.current !== sequence || activeFrame.current ||
          !historySourceCurrent(pending)) return;
      freshOrigin.current = true;
      setFrame(null);
      reopen();
    }).catch((value) => {
      if (source) quarantineReaderStorage(source);
      if (!alive.current || openSequence.current !== sequence ||
          !historySourceCurrent(pending)) return;
      setFrame(null);
      setFrameReady(false);
      setLoading(false);
      report(value);
    });
  }, [clearHistoryTraversal, historySourceCurrent, leaveDocument, publishTab, report]);
  const travel = useCallback(
    (direction: "back" | "forward", steps = 1) => {
      if (pendingHistoryTraversal.current) return;
      const current = tabRef.current;
      const stack = current[direction];
      const count = Math.min(stack.length, Math.max(1, Math.floor(steps)));
      const destination = stack.at(-count);
      if (!destination) return;
      historyNavigationPending.current = true;
      updateWorkspace({ historyNavigationPending: true });
      const previous = currentLocation({
        ...current,
        scroll: scrollPosition.current,
      });
      const next: Tab = {
        ...current,
        ...destination,
        [direction]: stack.slice(0, -count),
        [direction === "back" ? "forward" : "back"]: boundedHistory([
          ...current[direction === "back" ? "forward" : "back"],
          ...(previous ? [previous] : []),
          ...stack.slice(stack.length - count + 1).reverse(),
        ]),
      };
      const session = activeFrame.current;
      const locations = stack.slice(-count).reverse();
      if (session && frameReady && canNavigateReaderStorage() && count <= 200 &&
          locations.every((location) => location.bookId === session.location.bookId)) {
        const pending: HistoryTraversal = { requestId: crypto.randomUUID(), session, source: current, next };
        pendingHistoryTraversal.current = pending;
        if (historySourceCurrent(pending)) {
          persistVisit();
          pending.timer = window.setTimeout(() => reopenHistoryTraversal(pending, true), 5000);
          send({
            command: "history-traverse", requestId: pending.requestId,
            url: session.location.url, direction, y: destination.scroll,
            destinations: locations.map((location) => new URL(location.path, session.location.baseUrl).href),
          });
          return;
        }
        clearHistoryTraversal();
      }
      leaveDocument();
      scrollPosition.current = destination.scroll;
      publishTab(next);
    },
    [clearHistoryTraversal, frameReady, historySourceCurrent, leaveDocument, persistVisit,
      publishTab, reopenHistoryTraversal, send, updateWorkspace],
  );
  const scrollToTop = useCallback(() => {
    if (!frame) return;
    scrollPosition.current = 0;
    send({ command: "scroll", x: 0, y: 0 });
    publishTab({ ...tabRef.current, scroll: 0 });
  }, [frame, publishTab, send]);
  const randomArticle = useCallback(async () => {
    const current = tabRef.current;
    if (!current.bookId) return;
    const sequence = ++navigationSequence.current;
    try {
      const location = await api<ReaderLocation>("random_article", {
        bookId: current.bookId,
      });
      if (
        alive.current &&
        sequence === navigationSequence.current &&
        tabRef.current.bookId === current.bookId &&
        tabRef.current.path === current.path
      ) {
        // The native session schema accepts only tab-location fields; spreading
        // the full ReaderLocation (url/origin/baseUrl/...) makes the save fail.
        navigate({
          bookId: location.bookId,
          path: location.path,
          title: location.title,
          scroll: 0,
        });
      }
    } catch (value) {
      if (alive.current && sequence === navigationSequence.current) {
        // "Random article" is an optional capability: an archive with nothing
        // to open keeps the current page and reports quietly.
        setNotice(message(value) === NO_ARTICLES
          ? t("This archive has no articles to open.") : message(value));
        report(value, ERROR_SITES["reader.random-article"]);
      }
    }
  }, [navigate, report]);
  const pinCurrentArticle = useCallback(async () => {
    if (!isAndroid) return;
    const current = tabRef.current;
    const currentSettings = latest.current.settings;
    if (
      !current.bookId ||
      !current.path ||
      currentSettings.privateMode ||
      currentSettings.readOnly
    )
      return;
    const archive = latest.current.books.find(
      (item) => item.id === current.bookId,
    );
    try {
      const result = await api<{ pinned?: unknown }>("mobile_pin_shortcut", {
        bookId: current.bookId,
        path: current.path,
        title: current.title || archive?.title || t("Article"),
      });
      if (!alive.current) return;
      if (result?.pinned !== true) {
        setNotice(t("This launcher does not support pinned shortcuts."));
      } else {
        setNotice(t("Pinned to your home screen."));
      }
    } catch (value) {
      if (alive.current) report(value, ERROR_SITES["reader.pin-shortcut"]);
    }
  }, [report]);
  const updateZoom = useCallback((zoom: number | null) => {
    const currentBook = tabRef.current.bookId;
    if (!alive.current || !currentBook) return;
    const value = zoom === null ? null : Math.max(0.25, Math.min(5, zoom));
    if (latest.current.settings.readOnly) {
      latest.current.onWorkspaceChange((previous) => ({
        ...previous,
        bookZoom: { ...previous.bookZoom, [currentBook]: value },
      }));
      settingsRef.current = {
        ...settingsRef.current,
        zoom: value ?? latest.current.settings.zoom,
      };
    } else if (saveBookZoom(currentBook, value, preferenceHandlers.current)) {
      settingsRef.current = {
        ...settingsRef.current,
        zoom:
          value ??
          getReaderPreferences().pending.zoom ??
          getReaderPreferences().confirmed?.zoom ??
          latest.current.settings.zoom,
      };
    }
  }, []);
  const stepZoom = useCallback((direction: -1 | 1) => {
    updateZoom(Math.round((settingsRef.current.zoom + direction * 0.1) * 100) / 100);
  }, [updateZoom]);
  const updateSettings = useCallback((patch: Partial<Omit<ReaderAppearance, "speechPreferences">>) => {
    if (latest.current.settings.readOnly) {
      settingsRef.current = { ...settingsRef.current, ...patch };
      latest.current.onWorkspaceChange((previous) => ({
        ...previous,
        appearancePatch: { ...previous.appearancePatch, ...patch },
      }));
      return true;
    }
    if (!saveReaderPreferences(patch, preferenceHandlers.current)) return false;
    settingsRef.current = { ...settingsRef.current, ...patch };
    return true;
  }, []);
  const updateSpeech = useCallback((patch: Partial<SpeechPreference>, language = speechContext.current.language) => {
    const shared = getReaderPreferences();
    const current = !latest.current.settings.readOnly &&
      shared.privateMode === latest.current.settings.privateMode &&
      shared.readOnly === latest.current.settings.readOnly
      ? shared.pending.speechPreferences ?? shared.confirmed?.speechPreferences ?? settingsRef.current.speechPreferences
      : settingsRef.current.speechPreferences;
    const preference = { ...speechPreference(current, language), ...patch };
    if (Object.keys(patch).length) {
      const speechPreferences = { ...current, [language]: preference };
      if (latest.current.settings.readOnly) {
        latest.current.onWorkspaceChange((previous) => ({
          ...previous, appearancePatch: { ...previous.appearancePatch, speechPreferences },
        }));
      } else if (!saveSpeechPreference(language, patch, preferenceHandlers.current)) return;
      settingsRef.current = { ...settingsRef.current, speechPreferences };
    }
    const voice = selectedSpeechVoice(speechContext.current.voices, language, preference.voice);
    const neural = voice?.voiceURI.startsWith("kokoro:") ?? false;
    const voiceURI = (isMobile || neural) && !matchingSpeechVoices(speechContext.current.voices, language)
      .some((candidate) => candidate.voiceURI === voice?.voiceURI) ? "" : voice?.voiceURI ?? "";
    const options = { lang: language, rate: preference.rate, voice: voiceURI };
    const nativeOptions = { ...options, title: tabRef.current.title };
    if (isMobile) { void nativeSpeech.preferences(nativeOptions); return; }
    // Desktop: each engine applies its own preferences live; a change that
    // moves reading to the other engine stops it (the next play uses it).
    if (nativeReadingRef.current) {
      if (neural && voiceURI) void nativeSpeech.preferences(nativeOptions);
      else void nativeSpeech.stop();
      return;
    }
    if (neural) send({ command: "stop-speech" });
    else send({ command: "speech-preferences", ...options });
  }, [send, nativeSpeech.preferences, nativeSpeech.stop]);
  const adjustSpeech = useCallback((direction: number) => {
    const context = speechContext.current;
    const previousRate = speechPreference(settingsRef.current.speechPreferences, context.language).rate;
    const rate = adjustedSpeechRate(previousRate, direction);
    if (rate !== previousRate) updateSpeech({ rate });
  }, [updateSpeech]);
  useEffect(() => {
    if (!frameReady) return;
    send({ command: "zoom", value: readerSettings.zoom });
    send({
      command: "appearance",
      theme: readerSettings.readerTheme,
      fontFamily: readerSettings.fontFamily,
      fontSize: readerSettings.fontSize,
      width: readerSettings.readerWidth,
    });
  }, [
    frameReady,
    readerSettings.zoom,
    readerSettings.readerTheme,
    readerSettings.fontFamily,
    readerSettings.fontSize,
    readerSettings.readerWidth,
    send,
  ]);

  const completeSensor = useCallback(
    // `live` is the store status the auto-grant path just re-read. It skips the
    // consent write: replaying a grant must never re-create one the user removed.
    async (request: SensorRequest, allowed: boolean, live?: SensorStatus) => {
      const pending = pendingSensors.current.get(sensorKey(request));
      const session = activeFrame.current;
      if (
        !pending ||
        pending.cancelled ||
        !session ||
        session.sessionId !== request.sessionId
      )
        return;
      // Record the user's explicit choice before the write so a failure still
      // latches the safer answer for requests in this document.
      const decisionKey = sensorDecisionKey(
        session.installationId,
        request.sensor,
      );
      if (decisionKey)
        sensorDecisions.current.set(decisionKey, allowed ? "granted" : "denied");
      try {
        const status =
          live ??
          (await lifecycle(() =>
            api<SensorStatus>("sensor_consent", {
              sessionId: request.sessionId,
              sensor: request.sensor,
              allowed,
            }),
          ));
        const current = activeFrame.current;
        if (
          pending.cancelled ||
          !current ||
          current.sessionId !== request.sessionId ||
          !alive.current
        )
          return;
        const updated = {
          ...current,
          status: {
            ...current.status,
            [request.sensor]: status[request.sensor],
            nativeLocation: status.nativeLocation,
          },
        };
        activeFrame.current = updated;
        setFrame(updated);
        if (consentRef.current?.requestId === request.requestId) {
          consentRef.current = null;
          setConsent(null);
          setConsentBusy(false);
        }
        if (!allowed || status[request.sensor] !== "granted") {
          replySensor(request, {
            allowed: false,
            error: { code: 1, message: t("Permission was denied.") },
          });
          return;
        }
        if (request.sensor === "camera") {
          replySensor(request, { allowed: true, browser: true });
          return;
        }
        if (!status.nativeLocation) {
          // Never hand the archive the webview's own geolocation provider: it
          // raises a browser permission prompt (repeatedly, per context) instead
          // of this app's single native consent, and it bypasses the archive
          // scope. Report unavailability to the page instead.
          replySensor(request, {
            allowed: false,
            error: { code: 2, message: "Location is unavailable on this platform" },
          });
          return;
        }
        const position = await api("sensor_location", {
          sessionId: request.sessionId,
          requestId: pending.nativeId,
          highAccuracy: request.options?.enableHighAccuracy === true,
          timeoutMs: Math.round(
            Math.min(30_000, Math.max(0, request.options?.timeout ?? 30_000)),
          ),
        });
        if (
          !pending.cancelled &&
          activeFrame.current?.sessionId === request.sessionId &&
          alive.current
        )
          replySensor(request, { allowed: true, position });
      } catch (value) {
        if (
          !pending.cancelled &&
          activeFrame.current?.sessionId === request.sessionId &&
          alive.current
        ) {
          let error = { code: 2, message: message(value) };
          let cancelled = false;
          try {
            const locationError: unknown = JSON.parse(error.message);
            if (
              object(locationError) &&
              (locationError.code === 1 ||
                locationError.code === 2 ||
                locationError.code === 3) &&
              typeof locationError.message === "string"
            ) {
              cancelled = locationError.code === 2 && locationError.cancelled === true;
              error = {
                code: locationError.code,
                message: locationError.message,
              };
            }
          } catch {
            // Non-location API failures retain POSITION_UNAVAILABLE.
          }
          replySensor(request, {
            allowed: false,
            error,
            cancelled,
          });
          if (!cancelled)
            latest.current.onError(
              new Error(error.message),
              ERROR_SITES["reader.sensor-location"],
            );
        }
      } finally {
        pendingSensors.current.delete(sensorKey(request));
        if (
          alive.current &&
          activeFrame.current?.sessionId === request.sessionId
        ) {
          if (consentRef.current?.requestId === request.requestId) {
            consentRef.current = null;
            setConsent(null);
          }
          setConsentBusy(false);
        }
      }
    },
    [replySensor],
  );
  const promptSensor = useCallback(
    (request: SensorRequest) => {
      if (!consentRef.current) {
        consentRef.current = request;
        setConsent(request);
        return;
      }
      replySensor(request, {
        allowed: false,
        error: {
          code: 1,
          message: t(
            "Another permission request is already open. Please try again.",
          ),
        },
      });
      pendingSensors.current.delete(sensorKey(request));
    },
    [replySensor],
  );
  const confirmSensorGrant = useCallback(
    async (request: SensorRequest) => {
      const pending = pendingSensors.current.get(sensorKey(request));
      const session = activeFrame.current;
      if (
        !pending ||
        pending.cancelled ||
        !session ||
        session.sessionId !== request.sessionId
      )
        return;
      // The frame status is only a cache from its last native call. Re-read the
      // live decision before replaying a grant, so a decision the user just
      // changed (or one made in another reader session) cannot be auto-allowed.
      // Naming the installation lets an inactive session report its remembered
      // decision instead of a fail-closed denial that would overwrite the UI.
      let live: SensorStatus | null = null;
      try {
        live = await lifecycle(() =>
          api<SensorStatus>("sensor_status", {
            sessionId: request.sessionId,
            ...(session.installationId
              ? { installationId: session.installationId }
              : {}),
          }),
        );
      } catch {
        live = null;
      }
      const current = activeFrame.current;
      if (
        pending.cancelled ||
        !current ||
        current.sessionId !== request.sessionId ||
        !alive.current
      )
        return;
      if (live) {
        const updated = {
          ...current,
          status: {
            ...current.status,
            camera: live.camera,
            location: live.location,
            nativeLocation: live.nativeLocation,
          },
        };
        activeFrame.current = updated;
        setFrame(updated);
      }
      const decisionKey = sensorDecisionKey(
        current.installationId,
        request.sensor,
      );
      const intent = decisionKey
        ? sensorDecisions.current.get(decisionKey)
        : undefined;
      const disposition = live
        ? sensorRequestDisposition(live[request.sensor], intent)
        : intent === "denied"
          ? "deny"
          : "prompt";
      if (disposition === "allow" && live) {
        await completeSensor(request, true, live);
        return;
      }
      if (disposition === "deny") {
        replySensor(request, {
          allowed: false,
          error: { code: 1, message: t("Permission was denied.") },
        });
        pendingSensors.current.delete(sensorKey(request));
        return;
      }
      promptSensor(request);
    },
    [completeSensor, promptSensor, replySensor],
  );
  const drainPreview = useCallback(function drain(): void {
    const gate = previewGate.current;
    if (gate.pending || gate.timer !== undefined) return;
    const queued = previewQueued.current;
    if (!queued) return;
    const { session, path, sequence } = queued;
    const isCurrent = () =>
      alive.current &&
      latest.current.settings.linkPreviews &&
      !latest.current.suspended &&
      latest.current.interactive &&
      canNavigateReaderStorage() &&
      session.privacyGeneration === latest.current.privacyGeneration &&
      sequence === previewSequence.current &&
      activeFrame.current?.sessionId === session.sessionId;
    if (!isCurrent()) {
      previewQueued.current = null;
      return;
    }
    const delay = Math.max(0, 400 - (performance.now() - gate.at));
    if (delay > 0) {
      gate.timer = window.setTimeout(() => {
        gate.timer = undefined;
        drain();
      }, delay);
      return;
    }
    previewQueued.current = null;
    gate.pending = true;
    gate.at = performance.now();
    setPreview({
      bookId: session.location.bookId,
      requestedPath: path,
      article: null,
    });
    void api<ArticlePreview>("article_preview", {
      bookId: session.location.bookId,
      path,
    })
      .then((article) => {
        if (!isCurrent()) return;
        if (
          !object(article) ||
          !text(article.title, 4096) ||
          !text(article.excerpt, 8192) ||
          !text(article.path, 16_384) ||
          !text(article.mime, 256)
        ) {
          setPreview(null);
          return;
        }
        if (article.thumbnail !== null) {
          const thumbnail = article.thumbnail;
          const match =
            typeof thumbnail === "string" &&
            thumbnail.length <= 4 * Math.ceil(MAX_PREVIEW_THUMBNAIL_BYTES / 3) + 23
              ? /^data:image\/(?:png|jpeg|gif|webp);base64,([A-Za-z0-9+/]+={0,2})$/.exec(thumbnail)
              : null;
          const encoded = match?.[1] ?? "";
          const padding = encoded.endsWith("==") ? 2 : encoded.endsWith("=") ? 1 : 0;
          if (
            !match ||
            match[0] !== thumbnail ||
            encoded.length % 4 !== 0 ||
            encoded.length / 4 * 3 - padding > MAX_PREVIEW_THUMBNAIL_BYTES
          ) {
            setPreview(null);
            return;
          }
        }
        // Only the validated requested path controls either explicit Open action.
        setPreview({
          bookId: session.location.bookId,
          requestedPath: path,
          article,
        });
      })
      .catch(() => {
        if (alive.current && previewSequence.current === sequence)
          setPreview(null);
      })
      .finally(() => {
        gate.pending = false;
        drain();
      });
  }, []);

  useEffect(() => {
    if (!settings.linkPreviews) {
      previewSequence.current++;
      clearTimeout(previewGate.current.timer);
      previewGate.current.timer = undefined;
      previewQueued.current = null;
      setPreview(null);
    }
    if (frameReady) send({ command: "link-previews", enabled: settings.linkPreviews });
  }, [settings.linkPreviews, frameReady, send]);

  // Desktop photo chooser: deliver one image (or a cancellation) to the exact
  // frame that asked, or nothing at all if that frame has gone or changed.
  const photoCurrent = useCallback((request: PhotoRequest) => {
    const session = activeFrame.current;
    return !!session &&
      session.frameId === request.frameId &&
      session.sessionId === request.sessionId &&
      session.location.bookId === request.bookId &&
      session.location.origin === request.origin &&
      latest.current.privacyGeneration === request.privacyGeneration;
  }, []);
  const finishPhoto = useCallback((request: PhotoRequest, photo: PickedPhoto | null) => {
    if (photoRequestRef.current !== request) return;
    photoRequestRef.current = null;
    setPhotoRequest(null);
    setPhotoStage("choose");
    setPhotoError("");
    const target = frameRef.current?.contentWindow;
    if (!target || !photoCurrent(request)) return;
    const base = { channel: "kiwix-reader", version: 1, sessionId: request.sessionId, requestId: request.requestId };
    if (!photo) {
      target.postMessage({ ...base, type: "photo-cancel" }, request.origin);
      return;
    }
    const binary = atob(photo.data);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
    target.postMessage({
      ...base,
      type: "photo-result",
      name: photo.name,
      mime: photo.mime,
      lastModified: photo.lastModified ?? Date.now(),
      data: bytes.buffer,
    }, request.origin, [bytes.buffer]);
  }, [photoCurrent]);
  const choosePhotoFile = useCallback(async () => {
    const request = photoRequestRef.current;
    if (!request) return;
    setPhotoError("");
    setPhotoStage("picking");
    try {
      const picked = await api<PickedPhoto | null>("pick_image", { accept: request.accept });
      if (photoRequestRef.current !== request) return;
      if (!picked) {
        setPhotoStage("choose");
        return;
      }
      finishPhoto(request, picked);
    } catch (error) {
      if (photoRequestRef.current !== request) return;
      setPhotoError(errorText(error));
      setPhotoStage("choose");
    }
  }, [finishPhoto]);
  const openPhotoCamera = useCallback(async () => {
    const request = photoRequestRef.current;
    if (!request) return;
    setPhotoError("");
    setPhotoStage("camera");
    try {
      await api("camera_capture_open", { capture: request.capture, accept: request.accept, requestId: request.requestId });
    } catch (error) {
      if (photoRequestRef.current !== request) return;
      setPhotoError(errorText(error));
      setPhotoStage("choose");
    }
  }, []);
  const cancelPhoto = useCallback(() => {
    const request = photoRequestRef.current;
    if (!request) return;
    void api("camera_capture_close", {}).catch(() => undefined);
    finishPhoto(request, null);
  }, [finishPhoto]);
  useEffect(() => {
    if (!isDesktop) return;
    void api<boolean>("camera_capture_supported", {}).then(setCameraSupported).catch(() => setCameraSupported(false));
  }, []);
  // The camera window reports here: one photo, or a cancellation (with the
  // reason the camera could not start, if any).
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/webviewWindow")
      .then(({ getCurrentWebviewWindow }) => getCurrentWebviewWindow()
      .listen<{ status: string; requestId?: string; reason?: string | null } & Partial<PickedPhoto>>("photo-capture", ({ payload }) => {
        const request = photoRequestRef.current;
        // A result belongs to exactly the request that opened the camera.
        if (!request || payload.requestId !== request.requestId) return;
        if (payload.status === "photo" && typeof payload.data === "string" &&
            typeof payload.name === "string" && typeof payload.mime === "string") {
          finishPhoto(request, payload as PickedPhoto);
          return;
        }
        // Back to the choice: the user can still choose a file.
        setPhotoStage("choose");
        if (payload.reason) setPhotoError(String(payload.reason));
      }))
      .then((remove) => {
        if (disposed) remove();
        else unlisten = remove;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [finishPhoto]);
  // Keep keyboard focus in the dialog while the choices are busy.
  useEffect(() => {
    if (photoRequest && photoStage !== "choose") photoCancelButton.current?.focus();
  }, [photoRequest, photoStage]);
  // A request never outlives the document, tab or privacy generation it came from.
  useEffect(() => {
    const request = photoRequestRef.current;
    if (request && !photoCurrent(request)) {
      void api("camera_capture_close", {}).catch(() => undefined);
      photoRequestRef.current = null;
      setPhotoRequest(null);
      setPhotoStage("choose");
      setPhotoError("");
    }
  });
  // A new iframe context is the only trustworthy document boundary. Archive
  // code can report a navigation or rewrite its fragment without replacing the
  // document, so URL changes never reopen gesture registration.
  useLayoutEffect(() => {
    gestureHello.current = null;
    awaitingGestureHello.current = true;
    documentAnnounced.current = null;
    bridgeVouched.current = false;
  }, [frame?.contextId]);

  useEffect(() => {
    const receive = (event: MessageEvent<unknown>) => {
      const session = activeFrame.current;
      const source = frameRef.current?.contentWindow;
      // The frame's own window at the displayed origin. Announcements and
      // proofs are taken here, before the session gate: they matter while a
      // re-session is in flight and for sessions that are no longer admitted.
      const framed =
        source &&
        event.source === source &&
        event.isTrusted &&
        event.origin === displayedFrame.current?.location.origin &&
        object(event.data) &&
        event.data.channel === "kiwix-reader" &&
        event.data.version === 1
          ? event.data
          : null;
      if (
        framed?.type === "reading-gesture-hello" &&
        text(framed.token, 36) &&
        /^[0-9a-f-]{36}$/.test(framed.token)
      ) {
        // Every archive document's bridge announces itself at bootstrap,
        // before archive scripts, and again when restored from the
        // back-forward cache.
        const authority = panAuthority.current;
        const own =
          session !== null &&
          authority?.sessionId === session.sessionId &&
          authority.token === framed.token;
        documentAnnounced.current = framed.token;
        // The admitted session's own bridge repeating itself vouches for
        // nothing new: that document already has its session.
        if (!own) bridgeVouched.current = true;
        const probe = documentProbe.current;
        if (
          probe?.kept &&
          session &&
          probe.sessionId === session.sessionId &&
          (probe.nonce === null || documentReplaced(session))
        ) {
          // The load being proved belonged to this other document: its
          // announcement can follow its load.
          clearDocumentProbe();
          resessionUnproven(probe, probe.painted);
        } else settlePendingInit();
      } else if (framed?.type === "document-proof") {
        // Only the bridge that answered a session's reader-init holds the token
        // bound to it, and it answers with the nonce of the latest probe. For
        // an admitted session the proof settles that one load. For a session
        // that already lost admission it only vouches that this bridge is
        // still listening, so the replacement session may be handed to it.
        const probe = documentProbe.current;
        const authority = panAuthority.current;
        if (
          probe &&
          probe.nonce !== null &&
          text(framed.nonce, 36) &&
          framed.nonce === probe.nonce &&
          framed.sessionId === probe.sessionId &&
          authority?.sessionId === probe.sessionId &&
          authority.token !== "" &&
          text(framed.token, 36) &&
          framed.token === authority.token
        ) {
          if (!probe.kept) {
            clearDocumentProbe();
            bridgeVouched.current = true;
            settlePendingInit();
          } else if (session?.sessionId === probe.sessionId) clearDocumentProbe();
        }
        return;
      }
      if (
        !session ||
        !source ||
        event.source !== source ||
        event.origin !== session.location.origin ||
        !object(event.data)
      )
        return;
      const data = event.data;
      if (
        data.channel !== "kiwix-reader" ||
        data.version !== 1 ||
        !text(data.type, 40)
      )
        return;
      if (data.type === "reading-gesture-hello") {
        // The bridge announces before archive scripts; the first announcement
        // of a document wins, so a script-replaced document cannot take over.
        if (
          event.isTrusted &&
          awaitingGestureHello.current &&
          text(data.token, 36) &&
          /^[0-9a-f-]{36}$/.test(data.token)
        ) {
          gestureHello.current = {
            frameId: session.frameId,
            token: data.token,
          };
          awaitingGestureHello.current = false;
        }
        return;
      }
      if (data.sessionId !== session.sessionId) return;
      if (session.privacyGeneration !== latest.current.privacyGeneration) {
        clearHistoryTraversal();
        // The mounted document predates this window's generation: it cannot
        // navigate, write or request sensors until the reader re-bootstraps.
        // Preserve one genuine in-page destination for a bounded replay, then
        // re-establish authority instead of dropping the action silently.
        if (data.type === "external-link" && typeof data.newTab === "boolean" &&
            text(data.url, 16_384)) {
          const path = articlePath(data.url, session.location);
          if (path !== null)
            pendingNavigation.current = {
              generation: session.privacyGeneration,
              location: {
                bookId: session.location.bookId,
                path,
                title: t("Article"),
                scroll: 0,
              },
              newTab: data.newTab,
            };
        } else if (
          (data.type === "navigation" || data.type === "ready") &&
          text(data.url, 16_384)
        ) {
          const path = articlePath(data.url, session.location);
          if (path !== null && path !== tabRef.current.path)
            pendingNavigation.current = {
              generation: session.privacyGeneration,
              location: {
                bookId: session.location.bookId,
                path,
                title:
                  text(data.title, 4096) && data.title.trim()
                    ? data.title
                    : tabRef.current.title,
                scroll: 0,
              },
              newTab: false,
            };
        }
        reestablishAuthority();
        return;
      }
      if (data.type === "photo-request") {
        if (!text(data.requestId, 36) || !/^[0-9a-f-]{36}$/.test(data.requestId)) return;
        const reply = (type: "photo-accepted" | "photo-cancel") =>
          frameRef.current?.contentWindow?.postMessage(
            { channel: "kiwix-reader", version: 1, type, sessionId: session.sessionId, requestId: data.requestId },
            session.location.origin,
          );
        // Only the bridge can ask: it proves itself with this session's secret,
        // which it keeps from archive scripts. One request at a time. Any refusal
        // is answered, so the bridge never leaves the input dead.
        if (
          !event.isTrusted ||
          !photoSecrets.current.get(session.sessionId) ||
          data.secret !== photoSecrets.current.get(session.sessionId) ||
          photoRequestRef.current
        ) {
          reply("photo-cancel");
          return;
        }
        reply("photo-accepted");
        const request: PhotoRequest = {
          frameId: session.frameId,
          sessionId: session.sessionId,
          bookId: session.location.bookId,
          origin: session.location.origin,
          privacyGeneration: session.privacyGeneration,
          requestId: data.requestId,
          accept: text(data.accept, 1024) && data.accept.trim() ? data.accept : "image/*",
          capture: data.capture === "user" ? "user" : "environment",
        };
        photoRequestRef.current = request;
        setPhotoError("");
        setPhotoStage("choose");
        setPhotoRequest(request);
        return;
      }
      if (data.type === "reading-pan-bind") {
        // Pan is UI-only authority and keeps its per-session first-writer
        // binding. A pan-only bind must never authorize persistent zoom.
        if (
          event.isTrusted &&
          text(data.token, 36) &&
          /^[0-9a-f-]{36}$/.test(data.token)
        ) {
          const authority = panAuthority.current;
          if (authority?.sessionId !== session.sessionId) {
            panAuthority.current = {
              sessionId: session.sessionId,
              token: data.token,
              sequence: 0,
              contextId: session.contextId,
              origin: session.location.origin,
            };
            // This document's own announcement has now been answered; it does
            // not vouch for whatever the frame holds next.
            if (documentAnnounced.current === data.token) bridgeVouched.current = false;
            // A load that arrived before this binding can be put to proof now.
            const probe = documentProbe.current;
            if (probe?.kept && probe.nonce === null && probe.sessionId === session.sessionId)
              postDocumentProbe(probe, session.location.origin);
          } else if (authority.token !== data.token) {
            // The bridge binds exactly once per session, before any archive
            // script can know the session. A second, different claim means one
            // of the two was not the bridge: trust neither, so no proof and no
            // gesture is accepted for this session and its next load re-sessions.
            authority.token = "";
          }
        }
        return;
      }
      if (data.type === "reading-pan") {
        const authority = panAuthority.current;
        if (!event.isTrusted || !authority || authority.sessionId !== session.sessionId ||
            data.token !== authority.token || data.url !== session.location.url ||
            typeof data.hidden !== "boolean" || typeof data.sequence !== "number" ||
            !Number.isSafeInteger(data.sequence) || data.sequence <= authority.sequence) return;
        authority.sequence = data.sequence;
        if (!data.hidden) revealReadingControls();
        else if (panCanHide.current && canNavigateReaderStorage() &&
            workspaceRef.current?.getClientRects().length &&
            !document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')) {
          // A touch normally focuses the iframe already. Do not leave an old
          // shell button focused when its toolbar becomes inert.
          if (document.activeElement instanceof Element &&
              document.activeElement.closest(".rw-mobile-bar,.rw-statusbar,.sidebar,.workspace-bar,.status-bar"))
            frameRef.current?.focus({ preventScroll: true });
          setPanHidden(true);
        }
        return;
      }
      if (data.type === "reading-zoom") {
        const authority = panAuthority.current;
        const hello = gestureHello.current;
        if (!event.isTrusted || !authority || authority.sessionId !== session.sessionId ||
            !hello || hello.frameId !== session.frameId ||
            authority.token !== hello.token ||
            data.token !== authority.token || data.url !== session.location.url ||
            (data.direction !== -1 && data.direction !== 1) ||
            typeof data.sequence !== "number" || !Number.isSafeInteger(data.sequence) ||
            data.sequence <= authority.sequence) return;
        authority.sequence = data.sequence;
        if (!alive.current || latest.current.suspended || !latest.current.interactive ||
            session.location.bookId !== tabRef.current.bookId ||
            !latest.current.books.some((book) => book.id === session.location.bookId &&
              book.installationId === session.installationId) ||
            !canNavigateReaderStorage() || !document.hasFocus() || document.hidden ||
            !workspaceRef.current?.getClientRects().length ||
            document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')) return;
        stepZoom(data.direction);
        return;
      }
      if (data.type === "history-result") {
        const pending = pendingHistoryTraversal.current;
        if (pending && data.requestId === pending.requestId && data.accepted === false) {
          if (data.url === pending.session.location.url) reopenHistoryTraversal(pending);
          else clearHistoryTraversal();
        }
        return;
      }
      if (data.type === "export-snapshot") {
        const request = pendingExport.current;
        if (!request || data.requestId !== request.requestId ||
            data.url !== request.session.location.url ||
            session.sessionId !== request.session.sessionId ||
            session.location.url !== request.session.location.url) return;
        pendingExport.current = null;
        const snapshot = data.snapshot;
        if (text(data.error, 4096) && data.error) request.reject(new Error(data.error));
        else if (object(snapshot) && text(snapshot.html, 16 * 1024 * 1024) &&
            snapshot.documentUrl === data.url && typeof snapshot.width === "number" &&
            Number.isInteger(snapshot.width) && snapshot.width >= 1 && snapshot.width <= 16384 &&
            typeof snapshot.height === "number" && Number.isInteger(snapshot.height) &&
            snapshot.height >= 1 && snapshot.height <= 10000000)
          request.resolve({ html: snapshot.html, width: snapshot.width,
            height: snapshot.height, documentUrl: snapshot.documentUrl as string });
        else request.reject(new Error(t("The live page could not be captured.")));
        return;
      }
      if (data.type === "media-checkpoint") {
        if (restoredMediaDocument.current !== `${session.sessionId}\n${data.url}`) return;
        if (data.url !== session.location.url || !session.installationId ||
            latest.current.settings.privateMode || latest.current.settings.readOnly ||
            !validMediaPlaybackPositions(data.positions)) return;
        const article = articlePath(readerDocumentUrl(data.url).persistedUrl, session.location);
        if (article !== null) latest.current.onMediaCheckpoint({
          bookId: session.location.bookId, installationId: session.installationId, article,
          privacyGeneration: session.privacyGeneration,
          privacyPolicyGeneration: latest.current.privacyPolicyGeneration,
        }, data.positions);
        return;
      }
      if (data.type === "media-state") {
        if (
          typeof data.audible !== "boolean" ||
          typeof data.pictureInPicture !== "boolean"
        )
          return;
        frameMedia.current = {
          audible: data.audible,
          pictureInPicture: data.pictureInPicture,
        };
        publishMedia();
        return;
      }
      if (data.type === "speech-text" && text(data.requestId, 128)) {
        nativeSpeech.acceptText(data.requestId, data.text, data.error);
        return;
      }
      // A content frame may dismiss root-owned reader chrome, never invoke
      // application/native actions. Scope remains the active frame and session.
      if (data.type === "dismiss-reader-ui") {
        dismissReaderChrome();
        setLinkContext(null);
        previewSequence.current++;
        setPreview(null);
        return;
      }
      if (
        data.type === "history-travel" &&
        (data.direction === "back" || data.direction === "forward")
      ) {
        travel(data.direction);
        return;
      }
      if (data.type === "link-context" || data.type === "resource-context") {
        if (
          !text(data.url, 16_384) ||
          !text(data.documentUrl, 16_384) ||
          data.documentUrl !== session.location.url ||
          latest.current.suspended ||
          !latest.current.interactive ||
          !canNavigateReaderStorage() ||
          !workspaceRef.current?.getClientRects().length ||
          document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')
        )
          return;
        const path = articlePath(data.url, session.location);
        if (path === null && (data.type === "resource-context" || !validExternal(data.url))) return;
        // The archive supplies only bounded URL hints, never privileged markup.
        // Capture the document identity and revalidate it at the root action.
        previewSequence.current++;
        setPreview(null);
        setLinkContext({
          resource: data.type === "resource-context",
          url: data.url,
          documentUrl: data.documentUrl,
          sessionId: session.sessionId,
          frameId: session.frameId,
          interactionRevision: latest.current.interactionRevision,
          path,
        });
        return;
      }
      if (data.type === "article-preview") {
        if (!latest.current.settings.linkPreviews ||
            latest.current.suspended || !latest.current.interactive ||
            !canNavigateReaderStorage() ||
            !workspaceRef.current?.getClientRects().length ||
            document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]') ||
            !text(data.url, 16_384)) return;
        const path = articlePath(data.url, session.location);
        // A link into the page already on screen has nothing to preview.
        if (!path || path.split("#", 1)[0] === tabRef.current.path.split("#", 1)[0]) return;
        previewQueued.current = {
          session,
          path,
          sequence: ++previewSequence.current,
        };
        drainPreview();
        return;
      }
      if (data.type === "ready" || data.type === "navigation") {
        if (!text(data.url, 16_384)) return;
        const documentUrl = readerDocumentUrl(data.url);
        const path = articlePath(documentUrl.persistedUrl, session.location);
        if (path === null) {
          report(new Error(t("The article attempted to leave its archive.")));
          return;
        }
        const pending = pendingHistoryTraversal.current;
        let traversal: HistoryTraversal | null = null;
        if (data.historyRequestId !== undefined) {
          if (!pending || data.type !== "navigation" || data.historyRequestId !== pending.requestId ||
              !historySourceCurrent(pending) ||
              !canNavigateReaderStorage()) return;
          traversal = pending;
        }
        if (documentUrl.missed) {
          const missing = new Error(
            t("That article is not in this archive. Showing its main page."),
          );
          setNotice(missing.message);
          report(missing, ERROR_SITES["reader.navigation-missing"]);
        }
        setLinkContext(null);
        resetFind();
        const current = tabRef.current;
        const title =
          text(data.title, 4096) && data.title.trim()
            ? data.title
            : current.title;
        if (
          !updateReaderStorageLocation(source, {
            origin: session.location.origin,
            privacyGeneration: session.privacyGeneration,
            url: documentUrl.url,
          })
        )
          return;
        clearHistoryTraversal();
        // Keep the restore address current without assigning iframe.src, which
        // would reload native anchor/history and same-document SPA navigations.
        const updatedSession: FrameSession = {
          ...session,
          location: {
            ...session.location,
            url: documentUrl.url,
            path,
            title,
            ...(data.type === "ready" && text(data.mime, 256) && data.mime
              ? { mime: data.mime }
              : {}),
          },
        };
        activeFrame.current = updatedSession;
        if (frameNavigation.current?.frameId === session.frameId)
          frameNavigation.current.session = updatedSession;
        setFrame(updatedSession);
        const document: PassageDocument = {
          bookId: session.location.bookId,
          sessionId: session.sessionId,
          url: documentUrl.url,
          documentPath: passageDocumentPath(
            path,
            data.headingFragment === true,
          ),
        };
        passageDocumentRef.current = document;
        setPassageDocument(document);
        setPassageSelection(null);
        setSelection("");
        setPassageSelectionUnavailable(false);
        setPassageStatuses({});
        const changed = path !== current.path;
        const previous = currentLocation({
          ...current,
          scroll: scrollPosition.current,
        });
        const next = traversal ? { ...traversal.next, path, title } : {
          ...current,
          path,
          title,
          scroll: changed ? 0 : current.scroll,
          back:
            changed && previous && data.navigationKind !== "replace"
              ? [...current.back, previous]
              : current.back,
          forward:
            changed && data.navigationKind !== "replace" ? [] : current.forward,
        };
        if (!traversal && changed && data.navigationKind === "pop") {
          // Native archive history must move our real stacks, not append a new visit.
          const backIndex = lastLocationIndex(
            current.back,
            current.bookId,
            path,
          );
          const forwardIndex = lastLocationIndex(
            current.forward,
            current.bookId,
            path,
          );
          const direction =
            backIndex >= 0 ? "back" : forwardIndex >= 0 ? "forward" : null;
          if (direction) {
            const stack = current[direction];
            const index = direction === "back" ? backIndex : forwardIndex;
            next[direction] = stack.slice(0, index);
            next[direction === "back" ? "forward" : "back"] = [
              ...current[direction === "back" ? "forward" : "back"],
              ...(previous ? [previous] : []),
              ...stack.slice(index + 1).reverse(),
            ];
          }
        }
        next.back = boundedHistory(next.back);
        next.forward = boundedHistory(next.forward);
        resolvedKey.current = `${next.id}\n${next.bookId ?? ""}\n${next.path}\n${reload}\n${session.privacyGeneration}`;
        publishTab(next);
        if (changed) {
          previewSequence.current++;
          setPreview(null);
          setAttachment(null);
          clearTimeout(scrollTimer.current);
          persistVisit();
          scrollPosition.current = traversal ? traversal.next.scroll : 0;
          setPanel((value) => (value === "bookmark" ? null : value));
          setBookmarkBusy(bookmarkWrite.current);
          setExportBusy(false);
          setExternal(null);
        }
        if (Array.isArray(data.toc))
          setToc(
            data.toc
              .slice(0, 2000)
              .filter(
                (entry): entry is TocEntry =>
                  object(entry) &&
                  text(entry.id, 4096) &&
                  text(entry.title, 4096) &&
                  // An entry without visible text would be an unnamed button.
                  entry.title.trim() !== "" &&
                  typeof entry.level === "number" &&
                  Number.isInteger(entry.level) &&
                  entry.level >= 1 &&
                  entry.level <= 6,
              ),
          );
        const createVisit = historyNavigationPending.current ||
          (changed && (data.type === "navigation" || restoredDocument.current !== ""));
        historyNavigationPending.current = false;
        updateWorkspace({ historyNavigationPending: false });
        if (data.type === "ready") {
          const mediaDocument = `${session.sessionId}\n${data.url}`;
          if (restoredMediaDocument.current !== mediaDocument && session.installationId &&
              !latest.current.settings.privateMode && !latest.current.settings.readOnly) {
            restoredMediaDocument.current = mediaDocument;
            const positions = latest.current.readMediaCheckpoint({
              bookId: session.location.bookId, installationId: session.installationId,
              article: path, privacyGeneration: session.privacyGeneration,
              privacyPolicyGeneration: latest.current.privacyPolicyGeneration,
            });
            if (positions.length) send({ command: "restore-media", requestId: crypto.randomUUID(),
              url: data.url, positions });
          }
          setLoading(false);
          setFrameReady(true);
          setHeldPaint("");
          setProviderUnavailable((previous) => {
            const book = latest.current.books.find((item) => item.id === session.location.bookId);
            return book?.installationId === session.installationId &&
              isProviderUnavailableFor(previous, book) ? null : previous;
          });
          // The frame answered after all: its watchdog band is stale.
          if (watchdogFrame.current === session.frameId) {
            watchdogFrame.current = "";
            setError("");
          }
          setSelection("");
          if (!isMobile) setSpeaking(false);
          if (!isMobile && object(data.capabilities))
            setSpeechAvailable(data.capabilities.speech === true);
          if (!isMobile && Array.isArray(data.voices))
            setVoices(filterSpeechVoices(
              data.voices
                .slice(0, 1000)
                .filter(
                  (voice): voice is Voice =>
                    object(voice) &&
                    text(voice.name, 1024) &&
                    text(voice.lang, 128) &&
                    text(voice.voiceURI, 4096) &&
                    typeof voice.default === "boolean",
                ),
            ));
          send({ command: "zoom", value: settingsRef.current.zoom });
          send({
            command: "appearance",
            theme: settingsRef.current.readerTheme,
            fontFamily: settingsRef.current.fontFamily,
            fontSize: settingsRef.current.fontSize,
            width: settingsRef.current.readerWidth,
          });
          if (latest.current.settings.expandSections)
            send({ command: "expand-sections" });
          if (restoredDocument.current !== data.url) {
            restoredDocument.current = data.url;
            if (!changed && current.scroll > 0)
              send({ command: "scroll", x: 0, y: current.scroll });
          }
        }
        rememberVisit({
          bookId: session.location.bookId,
          path,
          title,
          scroll: scrollPosition.current,
        }, session.privacyGeneration);
        persistVisit(createVisit);
        return;
      }
      if (data.type === "scroll") {
        if (
          typeof data.y !== "number" ||
          !Number.isFinite(data.y) ||
          data.y < 0 ||
          data.y > 100_000_000 ||
          typeof data.x !== "number" ||
          !Number.isFinite(data.x)
        )
          return;
        publishScroll(data.y);
        const current = currentLocation(tabRef.current);
        if (current) rememberVisit(current, session.privacyGeneration);
        clearTimeout(scrollTimer.current);
        scrollTimer.current = setTimeout(() => persistVisit(), 900);
      } else if (data.type === "selection" && text(data.text, MAX_TEXT)) {
        setSelection(data.text);
        const document = passageDocumentRef.current;
        if (
          !document ||
          data.url !== document.url ||
          document.sessionId !== session.sessionId
        ) {
          setPassageSelection(null);
          return;
        }
        setPassageSelectionUnavailable(
          Boolean(data.text) && !passageAnchor(data.anchor),
        );
        setPassageSelection(
          passageAnchor(data.anchor)
            ? {
                exact: data.anchor.exact,
                prefix: data.anchor.prefix,
                suffix: data.anchor.suffix,
                start: data.anchor.start,
                end: data.anchor.end,
                ...document,
              }
            : null,
        );
      } else if (data.type === "highlight-status") {
        const document = passageDocumentRef.current;
        if (
          !document ||
          data.url !== document.url ||
          !Array.isArray(data.statuses) ||
          data.statuses.length > 200
        )
          return;
        const statuses: Record<string, PassageStatus> = {};
        const wire = passageWire.current;
        if (wire.url !== document.url || wire.sessionId !== document.sessionId)
          return;
        for (const entry of data.statuses) {
          if (
            !object(entry) ||
            !text(entry.id, 36) ||
            !HIGHLIGHT_ID.test(entry.id) ||
            !text(entry.bookmarkId, 36) ||
            !HIGHLIGHT_ID.test(entry.bookmarkId) ||
            !["found", "ambiguous", "unavailable"].includes(
              String(entry.status),
            )
          )
            return;
          const bookmarkId = wire.fromWire.get(entry.bookmarkId);
          if (
            bookmarkId === undefined ||
            !wire.permitted.has(`${bookmarkId}:${entry.id}`)
          )
            continue;
          statuses[`${bookmarkId}:${entry.id}`] = entry.status as PassageStatus;
        }
        setPassageStatuses((previous) => ({ ...previous, ...statuses }));
      } else if (
        data.type === "external-link" &&
        text(data.url, 16_384) &&
        typeof data.newTab === "boolean"
      ) {
        const internalPath = articlePath(data.url, session.location);
        if (internalPath !== null) {
          const destination = {
            bookId: session.location.bookId,
            path: internalPath,
            title: t("Article"),
            scroll: 0,
          };
          if (data.newTab) openDestination(destination, true);
          else openDestination(destination, false);
          return;
        }
        if (!validExternal(data.url)) {
          // Nothing failed and nothing needs reloading: say so quietly.
          setNotice(t("This type of external link is not permitted."));
          return;
        }
        setExternal({
          url: data.url,
        });
      } else if (data.type === "unsupported-link") {
        // The bridge blocked a link whose scheme (tel:, sms:, geo:, ...) neither
        // the shell nor the native opener accepts. It carries no address.
        setNotice(t("This type of external link is not permitted."));
      } else if (
        data.type === "find-result" &&
        typeof data.found === "boolean"
      ) {
        const request = pendingFind.current;
        if (!request || latest.current.suspended || !latest.current.interactive ||
            data.requestId !== request.requestId || data.query !== request.query ||
            data.url !== request.url || session.location.url !== request.url ||
            session.sessionId !== request.sessionId || session.frameId !== request.frameId) return;
        pendingFind.current = null;
        setFindFound(data.found);
      } else if (!isMobile && data.type === "voices" && Array.isArray(data.voices)) {
        setVoices(filterSpeechVoices(
          data.voices
            .slice(0, 1000)
            .filter(
              (voice): voice is Voice =>
                object(voice) &&
                text(voice.name, 1024) &&
                text(voice.lang, 128) &&
                text(voice.voiceURI, 4096) &&
                typeof voice.default === "boolean",
            ),
        ));
      } else if (
        !isMobile &&
        data.type === "speech-state" &&
        typeof data.speaking === "boolean"
      ) {
        setSpeaking(data.speaking);
        if (text(data.error, 4096) && data.error) report(new Error(data.error));
      } else if (data.type === "sensor-request") {
        if (
          !text(data.requestId, 128) ||
          !data.requestId ||
          !["camera", "location"].includes(String(data.sensor)) ||
          pendingSensors.current.has(
            sensorKey({
              sessionId: session.sessionId,
              requestId: data.requestId,
            }),
          ) ||
          pendingSensors.current.size >= 16
        )
          return;
        if (data.options !== undefined && !object(data.options)) return;
        const options = object(data.options) ? data.options : {};
        if (
          options.enableHighAccuracy !== undefined &&
          typeof options.enableHighAccuracy !== "boolean"
        )
          return;
        for (const key of ["timeout", "maximumAge"])
          if (
            options[key] !== undefined &&
            (typeof options[key] !== "number" ||
              !Number.isFinite(options[key]) ||
              (options[key] as number) < 0 ||
              (options[key] as number) > 4_294_967_295)
          )
            return;
        const request: SensorRequest = {
          requestId: data.requestId,
          sensor: data.sensor as Sensor,
          sessionId: session.sessionId,
          options: {
            enableHighAccuracy: options.enableHighAccuracy as
              boolean | undefined,
            timeout: options.timeout as number | undefined,
            maximumAge: options.maximumAge as number | undefined,
          },
        };
        pendingSensors.current.set(sensorKey(request), {
          ...request,
          nativeId: nextNativeRequest++,
          cancelled: false,
        });
        if (request.sensor === "location" && !session.status.nativeLocation) {
          // Archives only ever get the app's own location provider (never the
          // webview's), and this build has none. Asking for consent first would
          // end in "unavailable" after the user said yes. `cancelled` ends a
          // watchPosition instead of retrying every second.
          replySensor(request, {
            allowed: false,
            cancelled: true,
            error: { code: 2, message: "Location is unavailable on this platform" },
          });
          pendingSensors.current.delete(sensorKey(request));
          return;
        }
        if (session.status[request.sensor] === "denied") {
          replySensor(request, {
            allowed: false,
            error: { code: 1, message: t("Permission was denied.") },
          });
          pendingSensors.current.delete(sensorKey(request));
          return;
        }
        if (session.status[request.sensor] === "granted") {
          // Re-check the live store before replaying the cached grant.
          void confirmSensorGrant(request);
          return;
        }
        promptSensor(request);
      } else if (data.type === "sensor-cancel" && text(data.requestId, 128)) {
        const pending = pendingSensors.current.get(
          sensorKey({
            sessionId: session.sessionId,
            requestId: data.requestId,
          }),
        );
        if (!pending) return;
        pending.cancelled = true;
        pendingSensors.current.delete(sensorKey(pending));
        void api("sensor_cancel", {
          sessionId: session.sessionId,
          requestId: pending.nativeId,
        }).catch((value) => report(value, ERROR_SITES["reader.sensor-deactivate"]));
        if (consentRef.current?.requestId === data.requestId) {
          consentRef.current = null;
          setConsent(null);
          setConsentBusy(false);
        }
      }
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [
    adjustSpeech,
    clearHistoryTraversal,
    confirmSensorGrant,
    drainPreview,
    dismissReaderChrome,
    nativeSpeech.acceptText,
    historySourceCurrent,
    reopenHistoryTraversal,
    navigate,
    travel,
    persistVisit,
    publishMedia,
    publishScroll,
    publishTab,
    promptSensor,
    reload,
    rememberVisit,
    resetFind,
    revealReadingControls,
    updateWorkspace,
    replySensor,
    report,
    reestablishAuthority,
    openDestination,
    send,
    sendHighlights,
    stepZoom,
    clearDocumentProbe,
    documentReplaced,
    postDocumentProbe,
    resessionUnproven,
    settlePendingInit,
  ]);

  const changeSensorDecision = async (sensor: Sensor, reset = false) => {
    const session = activeFrame.current;
    if (!session || !frameReady) return;
    const source = frameRef.current?.contentWindow;
    // Retire the cached-grant nonce even if the durable handoff later fails.
    // A failed durable handoff quarantines storage until this window is reopened.
    freshOrigin.current = true;
    // Latch the user's explicit choice before the write. If the store rejects
    // it, the latched answer still stops this document from auto-granting.
    const decisionKey = sensorDecisionKey(session.installationId, sensor);
    if (decisionKey)
      sensorDecisions.current.set(decisionKey, reset ? "notAsked" : "denied");
    const current = tabRef.current;
    frameRef.current?.contentWindow?.postMessage(
      {
        channel: "kiwix-reader",
        version: 1,
        type: "sensor-revoke",
        sessionId: session.sessionId,
        sensor,
        decision: reset ? "notAsked" : "denied",
      },
      session.location.origin,
    );
    // Privacy revocation must stop archive media before the document is
    // navigated away; pagehide also pauses, but this covers the handoff window.
    send({ command: "pause-media" });
    latest.current.onMediaState?.({ audible: false, pictureInPicture: false });
    setPermissionsOpen(false);
    setFrameReady(false);
    setLoading(true);
    // Settle the policy decision while the native session still exists. Only a
    // saved decision may retire the document: the store still holds the old
    // decision after a failure, and reloading would replay it as an auto-grant.
    let failure: unknown = null;
    try {
      await lifecycle(() =>
        api(reset ? "sensor_reset" : "sensor_revoke", {
          sessionId: session.sessionId,
          sensor,
        }),
      );
    } catch (value) {
      failure = value;
    }
    const outcome = sensorDecisionWriteOutcome(failure === null);
    if (!outcome.reload) {
      report(failure, ERROR_SITES["reader.sensor-revoke"]);
      // The document was never retired, so keep it readable and show the
      // decision the store actually holds instead of a blocked claim or spinner.
      if (
        alive.current &&
        activeFrame.current?.sessionId === session.sessionId
      ) {
        setLoading(false);
        setFrameReady(true);
      }
      return;
    }
    // Retirement synchronously deactivates shell admission (advancing the open
    // sequence) and navigates the SAME browsing context, destroying sensor-using
    // descendants without dropping the old producer's genuine pagehide writes
    // or its bounded storage credit.
    const retirement = retireReaderDocuments({ restoreOnFailure: false });
    const sequence = openSequence.current;
    let retirementFailure: unknown = null;
    try {
      await retirement;
    } catch (value) {
      retirementFailure = value;
    }
    if (retirementFailure !== null) {
      if (source) quarantineReaderStorage(source);
      const message = t("Kiwix could not confirm that the article finished saving. Reading is paused to protect it. Close and reopen this window to continue.");
      if (alive.current && source && frameRef.current?.contentWindow === source) {
        // Destroy untracked descendant devices even when the proof timed out.
        // The storage host retains the old bounded authority; this is not an ack.
        setFrame(null);
        setFrameReady(false);
        setSpeaking(false);
        setLoading(false);
        setError(message);
      }
      report(new Error(message, { cause: retirementFailure }));
      return;
    }
    if (
      alive.current &&
      openSequence.current === sequence &&
      !activeFrame.current &&
      latest.current.privacyGeneration === session.privacyGeneration &&
      tabRef.current.bookId === current.bookId &&
      tabRef.current.path === current.path
    ) {
      setFrame(null);
      setReload((value) => value + 1);
    }
  };
  const runSearch = async (offset = 0, requestedQuery = query) => {
    if (latest.current.suspended || !alive.current || snippetSaving) return;
    const request = searchRequest.current;
    const privacy = capturePrivacyContext();
    if (!request || !privacy) return;
    const trimmed = requestedQuery.trim();
    if (!trimmed) {
      updateWorkspace({
        searchError: t("Enter a word or phrase to search your books."),
        searchValidation: true,
      });
      return;
    }
    const sequence = ++searchSequence.current;
    const isCurrent = () =>
      alive.current &&
      !latest.current.suspended &&
      sequence === searchSequence.current &&
      isPrivacyContextCurrent(privacy);
    setSearchBusy(true);
    updateWorkspace({ searchError: "", searchValidation: false });
    try {
      const result = await request.run({
        bookIds: searchBooks,
        query: trimmed,
        mode: searchMode,
        offset,
        limit: 50,
      }, privacy);
      if (!isCurrent()) return;
      updateWorkspace({
        searchResults: result,
        searchOffset: offset,
        searchedQuery: trimmed,
      });
    } catch (value) {
      if (isCurrent())
        updateWorkspace({
          searchError: message(value),
          searchValidation: false,
        });
    } finally {
      if (isCurrent()) setSearchBusy(false);
    }
  };
  // Search while typing, without a submit key. The 300 ms delay coalesces
  // keystrokes; an unchanged completed query is not searched twice.
  useEffect(() => {
    if (panel !== "search" && tab.bookId) return;
    const trimmed = query.trim();
    if (!trimmed) return;
    window.clearTimeout(searchDebounce.current);
    searchDebounce.current = window.setTimeout(() => {
      const state = latest.current.workspace;
      if (state.searchedQuery === trimmed && state.searchResults) return;
      if (!latest.current.suspended && alive.current) void runSearch(0, trimmed);
    }, 300);
    return () => window.clearTimeout(searchDebounce.current);
  }, [query, panel, searchBooks, searchMode, tab.bookId]);
  const updateSpellingStatus = useCallback((status: SpellingIndexStatus) => {
    latest.current.onWorkspaceChange((previous) => {
      if (!previous.searchResults) return previous;
      return {
        ...previous,
        searchResults: {
          ...previous.searchResults,
          spellingStatus: (previous.searchResults.spellingStatus ?? []).map(
            (item) => (item.bookId === status.bookId && item.installationId === status.installationId && item.sourceKey === status.sourceKey ? status : item),
          ),
        },
      };
    });
  }, []);
  const spellingAllowed = settings.spellingEnabled && !settings.privateMode && !settings.readOnly;
  const changeSpellingIndex = async (status: SpellingIndexStatus) => {
    if (!spellingAllowed || !status.installationId || !status.sourceKey || !availableBooks.some((book) =>
      book.id === status.bookId && book.installationId === status.installationId)) return;
    const sequence = searchSequence.current;
    setSpellingBusy(status.bookId);
    try {
      const result = await api<SpellingIndexStatus>(
        status.state === "building" ? "spelling_cancel" : "spelling_build",
        { bookId: status.bookId, installationId: status.installationId, sourceKey: status.sourceKey },
      );
      if (alive.current && sequence === searchSequence.current)
        updateSpellingStatus(result);
    } catch (value) {
      if (alive.current && sequence === searchSequence.current) report(value);
    } finally {
      if (alive.current) setSpellingBusy(null);
    }
  };
  const spellingStatuses = searchResults?.spellingStatus ?? [];
  const spellingCurrent =
    (!(searchResults?.corrections?.length) || spellingStatuses.length > 0) &&
    spellingStatuses.every((status) => status.sourceKey && availableBooks.some((entry) =>
      entry.id === status.bookId && entry.installationId === status.installationId
    ));
  const buildingIndexes = spellingAllowed && spellingCurrent
    ? (searchResults?.spellingStatus ?? [])
        .filter((status) => status.installationId && status.state === "building")
        .map((status) => JSON.stringify([status.bookId, status.installationId, status.sourceKey]))
        .join("\n")
    : "";
  useEffect(() => {
    if (!buildingIndexes) return;
    let cancelled = false;
    const sequence = searchSequence.current;
    const timer = setTimeout(() => {
      void Promise.all(
        buildingIndexes.split("\n").map(async (ref) => {
          try {
            const [bookId, installationId, sourceKey]: [string, string, string] = JSON.parse(ref);
            const status = await api<SpellingIndexStatus>("spelling_status", {
              bookId, installationId, sourceKey,
            });
            if (
              !cancelled &&
              alive.current &&
              sequence === searchSequence.current
            )
              updateSpellingStatus(status);
          } catch (value) {
            if (
              !cancelled &&
              alive.current &&
              sequence === searchSequence.current
            )
              updateWorkspace({
                searchError: message(value),
                searchValidation: false,
              });
          }
        }),
      );
    }, 900);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [buildingIndexes, searchResults, updateSpellingStatus, updateWorkspace]);
  const invalidateSearch = () => {
    stopSearch();
    setSearchBusy(false);
    updateWorkspace({
      searchResults: null,
      searchError: "",
      searchValidation: false,
      searchOffset: 0,
    });
  };
  const openHit = (hit: SearchHit, newTab = false) => {
    const destination = {
      bookId: hit.bookId,
      path: hit.path,
      title: hit.title,
      scroll: 0,
    };
    if (
      !latest.current.settings.privateMode &&
      !latest.current.settings.readOnly &&
      latest.current.workspace.searchedQuery
    ) {
      void api("record_search", {
        query: latest.current.workspace.searchedQuery,
        bookId: bookId ?? undefined,
      })
        .then(() => latest.current.onRefresh())
        .catch((value) => report(value, ERROR_SITES["reader.record-search"]));
    }
    if (newTab) openDestination(destination, true);
    else {
      openDestination(destination, false);
      setPanel(null);
    }
  };
  const storeBookmark = async (
    bookmark: BookmarkSave,
  ): Promise<Bookmark | null> => {
    if (bookmarkWrite.current || latest.current.settings.readOnly) return null;
    bookmarkWrite.current = true;
    const privacy = latest.current.privacyGeneration;
    setBookmarkBusy(true);
    try {
      const saved = await api<Bookmark>("save_bookmark", { bookmark });
      latest.current.onRefresh();
      if (!alive.current || privacy !== latest.current.privacyGeneration)
        return null;
      return saved;
    } catch (value) {
      if (alive.current && privacy === latest.current.privacyGeneration)
        report(value);
      return null;
    } finally {
      bookmarkWrite.current = false;
      if (alive.current) setBookmarkBusy(false);
    }
  };
  const saveBookmark = async (event: FormEvent) => {
    event.preventDefault();
    const current = tabRef.current;
    if (!current.bookId || latest.current.settings.readOnly) return;
    const key = bookmarkKey(current);
    const draft = latest.current.workspace.bookmarkDrafts[key];
    const existing = latest.current.bookmarks.find(
      (item) => bookmarkKey(item) === key,
    );
    const saved = await storeBookmark({
      ...(existing ? { id: existing.id } : {}),
      bookId: current.bookId,
      path: current.path,
      title:
        (draft?.title ?? existing?.title ?? current.title).trim() ||
        current.title,
      note: draft?.note ?? existing?.note ?? "",
      tags: draft
        ? draft.tags
            .split(",")
            .map((tag) => tag.trim())
            .filter(Boolean)
        : (existing?.tags ?? []),
    });
    if (!saved) return;
    clearBookmarkDraft(key, draft);
    if (bookmarkKey(tabRef.current) === key) {
      setNotice(t("Saved to your reading list."));
      setPanel(null);
    }
  };
  const saveSelectedPassage = async () => {
    const selected = passageSelection;
    const document = passageDocumentRef.current;
    const session = activeFrame.current;
    const current = tabRef.current;
    const selectedPath =
      selected && session ? articlePath(selected.url, session.location) : null;
    if (
      !selected ||
      !document ||
      !session ||
      !current.bookId ||
      selected.sessionId !== session.sessionId ||
      selected.url !== session.location.url ||
      selected.documentPath !== document.documentPath ||
      selected.bookId !== current.bookId ||
      session.privacyGeneration !== latest.current.privacyGeneration ||
      selectedPath === null ||
      passageDocumentKey(selectedPath) !== passageDocumentKey(current.path) ||
      latest.current.settings.readOnly
    )
      return;
    const existing =
      latest.current.bookmarks.find(
        (item) => bookmarkKey(item) === bookmarkKey(current),
      ) ?? applicablePassages(latest.current.bookmarks, document)[0]?.bookmark;
    const highlight: Highlight = {
      id: crypto.randomUUID(),
      documentPath: selected.documentPath,
      exact: selected.exact,
      prefix: selected.prefix,
      suffix: selected.suffix,
      start: selected.start,
      end: selected.end,
      comment: "",
    };
    const saved = await storeBookmark({
      ...(existing ? { id: existing.id } : { title: current.title }),
      bookId: current.bookId,
      path: existing?.path ?? current.path,
      highlightChange: { type: "add", highlight },
    });
    if (
      saved &&
      passageDocumentRef.current?.url === selected.url &&
      passageDocumentRef.current.sessionId === selected.sessionId
    ) {
      setPassageSelection((previous) =>
        previous === selected ? null : previous,
      );
      setNotice(t("Passage highlighted. You can add a comment below."));
    }
  };
  const changePassage = async (
    bookmarkId: string,
    highlightId: string,
    remove: boolean,
  ) => {
    const document = passageDocumentRef.current;
    const session = activeFrame.current;
    const currentPath =
      document && session ? articlePath(document.url, session.location) : null;
    if (
      !document ||
      !session ||
      session.sessionId !== document.sessionId ||
      session.privacyGeneration !== latest.current.privacyGeneration ||
      document.bookId !== tabRef.current.bookId ||
      currentPath === null ||
      passageDocumentKey(currentPath) !==
        passageDocumentKey(tabRef.current.path) ||
      latest.current.settings.readOnly
    )
      return;
    const entry = applicablePassages(latest.current.bookmarks, document).find(
      ({ bookmark, highlight }) =>
        bookmark.id === bookmarkId && highlight.id === highlightId,
    );
    if (!entry) return;
    const draftKey = `${bookmarkId}:${highlightId}`;
    const comment = commentDrafts[draftKey] ?? entry.highlight.comment;
    const saved = await storeBookmark({
      id: entry.bookmark.id,
      bookId: entry.bookmark.bookId,
      path: entry.bookmark.path,
      highlightChange: remove
        ? { type: "remove", id: highlightId }
        : { type: "comment", id: highlightId, comment },
    });
    if (
      !saved ||
      passageDocumentRef.current?.url !== document.url ||
      passageDocumentRef.current.sessionId !== document.sessionId
    )
      return;
    setCommentDrafts((previous) => {
      if (!remove && previous[draftKey] !== comment) return previous;
      const next = { ...previous };
      delete next[draftKey];
      return next;
    });
    setNotice(remove ? t("Highlight deleted.") : t("Comment saved."));
  };
  const removeBookmark = async () => {
    const current = tabRef.current;
    const key = bookmarkKey(current);
    const existing = latest.current.bookmarks.find(
      (item) => bookmarkKey(item) === key,
    );
    if (!existing || latest.current.settings.readOnly || bookmarkWrite.current)
      return;
    const draft = latest.current.workspace.bookmarkDrafts[key];
    const privacy = capturePrivacyContext();
    if (!privacy || !isPrivacyContextCurrent(privacy)) return;
    bookmarkWrite.current = true;
    setBookmarkBusy(true);
    try {
      await api("remove_bookmark", { id: existing.id, privacyGeneration: privacy.generation });
      if (!isPrivacyContextCurrent(privacy)) return;
      latest.current.onRefresh();
      if (!alive.current)
        return;
      clearBookmarkDraft(key, draft);
      if (bookmarkKey(tabRef.current) === key) {
        setPanel(null);
        setNotice(t("Removed from your reading list."));
      }
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy))
        report(value);
    } finally {
      bookmarkWrite.current = false;
      if (alive.current) setBookmarkBusy(false);
    }
  };
  // `share` and `print` reuse the same PDF render path and hand the result to
  // the system share sheet or the real macOS print panel instead of the save
  // destination.
  const exportArticle = async (
    format: "pdf" | "html" | "mhtml" = "pdf",
    share = false,
    print = false,
  ) => {
    const current = tabRef.current;
    const source = activeFrame.current;
    const privacy = capturePrivacyContext();
    if (!current.bookId || !source || !privacy || exportBusy ||
        latest.current.suspended ||
        source.privacyGeneration !== latest.current.privacyGeneration ||
        source.location.bookId !== current.bookId ||
        source.location.path !== current.path) return;
    const mime = source.location.mime;
    if (format === "mhtml" && !isHtml(mime)) return;
    if (latest.current.settings.readOnly) {
      setNotice(t("Read-only mode prevents saving files."));
      return;
    }
    setExportBusy(true);
    try {
      const book = latest.current.books.find((book) => book.id === current.bookId);
      if (!book || source.installationId !== book.installationId)
        throw new Error(t("The archive changed; reopen the article before exporting."));
      if (format === "pdf" && isHtml(mime)) {
        const title = Array.from(current.title).slice(0, 55).join("");
        const name = exportName(title, "application/pdf")
          .replace(/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?=\.|$)/i, "_$1");
        // The supported ES2022 webviews predate Promise.withResolvers.
        const liveSnapshot = await new Promise<LiveArticleSnapshot>((resolve, reject) => {
          const requestId = crypto.randomUUID();
          const timer = window.setTimeout(() => {
            if (pendingExport.current?.requestId === requestId) pendingExport.current = null;
            reject(new Error(t("The live page could not be captured.")));
          }, 60000);
          pendingExport.current = {
            requestId, session: source,
            resolve: (snapshot) => { clearTimeout(timer); resolve(snapshot); },
            reject: (error) => { clearTimeout(timer); reject(error); },
          };
          send({ command: "export-snapshot", requestId, url: source.location.url });
        });
        if (!isPrivacyContextCurrent(privacy) || latest.current.suspended ||
            activeFrame.current?.sessionId !== source.sessionId ||
            activeFrame.current.location.url !== liveSnapshot.documentUrl)
          throw new Error(t("The article changed before export completed."));
        if (print)
          await printArticlePdf(
            current.bookId,
            book.installationId,
            current.path,
            name,
            liveSnapshot,
          );
        else if (share)
          await shareArticlePdf(
            current.bookId,
            book.installationId,
            current.path,
            name,
            liveSnapshot,
          );
        else
          await saveArticlePdf(
            current.bookId,
            book.installationId,
            current.path,
            name,
            liveSnapshot,
          );
      } else if (format === "mhtml") {
        const stem = exportName(current.title, "text/html").replace(
          /\.html$/,
          "",
        );
        await saveMhtml(current.bookId, book.installationId, current.path, `${stem}.mhtml`);
      } else
        await saveResource(
          current.bookId,
          book.installationId,
          current.path,
          exportName(current.title, mime),
        );
    } catch (value) {
      if (alive.current && isPrivacyContextCurrent(privacy)) report(value);
    } finally {
      if (
        alive.current &&
        tabRef.current.bookId === current.bookId &&
        tabRef.current.path === current.path
      )
        setExportBusy(false);
    }
  };
  // Reopen the article as a genuinely new document. Assigning the live frame
  // the address it already shows is only a same-document navigation whenever
  // that address carries a fragment: nothing is fetched and the old document,
  // with the policy it was served under, stays. So retire the document through
  // its real pagehide and storage handoff, drop the browsing context, and let
  // the open effect build a new one. An unconfirmed handoff removes the frame
  // rather than leave the old document presented as reloaded.
  const reopenFreshDocument = async () => {
    const current = tabRef.current;
    const privacyGeneration = latest.current.privacyGeneration;
    const source = frameRef.current?.contentWindow;
    // Whatever interrupts this still reopens instead of keeping the old key.
    resolvedKey.current = "";
    // A frame that never registered storage is not deactivated by retirement.
    void deactivateFrame().catch(report);
    setFrameReady(false);
    setLoading(true);
    const retirement = retireReaderDocuments({ restoreOnFailure: false });
    const sequence = openSequence.current;
    let retirementFailure: unknown = null;
    try {
      await retirement;
    } catch (value) {
      retirementFailure = value;
    }
    if (retirementFailure !== null) {
      if (source) quarantineReaderStorage(source);
      // The band this message appears in carries "Reload article", which
      // discards the quarantine this just created and reopens the page.
      const message = t("Kiwix could not confirm that this page finished saving, so it was closed. Choose Reload article to open it again. Anything the page had not finished saving is lost.");
      // Still the frame this started with (or still none): never show the
      // unretired document as reloaded, and never leave a spinner behind.
      if (alive.current && frameRef.current?.contentWindow === source) {
        // The storage host retains the old bounded authority; this is not an ack.
        setFrame(null);
        setFrameReady(false);
        setSpeaking(false);
        setLoading(false);
        setError(message);
      }
      report(new Error(message, { cause: retirementFailure }));
      return;
    }
    if (
      alive.current &&
      openSequence.current === sequence &&
      !activeFrame.current &&
      latest.current.privacyGeneration === privacyGeneration &&
      tabRef.current.id === current.id &&
      tabRef.current.bookId === current.bookId &&
      tabRef.current.path === current.path
    ) {
      setFrame(null);
      setReload((value) => value + 1);
    }
  };
  const [restrictedBusy, setRestrictedBusy] = useState(false);
  const restricted = frame?.location.restricted === true;
  const toggleRestricted = async () => {
    const current = tabRef.current;
    if (!current.bookId || !frame || restrictedBusy) return;
    setRestrictedBusy(true);
    try {
      const next = await setReaderRestricted(current.bookId, !restricted);
      publishTab({ ...tabRef.current, restricted: next });
      // The policy is the Content-Security-Policy of a newly served document.
      // The banner follows the frame, so it changes only with that document.
      await reopenFreshDocument();
    } catch (value) {
      if (alive.current) report(value);
    } finally {
      if (alive.current) setRestrictedBusy(false);
    }
  };
  // Desktop printing uses the admitted native export path and system dialog.
  // Android's PDF export is its system print dialog. A mobile webview does not
  // implement window.print() for the archive frame, so only the browser-only
  // preview keeps that behavior; iOS has no print path and says so.
  const printArticle = () => {
    if (!isNative) {
      send({ command: "print" });
      return;
    }
    const source = activeFrame.current;
    if (!source || latest.current.settings.readOnly ||
        !isHtml(source.location.mime)) return;
    if (canPrintArticlePdf) void exportArticle("pdf", false, true);
    else if (isAndroid) void exportArticle("pdf");
    else setNotice(t("Printing is not available on this device. Save or share the article as a PDF instead."));
  };
  const openAttachment = async () => {
    const target = attachment;
    if (
      !target ||
      latest.current.settings.readOnly ||
      attachmentBusy ||
      tabRef.current.bookId !== target.bookId ||
      tabRef.current.path !== target.path
    )
      return;
    setAttachmentBusy(true);
    setAttachmentError("");
    try {
      await api("open_attachment", {
        bookId: target.bookId,
        path: target.path,
        confirmed: true,
      });
      if (alive.current)
        setAttachment((current) => (current === target ? null : current));
    } catch (value) {
      if (alive.current) setAttachmentError(message(value));
    } finally {
      if (alive.current) setAttachmentBusy(false);
    }
  };
  const openExternal = async () => {
    const request = external;
    if (!request || latest.current.settings.externalLinks === "deny" ||
        externalBusy || !validExternal(request.url)) return;
    setExternalBusy(true);
    try {
      await api("open_external", { url: request.url, confirmed: true });
      if (alive.current)
        setExternal((current) => (current === request ? null : current));
    } catch (value) {
      report(value);
    } finally {
      if (alive.current) setExternalBusy(false);
    }
  };
  const toggleFullscreen = useCallback(async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await workspaceRef.current?.requestFullscreen();
    } catch (value) {
      report(value, ERROR_SITES["reader.fullscreen"]);
    }
  }, [report]);
  useEffect(() => {
    const changed = () =>
      setFullscreen(document.fullscreenElement === workspaceRef.current);
    document.addEventListener("fullscreenchange", changed);
    return () => document.removeEventListener("fullscreenchange", changed);
  }, []);
  const openFind = useCallback(() => {
    const ownerTab = tabRef.current.id;
    const revision = latest.current.interactionRevision;
    if (!findBarRef.current?.contains(document.activeElement)) {
      const target = document.activeElement;
      findInvoker.current =
        target instanceof HTMLElement && workspaceRef.current?.contains(target) &&
        !panelRef.current?.contains(target) ? target : null;
    }
    setFindOpen(true);
    requestAnimationFrame(() => {
      if (!alive.current || latest.current.suspended || !latest.current.interactive ||
          tabRef.current.id !== ownerTab || latest.current.interactionRevision !== revision ||
          !workspaceRef.current?.getClientRects().length ||
          workspaceRef.current.closest("[inert], [hidden]") ||
          document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')) return;
      findRef.current?.focus();
      findRef.current?.select();
    });
  }, []);
  const find = (backwards = false) => {
    const session = activeFrame.current;
    if (!findQuery || !session || !frameReady || latest.current.suspended ||
        !latest.current.interactive || session.privacyGeneration !== latest.current.privacyGeneration) return;
    const request = {
      requestId: crypto.randomUUID(), query: findQuery, url: session.location.url,
      sessionId: session.sessionId, frameId: session.frameId,
    };
    pendingFind.current = request;
    setFindFound(null);
    send({ command: "find", query: request.query, requestId: request.requestId,
      url: request.url, backwards });
  };
  const openSearch = useCallback(() => {
    const ownerTab = tabRef.current.id;
    const revision = latest.current.interactionRevision;
    if (tabRef.current.bookId && !latest.current.workspace.searchScopeInitialized) {
      setSearchBooks([tabRef.current.bookId]);
    }
    setPanel("search");
    requestAnimationFrame(() => {
      if (!alive.current || latest.current.suspended || !latest.current.interactive ||
          tabRef.current.id !== ownerTab || latest.current.interactionRevision !== revision ||
          latest.current.workspace.panel !== "search" ||
          !workspaceRef.current?.getClientRects().length ||
          workspaceRef.current.closest("[inert], [hidden]") ||
          document.querySelector('dialog[open], [role="dialog"], .app-shell[hidden]')) return;
      searchRef.current?.focus();
    });
  }, [setPanel]);
  // Shortcut handlers read the newest render's closures through a ref, so the
  // listeners below subscribe once instead of after every scroll-driven render.
  const shortcutAction = (name: string) => {
    if (latest.current.suspended || !workspaceRef.current?.getClientRects().length ||
        document.querySelector(".app-shell[hidden], dialog[open]")) return;
    if (name === "speech-slower" || name === "speech-faster") {
      if (alive.current && document.hasFocus() && activeFrame.current &&
          activeFrame.current.privacyGeneration === latest.current.privacyGeneration &&
          !document.querySelector('[role="dialog"],dialog[open],.app-shell[hidden]'))
        adjustSpeech(name === "speech-slower" ? -1 : 1);
      return;
    }
    if (name === "search") openSearch();
    else if (name === "home") {
      const archive = latest.current.books.find((book) =>
        book.id === tabRef.current.bookId && book.available);
      if (archive) navigate({
        bookId: archive.id, path: archive.mainPath, title: archive.title, scroll: 0,
      });
    }
    else if (name === "random" && tabRef.current.bookId) void randomArticle();
    else if (name === "toc")
      setPanel((value) => (value === "contents" ? null : "contents"));
    else if (name === "find" && tabRef.current.bookId) openFind();
    else if (
      name === "print" &&
      activeFrame.current &&
      !latest.current.settings.readOnly
    )
      printArticle();
    else if (name === "save" && activeFrame.current) void exportArticle();
    else if (name === "bookmark" && activeFrame.current) setPanel("bookmark");
    else if (name === "zoom-reset") updateZoom(null);
    else if (name === "zoom-in" || name === "zoom-out")
      stepZoom(name === "zoom-out" ? -1 : 1);
    else if (name === "back" || name === "forward") travel(name);
    else if (name === "fullscreen") void toggleFullscreen();
    else if (name === "escape") {
      dismissReaderChrome();
      previewSequence.current++;
      setPreview(null);
    }
  };
  const shortcutKeyboard = (event: KeyboardEvent) => {
    if (!event.isTrusted) return;
    const modifier = event.metaKey || event.ctrlKey;
    let name = "";
    if (!modifier && !event.altKey && !event.isComposing && ["<", ">"].includes(event.key)) {
      const editor = event.composedPath().some((node) => node instanceof Element &&
        node.closest("input,textarea,select,[contenteditable]:not([contenteditable='false']),[role='textbox']"));
      if (editor || !alive.current || !activeFrame.current || !tabRef.current.bookId ||
          activeFrame.current.privacyGeneration !== latest.current.privacyGeneration ||
          latest.current.suspended || !document.hasFocus() ||
          !workspaceRef.current?.getClientRects().length ||
          document.querySelector('[role="dialog"],dialog[open],.app-shell[hidden]')) return;
      event.preventDefault();
      adjustSpeech(event.key === "<" ? -1 : 1);
      return;
    }
    if (modifier) {
      const key = event.key.toLowerCase();
      name =
        key === "k"
          ? "search"
          : key === "f"
            ? "find"
            : key === "p"
              ? "print"
              : key === "s"
                ? "save"
                : key === "d"
                  ? "bookmark"
                  : ["+", "="].includes(key)
                    ? "zoom-in"
                    : key === "-"
                      ? "zoom-out"
                      : key === "0"
                        ? "zoom-reset"
                        : "";
    } else if (event.altKey && event.key === "ArrowLeft") name = "back";
    else if (event.altKey && event.key === "ArrowRight") name = "forward";
    else if (event.altKey && event.key === "Home") name = "home";
    else if (event.key === "F11") name = "fullscreen";
    else if (event.key === "Escape") name = "escape";
    if (isDesktop && name !== "escape") return;
    if (name && !document.querySelector("dialog[open]")) {
      event.preventDefault();
      shortcutAction(name);
    }
  };
  const shortcutMouseHistory = (event: MouseEvent) => {
    if (
      !event.isTrusted ||
      ![3, 4].includes(event.button) ||
      document.querySelector("dialog[open]")
    )
      return;
    event.preventDefault();
    travel(event.button === 3 ? "back" : "forward");
  };
  const shortcutSuppressMouseHistory = (event: MouseEvent) => {
    if (event.isTrusted && [3, 4].includes(event.button))
      event.preventDefault();
  };
  const shortcutHandlers = useRef({
    action: shortcutAction,
    keyboard: shortcutKeyboard,
    mouseHistory: shortcutMouseHistory,
    suppressMouseHistory: shortcutSuppressMouseHistory,
  });
  shortcutHandlers.current = {
    action: shortcutAction,
    keyboard: shortcutKeyboard,
    mouseHistory: shortcutMouseHistory,
    suppressMouseHistory: shortcutSuppressMouseHistory,
  };
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) =>
      shortcutHandlers.current.keyboard(event);
    const mouseHistory = (event: MouseEvent) =>
      shortcutHandlers.current.mouseHistory(event);
    const suppressMouseHistory = (event: MouseEvent) =>
      shortcutHandlers.current.suppressMouseHistory(event);
    const action = (name: string) => shortcutHandlers.current.action(name);
    window.addEventListener("keydown", keyboard);
    window.addEventListener("mouseup", mouseHistory);
    window.addEventListener("mousedown", suppressMouseHistory);
    window.addEventListener("auxclick", suppressMouseHistory);
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    if (isDesktop)
      void import("@tauri-apps/api/webviewWindow")
        .then(({ getCurrentWebviewWindow }) =>
          getCurrentWebviewWindow().listen<unknown>(
            "kiwix-reader-shortcut",
            (event) => {
              if (
                typeof event.payload === "string" &&
                [
                  "search",
                  "find",
                  "print",
                  "save",
                  "bookmark",
                  "zoom-in",
                  "zoom-out",
                  "zoom-reset",
                  "back",
                  "forward",
                  "random",
                  "home",
                  "toc",
                  "fullscreen",
                  "speech-slower",
                  "speech-faster",
                ].includes(event.payload)
              )
                action(event.payload);
            },
          ),
        )
        .then((dispose) => {
          if (cancelled) dispose();
          else unlisten = dispose;
        })
        .catch((value) => report(value, ERROR_SITES["reader.shortcut-listener"]));
    if (isMobile) latest.current.onShortcutHandler?.(action);
    return () => {
      cancelled = true;
      if (isMobile) latest.current.onShortcutHandler?.(null);
      unlisten?.();
      window.removeEventListener("keydown", keyboard);
      window.removeEventListener("mouseup", mouseHistory);
      window.removeEventListener("mousedown", suppressMouseHistory);
      window.removeEventListener("auxclick", suppressMouseHistory);
    };
  }, [report]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 5000);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    if (!frame || frameReady || props.suspended || retainedActivation.current ||
        activeFrame.current?.frameId !== frame.frameId) return;
    const frameId = frame.frameId;
    const timer = setTimeout(() => {
      if (
        !alive.current ||
        activeFrame.current?.frameId !== frameId ||
        frameRef.current?.dataset.frameId !== frameId
      )
        return;
      setLoading(false);
      // A document that never authenticated is not left on screen.
      setHeldPaint("");
      watchdogFrame.current = frameId;
      report(
        new Error(
          t(
            "The archive did not respond to the secure reader connection. Reload the article to try again.",
          ),
        ),
      );
    }, 20_000);
    return () => clearTimeout(timer);
  }, [frame, frameReady, props.suspended, report]);

  const searchStatus = searchStatusMessage({
    busy: searchBusy,
    resultCount: searchResults ? searchResults.hits.length : null,
    total: searchResults ? searchResults.total : null,
    offset: searchOffset,
    error: searchError,
    validation: searchValidation,
    hasMore: searchResults?.hasMore ?? false,
  });
  const searchStatusText = (() => {
    if (!searchStatus) return "";
    switch (searchStatus.kind) {
      case "searching":
        return t("Searching…");
      case "validation":
        return t("Enter a word or phrase to search your books.");
      case "failed":
        return t("Search failed");
      case "no-results":
        return t("No results");
      case "page":
        return (
          t("{start}–{end} of {total}", searchStatus.values) +
          (searchStatus.hasMore ? " " + t("More results are available.") : "")
        );
      case "results":
        return (
          t("About {count} results", {
            count: searchStatus.values.count,
          }) +
          (searchStatus.hasMore ? " " + t("More results are available.") : "")
        );
    }
  })();
  // The panel is rendered in the search aside, and inline on the empty page.
  // Anywhere else the whole tree is dead weight on every render.
  const scopedBook = searchBooks.length === 1 && searchBooks[0] === tab.bookId
    ? books.find((book) => book.id === tab.bookId)?.title || tab.bookId
    : null;
  const searchContent =
    panel === "search" || (!tab.bookId && availableBooks.length > 0) ? (
    <div className="rw-search-content">
      <div className="rw-visually-hidden" role="status" aria-live="polite">
        {searchStatusText}
      </div>
      <form
        className="rw-search-form"
        role="search"
        aria-label={t("Search your offline library")}
        onSubmit={(event) => {
          event.preventDefault();
          if (!searchBusy) void runSearch();
        }}
      >
        <label htmlFor={`reader-search-${tab.id}`}>
          {searchBooks.length === 1 && searchBooks[0] === tab.bookId
            ? t("Search this book")
            : t("Search your offline library")}
        </label>
        <div className="rw-search-input">
          <Search size={18} aria-hidden="true" />
          <input
            id={`reader-search-${tab.id}`}
            ref={searchRef}
            value={query}
            onChange={(event) => {
              updateWorkspace({ query: event.target.value });
              invalidateSearch();
            }}
            placeholder={t("Search books")}
            maxLength={1000}
          />
          {searchBusy ? (
            <button type="button" onClick={invalidateSearch}>
              {t("Cancel")}
            </button>
          ) : (
            <button type="submit" disabled={snippetSaving || !availableBooks.length}>
              {t("Search")}
            </button>
          )}
        </div>
        {tab.bookId && (
          <button
            type="button"
            onClick={() => {
              const bookId = tab.bookId;
              if (!bookId) return;
              setSearchBooks(
                searchBooks.length === 1 && searchBooks[0] === bookId
                  ? []
                  : [bookId],
              );
              invalidateSearch();
            }}
          >
            {searchBooks.length === 1 && searchBooks[0] === tab.bookId
              ? t("Search all books")
              : t("Search this book")}
          </button>
        )}
        <details className="rw-secondary-controls rw-search-settings">
          <summary>
            <span>{t("Search options")}</span>
            <span className="rw-settings-summary">
              {searchMode === "fulltext" ? t("Full text") : t("Article titles")}
              {" · "}
              {searchBooks.length
                ? t("{count} selected books", { count: searchBooks.length })
                : t("All available books")}
            </span>
          </summary>
          <div className="rw-search-options">
            <label>
              {t("Search mode")}
              <select
                value={searchMode}
                onChange={(event) => {
                  updateWorkspace({
                    searchMode: event.target.value as "title" | "fulltext",
                  });
                  invalidateSearch();
                }}
              >
                <option value="title">{t("Article titles")}</option>
                <option value="fulltext">{t("Full text")}</option>
              </select>
            </label>
            <label className="rw-snippet-option">
              <input
                type="checkbox"
                checked={settings.showSearchSnippets}
                disabled={settings.readOnly || snippetSaving}
                onChange={(event) => void changeSearchSnippets(event.target.checked)}
              />
              <span>
                {t("Show search snippets")}
                <small>{t("Changes apply to your next search.")}</small>
              </span>
            </label>
            <details>
              <summary>
                {searchBooks.length
                  ? t("{count} selected books", { count: searchBooks.length })
                  : t("All available books")}
                <ChevronDown size={14} aria-hidden="true" />
              </summary>
              <div className="rw-book-checks">
                <button
                  type="button"
                  onClick={() => {
                    setSearchBooks([]);
                    invalidateSearch();
                  }}
                >
                  {t("Search all books")}
                </button>
                {availableBooks.map((item) => (
                  <label key={item.id}>
                    <input
                      type="checkbox"
                      checked={searchBooks.includes(item.id)}
                      onChange={(event) => {
                        setSearchBooks((current) =>
                          event.target.checked
                            ? [...current, item.id]
                            : current.filter((id) => id !== item.id),
                        );
                        invalidateSearch();
                      }}
                    />
                    <span>
                      {item.title}
                      {searchMode === "fulltext" && !item.hasFulltextIndex && (
                        <small>{t("No full-text index")}</small>
                      )}
                    </span>
                  </label>
                ))}
              </div>
            </details>
          </div>
        </details>
      </form>
      {searchError && (
        // The search live region announces failures; this copy stays visual
        // so a failure is not announced twice.
        <p className="rw-inline-error">{errorText(searchError)}</p>
      )}
      {searchResults && (
        <section
          className="rw-search-results"
          aria-label={t("Search results")}
          aria-busy={searchBusy}
        >
          {scopedBook && (
            <p className="rw-search-scope">{t("Searching {book}", { book: scopedBook })}</p>
          )}
          <div className="rw-result-count">
            {t("About {count} results", { count: searchResults.total })}
          </div>
          {searchResults.errors.map((item) => (
            <p key={item.bookId} className="rw-inline-error">
              {t("{book}: {error}", {
                book:
                  books.find((candidate) => candidate.id === item.bookId)
                    ?.title || item.bookId,
                error: errorText(item.error),
              })}
            </p>
          ))}
          {!searchResults.hits.length && (
            <p className="rw-muted">
              {t("No articles found. Try another word or search more books.")}
            </p>
          )}
          {spellingAllowed && !searchResults.hits.length && !spellingCurrent && (
            <button type="button" className="rw-text-button" onClick={() => void runSearch(0)}>
              {t("Search articles")}
            </button>
          )}
          {spellingAllowed && !searchResults.hits.length && spellingCurrent && (
            <section
              className="rw-spelling"
              aria-label={t("Spelling suggestions")}
            >
              {(searchResults.corrections ?? []).map((correction) => (
                <button
                  className="rw-text-button"
                  type="button"
                  key={correction.correctedQuery}
                  disabled={searchBusy}
                  onClick={() => {
                    updateWorkspace({ query: correction.correctedQuery });
                    void runSearch(0, correction.correctedQuery);
                  }}
                >
                  {t("Search instead for “{query}”", {
                    query: correction.correctedQuery,
                  })}
                </button>
              ))}
              {(searchResults.spellingStatus ?? []).map((status) => (
                <div className="rw-spelling-index" key={status.bookId}>
                  <strong>
                    {books.find((item) => item.id === status.bookId)?.title ??
                      status.bookId}
                  </strong>
                  <span role="status">
                    {status.state === "building"
                      ? t("Building spelling index: {processed} / {total}", {
                          processed: formatNumber(status.processed),
                          total: formatNumber(status.total),
                        })
                      : status.state === "rebuild"
                        ? t("Rebuild the spelling index for this version. Your previous index is kept.")
                      : status.state === "ready"
                        ? t("Spelling index ready")
                        : status.state === "error"
                          ? errorText(status.error ?? "")
                          : t("Spelling index not built")}
                  </span>
                  {status.state !== "ready" &&
                    (status.state !== "building" ||
                      !("cancellable" in status) ||
                      status.cancellable !== false) && (
                      <button
                        type="button"
                        className="rw-text-button"
                        disabled={
                          settings.privateMode ||
                          settings.readOnly ||
                          spellingBusy !== null
                        }
                        onClick={() => void changeSpellingIndex(status)}
                      >
                        {status.state === "building"
                          ? t("Cancel indexing")
                          : t("Build spelling index")}
                      </button>
                    )}
                </div>
              ))}
            </section>
          )}
          {searchResults.hits.map((hit, index) => (
            <article
              className="rw-result"
              key={`${hit.bookId}:${hit.path}:${index}`}
            >
              <div>
                <button
                  className="rw-result-title"
                  type="button"
                  onClick={(event) =>
                    openHit(hit, event.metaKey || event.ctrlKey)
                  }
                >
                  {hit.title}
                </button>
                {!scopedBook && (
                  <span className="rw-result-book">
                    {books.find((item) => item.id === hit.bookId)?.title ||
                      hit.bookId}
                  </span>
                )}
                {hit.snippet && <p>{hit.snippet}</p>}
              </div>
              <IconButton
                label={t("Open {title} in a new tab", { title: hit.title })}
                onClick={() => openHit(hit, true)}
              >
                <ArrowUpRight size={17} />
              </IconButton>
            </article>
          ))}
          {(searchOffset > 0 || searchResults.hasMore) && (
            <nav
              className="rw-pagination"
              aria-label={t("Search result pages")}
            >
              <button
                type="button"
                disabled={searchOffset === 0 || searchBusy}
                onClick={() => void runSearch(Math.max(0, searchOffset - 50))}
              >
                <ChevronLeft size={16} />
                {t("Previous")}
              </button>
              <span>
                {t("{start}–{end} of {total}", {
                  start: formatNumber(searchOffset + 1),
                  end: formatNumber(
                    Math.min(
                      searchOffset + searchResults.hits.length,
                      searchResults.total,
                    ),
                  ),
                  total: formatNumber(searchResults.total),
                })}
              </span>
              <button
                type="button"
                disabled={!searchResults.hasMore || searchBusy}
                onClick={() => void runSearch(searchOffset + 50)}
              >
                {t("Next")}
                <ChevronRight size={16} />
              </button>
            </nav>
          )}
        </section>
      )}
      {!settings.privateMode && recentSearches.length > 0 && (
        <section
          className="rw-recent-searches"
          aria-label={t("Recent searches")}
        >
          <h3>{t("Recent searches")}</h3>
          <ul>
            {recentSearches.slice(0, 6).map((recent) => (
              <li key={recent}>
                <button
                  type="button"
                  onClick={() => {
                    updateWorkspace({ query: recent });
                    void runSearch(0, recent);
                  }}
                >
                  {recent}
                </button>
                <IconButton
                  label={t("Remove recent search: {query}", { query: recent })}
                  disabled={settings.readOnly}
                  onClick={() => {
                    void api("remove_search", { query: recent, bookId: bookId ?? undefined })
                      .then(() => latest.current.onRefresh())
                      .catch((value) =>
                        report(value, ERROR_SITES["reader.remove-search"]),
                      );
                  }}
                >
                  <X size={13} />
                </IconButton>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
    ) : null;

  const readerControls = (
      <div
        className="rw-toolbar"
        role="toolbar"
        aria-label={t("Reader navigation")}
      >
        <div className="rw-navigation">
          <IconButton
            label={t("Back")}
            disabled={!tab.back.length}
            onClick={() => travel("back")}
          >
            <ArrowLeft size={18} />
          </IconButton>
          <HistoryDisclosure label={t("Back history")}>
            <div role="group" aria-label={t("Back history")}>
              {!tab.back.length && <span>{t("No previous pages")}</span>}
              {tab.back
                .slice()
                .reverse()
                .map((item, index) => (
                  <button
                    type="button"
                    key={`${item.bookId}:${item.path}:${index}`}
                    title={item.path}
                    onClick={(event) => {
                      event.currentTarget
                        .closest("details")
                        ?.removeAttribute("open");
                      travel("back", index + 1);
                    }}
                  >
                    {item.title || item.path}
                  </button>
                ))}
            </div>
          </HistoryDisclosure>
          <IconButton
            label={t("Forward")}
            disabled={!tab.forward.length}
            onClick={() => travel("forward")}
          >
            <ArrowRight size={18} />
          </IconButton>
          <HistoryDisclosure label={t("Forward history")}>
            <div role="group" aria-label={t("Forward history")}>
              {!tab.forward.length && <span>{t("No next pages")}</span>}
              {tab.forward
                .slice()
                .reverse()
                .map((item, index) => (
                  <button
                    type="button"
                    key={`${item.bookId}:${item.path}:${index}`}
                    title={item.path}
                    onClick={(event) => {
                      event.currentTarget
                        .closest("details")
                        ?.removeAttribute("open");
                      travel("forward", index + 1);
                    }}
                  >
                    {item.title || item.path}
                  </button>
                ))}
            </div>
          </HistoryDisclosure>
          <IconButton
            label={t("Archive home")}
            disabled={!book?.available}
            onClick={() =>
              book &&
              navigate({
                bookId: book.id,
                path: book.mainPath,
                title: book.title,
                scroll: 0,
              })
            }
          >
            <Home size={18} />
          </IconButton>
          <IconButton
            label={t("Random article")}
            disabled={!book?.available}
            onClick={() => void randomArticle()}
          >
            <Shuffle size={17} />
          </IconButton>
        </div>
        <div className="rw-document-identity">
          {tab.bookId ? (
            <>
              <span className="rw-document-book">
                {book?.title || t("Archive")}
              </span>
              <strong title={tab.title}>
                {tab.title || t("Opening article…")}
              </strong>
            </>
          ) : (
            <strong>{t("New tab")}</strong>
          )}
        </div>
        <div className="rw-toolbar-actions">
          <IconButton
            label={t("Search books")}
            data-reader-control="search"
            active={panel === "search"}
            onClick={panel === "search" ? dismissPanel : openSearch}
          >
            <Search size={18} />
          </IconButton>
          <IconButton
            label={t("Table of contents")}
            data-reader-control="contents"
            active={panel === "contents"}
            disabled={!frameReady || toc.length === 0}
            onClick={() => setPanel(panel === "contents" ? null : "contents")}
          >
            <List size={19} />
          </IconButton>
          <IconButton
            label={t("Find in article")}
            data-reader-control="find"
            active={findOpen}
            disabled={!frameReady}
            onClick={openFind}
          >
            <FileText size={18} />
          </IconButton>
          <IconButton
            label={
              savedBookmark
                ? t("Edit bookmark and notes")
                : t("Bookmark and add notes")
            }
            active={Boolean(savedBookmark)}
            data-reader-control="bookmark"
            disabled={!frameReady}
            onClick={() => setPanel("bookmark")}
          >
            <BookmarkIcon
              size={18}
              fill={savedBookmark ? "currentColor" : "none"}
            />
          </IconButton>
          {isAndroid &&
            tab.bookId &&
            book?.available &&
            !settings.privateMode &&
            !settings.readOnly && (
              <IconButton
                label={t("Pin this article to your home screen")}
                disabled={!frameReady || !tab.path}
                onClick={() => void pinCurrentArticle()}
              >
                <Pin size={18} />
              </IconButton>
            )}
          <IconButton
            label={
              restricted
                ? t("Leave restricted mode")
                : t("Open in restricted mode")
            }
            active={restricted}
            disabled={!frameReady || restrictedBusy}
            onClick={() => void toggleRestricted()}
          >
            <ShieldAlert size={18} />
          </IconButton>
          <IconButton
            label={t("Reading appearance")}
            data-reader-control="appearance"
            active={panel === "appearance"}
            onClick={() =>
              setPanel(panel === "appearance" ? null : "appearance")
            }
          >
            <Type size={18} />
          </IconButton>
          <IconButton
            label={readingSpeech
              ? nativeReading
                ? nativeSpeech.paused ? t("Resume reading") : t("Pause reading")
                : t("Stop reading")
              : t("Start reading aloud")}
            data-reader-control="speech-quick"
            active={readingSpeech}
            disabled={!(speechAvailable || neuralSelected || nativeReading) || !frameReady}
            onClick={toggleReading}
          >
            {readingSpeech
              ? nativeReading
                ? nativeSpeech.paused ? <Play size={18} /> : <Pause size={18} />
                : <Square size={18} />
              : <Play size={18} />}
          </IconButton>
          <IconButton
            label={t("Read aloud")}
            data-reader-control="speech"
            active={panel === "speech"}
            disabled={!frameReady}
            onClick={() => setPanel(panel === "speech" ? null : "speech")}
          >
            <Volume2 size={18} />
          </IconButton>
          {frame && !isHtml(frame.location.mime) && (
            <IconButton
              label={t("Open attachment in another application")}
              disabled={!frameReady || settings.readOnly}
              onClick={(event) => {
                if (!event.isTrusted || !tabRef.current.bookId) return;
                setAttachmentError("");
                setAttachment({
                  ...frame.location,
                  ...currentLocation(tabRef.current)!,
                });
              }}
            >
              <ExternalLink size={18} />
            </IconButton>
          )}
          <IconButton
            label={fullscreen ? t("Exit fullscreen") : t("Fullscreen")}
            onClick={() => void toggleFullscreen()}
          >
            {fullscreen ? <Minimize size={18} /> : <Maximize size={18} />}
          </IconButton>
        </div>
      </div>
  );

  return (
    <div
      ref={workspaceRef}
      className={`reader-workspace${tab.bookId ? "" : " is-empty"}${readingControlsHidden ? " reading-controls-hidden" : ""}`}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || !(event.target instanceof Element))
          return;
        const menu = event.target.closest<HTMLDetailsElement>(
          "details.rw-history-menu",
        );
        if (!menu?.open) return;
        event.preventDefault();
        event.stopPropagation();
        menu.open = false;
        menu.querySelector<HTMLElement>("summary")?.focus();
      }}
    >
      {compact ? (
        <div className="rw-mobile-bar" role="toolbar" aria-label={t("Reader navigation")}
          inert={readingControlsHidden} aria-hidden={readingControlsHidden || undefined}>
          <IconButton
            label={tab.back.length ? t("Back") : t("Open library")}
            disabled={!tab.back.length && !props.onOpenLibrary}
            onClick={() => tab.back.length ? travel("back") : props.onOpenLibrary?.()}
          >
            {tab.back.length ? <ArrowLeft size={18} /> : <LibraryIcon size={18} />}
          </IconButton>
          <div className="rw-mobile-identity">
            <div>
              <strong title={tab.title} dir="auto">
                {tab.bookId ? tab.title || t("Opening article…") : t("New tab")}
              </strong>
              {settings.privateMode && (
                <span role="img" aria-label={t("Private reading")} title={t("Private reading")}>
                  <ShieldCheck size={15} aria-hidden="true" />
                </span>
              )}
              {settings.readOnly && (
                <span role="img" aria-label={t("Read-only")} title={t("Read-only")}>
                  <LockKeyhole size={15} aria-hidden="true" />
                </span>
              )}
            </div>
          </div>
          <IconButton
            label={readingSpeech
              ? nativeReading
                ? nativeSpeech.paused ? t("Resume reading") : t("Pause reading")
                : t("Stop reading")
              : t("Start reading aloud")}
            data-reader-control="speech-quick"
            active={readingSpeech}
            disabled={!(speechAvailable || neuralSelected || nativeReading) || !frameReady}
            onClick={toggleReading}
          >
            {readingSpeech
              ? nativeReading
                ? nativeSpeech.paused ? <Play size={19} /> : <Pause size={19} />
                : <Square size={19} />
              : <Play size={19} />}
          </IconButton>
          <IconButton label={t("Search books")} data-reader-control="search" active={panel === "search"} onClick={panel === "search" ? dismissPanel : openSearch}>
            <Search size={19} />
          </IconButton>
          {props.mobileTabs && (
            <IconButton
              label={t("Open reader tabs")}
              aria-expanded={props.mobileTabs.expanded}
              aria-controls="reader-tabs"
              data-mobile-tabs-trigger="true"
              onClick={async () => {
                try {
                  // The shell switcher is outside the reader fullscreen subtree.
                  if (document.fullscreenElement) await document.exitFullscreen();
                  if (alive.current) latest.current.mobileTabs?.onToggle();
                } catch (value) {
                  report(value, ERROR_SITES["reader.fullscreen"]);
                }
              }}
            >
              <span className="rw-mobile-tab-count">{formatNumber(props.mobileTabs.count)}</span>
            </IconButton>
          )}
          <IconButton
            label={t("Reader navigation")}
            data-reader-control="navigation"
            aria-haspopup="dialog"
            aria-expanded={controlsOpen}
            onClick={() => setControlsOpen(true)}
          >
            <Ellipsis size={21} />
          </IconButton>
        </div>
      ) : readerControls}
      {readingControlsHidden && (
        <button type="button" className="rw-reveal-controls" onClick={revealReadingControls}>
          <ChevronDown size={18} aria-hidden="true" />
          {t("Reader navigation")}
        </button>
      )}
      {compact && controlsOpen && !consent && !external && !attachment && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={t("Reader navigation")}
          className="rw-mobile-controls"
          dismissOnBackdrop
          onClose={() => setControlsOpen(false)}
        >
          <div
            onClick={(event) => {
              const button = event.target instanceof Element
                ? event.target.closest<HTMLButtonElement>("button")
                : null;
              if (button && !button.disabled) setControlsOpen(false);
            }}
          >
            {readerControls}
            <div className="rw-mobile-overflow-actions">
              <IconButton label={t("Archive permissions")} disabled={!frame} onClick={() => setPermissionsOpen(true)}>
                <ShieldCheck size={18} />
              </IconButton>
              <p className="rw-muted"><ShieldCheck size={14} aria-hidden="true" /> {t("Offline archive")}</p>
              {settings.privateMode && <p className="rw-muted">{t("Private reading")}</p>}
              {settings.readOnly && <p className="rw-muted">{t("Read-only")}</p>}
              <IconButton label={t("Shortcuts")} onClick={() => setShortcuts(true)}>
                <CircleHelp size={18} />
              </IconButton>
            </div>
          </div>
        </Dialog>
      )}
      {findOpen && (
        <form
          ref={findBarRef}
          className="rw-findbar"
          role="search"
          aria-label={t("Find in article")}
          onSubmit={(event) => {
            event.preventDefault();
            find();
          }}
        >
          <Search size={16} aria-hidden="true" />
          <input
            ref={findRef}
            value={findQuery}
            onChange={(event) => {
              setFindQuery(event.target.value);
              resetFind();
            }}
            placeholder={t("Find in this article…")}
            aria-label={t("Find in this article")}
            maxLength={2000}
            onKeyDown={(event) => {
              if (event.key === "Enter" && event.shiftKey) {
                event.preventDefault();
                find(true);
              }
            }}
          />
          <span role="status">
            {findFound === false
              ? t("No matches")
              : findFound === true
                ? t("Match found")
                : ""}
          </span>
          <IconButton
            label={t("Previous match")}
            disabled={!findQuery}
            onClick={() => find(true)}
          >
            <ChevronLeft size={17} />
          </IconButton>
          <IconButton
            label={t("Next match")}
            disabled={!findQuery}
            onClick={() => find()}
          >
            <ChevronRight size={17} />
          </IconButton>
          <IconButton
            label={t("Close find")}
            onClick={dismissFind}
          >
            <X size={17} />
          </IconButton>
        </form>
      )}
      {(error || providerUnavailable) && (
        <div className="rw-error" role={error ? "alert" : "status"}>
          <span>{errorText(error || providerUnavailable)}</span>
          <button
            type="button"
            onClick={() => {
              discardQuarantinedReaderStorage();
              leaveDocument();
              void reopenFreshDocument();
            }}
          >
            {t("Reload article")}
          </button>
          <IconButton label={t("Dismiss error")} onClick={() => { setError(""); setProviderUnavailable(null); }}>
            <X size={16} />
          </IconButton>
        </div>
      )}
      {restricted && (
        <div className="rw-error rw-restricted" role="status">
          <span>
            {t(
              "Restricted mode blocks archive scripts, forms, workers and remote network requests. Some pages may not work.",
            )}
          </span>
          <button
            type="button"
            disabled={restrictedBusy}
            onClick={() => void toggleRestricted()}
          >
            {t("Leave restricted mode")}
          </button>
        </div>
      )}
      {book?.tags.some((tag) => tag.trim().toLowerCase() === "_sw:yes") && (
        <div className="rw-error" role="status">
          <span>
            {t("This archive requires legacy Service Worker replay, which this reader does not support. Open it with another compatible reader.")}
          </span>
        </div>
      )}
      {exportBusy && (
        <div className="rw-export-progress" role="status">
          <Download size={16} aria-hidden="true" />
          <span>{t("Saving…")}</span>
        </div>
      )}
      <div className="rw-body">
        {panel && (
          <aside
            ref={panelRef}
            className={`rw-panel rw-panel-${panel}`}
            aria-label={
              panel === "search"
                ? t("Search books")
                : panel === "contents"
                  ? t("Table of contents")
                  : panel === "appearance"
                    ? t("Reading appearance")
                    : panel === "speech"
                      ? t("Read aloud")
                      : t("Bookmark and notes")
            }
          >
            <header>
              <h2>
                {panel === "search"
                  ? t("Search books")
                  : panel === "contents"
                    ? t("On this page")
                    : panel === "appearance"
                      ? t("Reading appearance")
                      : panel === "speech"
                        ? t("Read aloud")
                        : t("Bookmark and notes")}
              </h2>
              <IconButton
                label={t("Close panel")}
                onClick={dismissPanel}
              >
                <X size={18} />
              </IconButton>
            </header>
            {panel === "search" && searchContent}
            {panel === "contents" && (
              <nav className="rw-toc" aria-label={t("Article sections")}>
                {toc.length ? (
                  toc.map((entry, index) => (
                    <button
                      type="button"
                      key={`${entry.id}:${index}`}
                      style={{
                        paddingInlineStart: `${1 + (entry.level - 1) * 0.7}rem`,
                      }}
                      onClick={() => send({ command: "toc", id: entry.id })}
                    >
                      {entry.title}
                    </button>
                  ))
                ) : (
                  <p className="rw-muted">
                    {t("This article has no section headings.")}
                  </p>
                )}
              </nav>
            )}
            {panel === "appearance" && (
              <div className="rw-panel-content">
                {settings.readOnly && (
                  <p className="rw-footnote">
                    {t(
                      "Read-only mode is enabled. Appearance changes last for this tab only; file exports are disabled.",
                    )}
                  </p>
                )}
                <fieldset>
                  <legend>{t("Page theme")}</legend>
                  <div className="rw-theme-options">
                    {(["original", "light", "sepia", "dark"] as const).map(
                      (theme) => (
                        <button
                          className={`rw-theme-${theme}`}
                          type="button"
                          aria-pressed={readerSettings.readerTheme === theme}
                          key={theme}
                          onClick={() => updateSettings({ readerTheme: theme })}
                        >
                          {theme === "original"
                            ? t("Original")
                            : theme === "light"
                              ? t("Light")
                              : theme === "sepia"
                                ? t("Paper")
                                : t("Dark")}
                          {readerSettings.readerTheme === theme && (
                            <Check size={14} aria-hidden="true" />
                          )}
                        </button>
                      ),
                    )}
                  </div>
                </fieldset>
                <label className="rw-field">
                  {t("Typeface")}
                  <select
                    value={readerSettings.fontFamily}
                    onChange={(event) =>
                      updateSettings({
                        fontFamily: event.target
                          .value as Settings["fontFamily"],
                      })
                    }
                  >
                    <option value="original">{t("Archive original")}</option>
                    <option value="serif">{t("Serif")}</option>
                    <option value="sans">{t("Sans serif")}</option>
                  </select>
                </label>
                <label className="rw-field">
                  {t("Text size: {size}", {
                    size: formatNumber(readerSettings.fontSize),
                  })}
                  <input
                    type="range"
                    min="10"
                    max="48"
                    step="1"
                    value={readerSettings.fontSize}
                    onChange={(event) =>
                      updateSettings({ fontSize: Number(event.target.value) })
                    }
                  />
                </label>
                <label className="rw-field">
                  {t("Reading width")}
                  <select
                    value={readerSettings.readerWidth}
                    onChange={(event) =>
                      updateSettings({
                        readerWidth: event.target
                          .value as Settings["readerWidth"],
                      })
                    }
                  >
                    <option value="original">{t("Original")}</option>
                    <option value="comfortable">{t("Comfortable")}</option>
                    <option value="full">{t("Full width")}</option>
                  </select>
                </label>
                <div className="rw-zoom-control">
                  <span>{t("Page zoom")}</span>
                  <IconButton
                    label={t("Zoom out")}
                    disabled={!tab.bookId || readerSettings.zoom <= 0.25}
                    onClick={() => stepZoom(-1)}
                  >
                    <Minus size={16} />
                  </IconButton>
                  <button
                    type="button"
                    title={t("Use default zoom")}
                    disabled={!tab.bookId}
                    onClick={() => updateZoom(null)}
                  >
                    {t("{percent}%", {
                      percent: formatNumber(
                        Math.round(readerSettings.zoom * 100),
                      ),
                    })}
                  </button>
                  <IconButton
                    label={t("Zoom in")}
                    disabled={!tab.bookId || readerSettings.zoom >= 5}
                    onClick={() => stepZoom(1)}
                  >
                    <Plus size={16} />
                  </IconButton>
                </div>
                <details className="rw-secondary-controls">
                  <summary>{t("Zoom for this book")}</summary>
                  <div className="rw-secondary-content">
                    <label className="rw-field">
                      {t("Zoom for this book")}
                      <input
                        type="range"
                        min="0.25"
                        max="5"
                        step="0.05"
                        value={readerSettings.zoom}
                        disabled={!tab.bookId}
                        onChange={(event) => updateZoom(Number(event.target.value))}
                      />
                    </label>
                    <p className="rw-muted">
                      {settings.readOnly
                        ? t(
                            "Zoom changes apply only to this tab in read-only mode.",
                          )
                        : t(
                            "Zoom is remembered for this book. Reset to use the global default.",
                          )}
                    </p>
                  </div>
                </details>
                <details className="rw-secondary-controls">
                  <summary>{t("Print and export")}</summary>
                  <div className="rw-secondary-content">
                    {/* Mobile has no separate print path: Android's print
                        dialog is the PDF row below, and iOS has none. */}
                    {!isMobile && (
                    <button
                      className="rw-action-row"
                      type="button"
                      disabled={!frameReady || settings.readOnly ||
                        (canPrintArticlePdf &&
                          (exportBusy || !frame || !isHtml(frame.location.mime)))}
                      onClick={() => {
                        if (!latest.current.settings.readOnly) printArticle();
                      }}
                    >
                      <Printer size={18} />
                      <span>
                        {t("Print / save as PDF")}
                        <small>
                          {t("Choose a printer or PDF in the system dialog.")}
                        </small>
                      </span>
                    </button>
                    )}
                    <button
                      className="rw-action-row"
                      type="button"
                      disabled={!frameReady || exportBusy || settings.readOnly ||
                        !frame || !isHtml(frame.location.mime)}
                      onClick={(event) => {
                        if (event.isTrusted) void exportArticle("pdf");
                      }}
                    >
                      <FileText size={18} />
                      <span>
                        {exportBusy ? t("Saving…") : isMobile && !isIOS
                          ? t("Print / save PDF") : t("Save article as PDF")}
                        <small>
                          {isMobile && !isIOS
                            ? t("Choose Save as PDF in the Android system print dialog.")
                            : t("Save a paginated PDF using the native file picker.")}
                        </small>
                        <small>{t("Captures the current page, including expanded sections and entered text, without executable scripts or network access.")}</small>
                      </span>
                    </button>
                    {canShareArticlePdf && (
                      <button
                        className="rw-action-row"
                        type="button"
                        disabled={
                          !frameReady ||
                          exportBusy ||
                          settings.readOnly ||
                          !frame ||
                          !isHtml(frame.location.mime)
                        }
                        onClick={(event) => {
                          if (event.isTrusted) void exportArticle("pdf", true);
                        }}
                      >
                        <Share2 size={18} />
                        <span>
                          {t("Share")}
                          <small>{t("Shares a PDF of the current page, including expanded sections and entered text.")}</small>
                          {isAndroid && (
                            <small>{t("Android shares screen-layout page images. Links and accessible text may not be preserved; print styles and page-break rules are not applied.")}</small>
                          )}
                        </span>
                      </button>
                    )}
                    <button
                      className="rw-action-row"
                      type="button"
                      disabled={!frameReady || exportBusy || settings.readOnly}
                      onClick={() => void exportArticle("html")}
                    >
                      <Download size={18} />
                      <span>
                        {exportBusy
                          ? t("Saving…")
                          : frame && isHtml(frame.location.mime)
                            ? t("Save original HTML")
                            : t("Save original file")}
                        <small>{t("Export the original file, unchanged.")}</small>
                      </span>
                    </button>
                    <button
                      className="rw-action-row"
                      type="button"
                      disabled={
                        !frameReady ||
                        exportBusy ||
                        settings.readOnly ||
                        !frame ||
                        !isHtml(frame.location.mime)
                      }
                      onClick={() => void exportArticle("mhtml")}
                    >
                      <FileText size={18} />
                      <span>
                        {t("Save article as MHTML")}
                        <small>
                          {t(
                            "Bundle the article and its static archive resources in one file.",
                          )}
                        </small>
                      </span>
                    </button>
                  </div>
                </details>
                <button
                  className="rw-action-row"
                  type="button"
                  onClick={() => setShortcuts(true)}
                >
                  <CircleHelp size={18} />
                  <span>{t("Keyboard shortcuts")}</span>
                </button>
              </div>
            )}
            {panel === "speech" && (
              <div className="rw-panel-content">
                <KokoroDownloadPrompt storageKey="kiwix-kokoro-read-aloud-v1"
                  enabled={!settings.privateMode && !settings.readOnly} />
                <p className="rw-muted">
                  {selection
                    ? t("Your selected passage is ready to read aloud.")
                    : t(
                        "Listen to this article, or select a passage to read just that part.",
                      )}
                </p>
                {!speechAvailable && (
                  <p className="rw-inline-error" role="status">
                    {isMobile
                      ? t("Offline read-aloud is unavailable. Check installed voices in system settings.")
                      : t("Speech synthesis is not available in this system webview.")}
                    {speechAvailabilityError && settings.showTechnicalErrors && (
                      <small><br />{speechAvailabilityError}</small>
                    )}
                  </p>
                )}
                <button
                  className="rw-primary rw-wide"
                  type="button"
                  disabled={!(speechAvailable || neuralSelected || nativeReading) || !frameReady}
                  onClick={toggleReading}
                >
                  {readingSpeech
                    ? nativeReading
                      ? nativeSpeech.paused ? <Play size={18} /> : <Pause size={18} />
                      : <Square size={18} />
                    : <Play size={18} />}
                  {readingSpeech
                    ? nativeReading
                      ? nativeSpeech.paused ? t("Resume reading") : t("Pause reading")
                      : t("Stop reading")
                    : selection
                      ? t("Read selection")
                      : t("Read article")}
                </button>
                {nativeReading && (
                  <p className="rw-muted" role="status">
                    {nativeSpeech.paused
                      ? t("Read-aloud is paused.")
                      : nativeSpeech.pending
                        ? t("Preparing read-aloud…")
                        : effectiveVoice
                          ? t("Reading aloud with {name}.", { name: effectiveVoice.name })
                          : t("Reading aloud.")}
                  </p>
                )}
                {nativeReading && (
                  <div className="rw-speech-transport">
                    <button
                      type="button"
                      onClick={() => {
                        if (nativeSpeech.paused) void nativeSpeech.resume();
                        else void nativeSpeech.pause();
                      }}
                    >
                      {nativeSpeech.paused ? <Play size={16} /> : <Pause size={16} />}
                      {nativeSpeech.paused ? t("Resume reading") : t("Pause reading")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void nativeSpeech.previous()}
                    >
                      <ChevronLeft size={16} />
                      {t("Previous paragraph")}
                    </button>
                    {/* iOS system voices support only previous; Kokoro owns all transports. */}
                    {(isAndroid || isDesktop || nativeSpeech.kokoroActive) && (
                      <>
                        <button type="button" onClick={() => void nativeSpeech.next()}>
                          <ChevronRight size={16} />{t("Next paragraph")}
                        </button>
                        <button type="button" onClick={() => void nativeSpeech.rewind(15)}>
                          {t("Back 15 seconds")}
                        </button>
                        <button type="button" onClick={() => void nativeSpeech.forward(15)}>
                          {t("Forward 15 seconds")}
                        </button>
                      </>
                    )}
                    <button type="button" onClick={stopReading}>
                      <Square size={16} />
                      {t("Stop reading")}
                    </button>
                  </div>
                )}
                <details className="rw-secondary-controls">
                  <summary>
                    <span>{t("Voice settings")}</span>
                    <span className="rw-settings-summary">{languageName(speechLanguage)}</span>
                  </summary>
                  <div className="rw-secondary-content">
                    <label className="rw-field">
                      {t("Reading language")}
                      <select value={speechLanguage} onChange={(event) => {
                        const language = event.target.value;
                        updateWorkspace({ speechLanguages: { ...workspace.speechLanguages, [bookId ?? ""]: language } });
                        updateSpeech({}, language);
                      }}>
                        {[...new Set([...archiveLanguages, speechLanguage, ...voices.map((voice) => normalizeLocale(voice.lang)).filter((language): language is string => Boolean(language))])].map((language) =>
                          <option key={language} value={language}>{languageName(language)}</option>
                        )}
                      </select>
                    </label>
                    <label className="rw-field">
                      {t("Voice")}
                      <select
                        value={effectiveVoice?.voiceURI ?? ""}
                        onChange={(event) =>
                          updateSpeech({ voice: event.target.value })
                        }
                      >
                        <option value="">{t("Automatic voice for {language}", { language: languageName(speechLanguage) })}</option>
                        {speech.voice && (!voices.some((voice) => voice.voiceURI === speech.voice) ||
                          (speech.voice.startsWith("kokoro:") && !speechLanguage.toLowerCase().startsWith("en"))) && (
                          <option value={speech.voice}>{t("Saved voice is unavailable")}</option>
                        )}
                        {matchingVoices.length > 0 && <optgroup label={t("Matching language")}>
                          {matchingVoices.map((voice) => (
                            <option key={voice.voiceURI} value={voice.voiceURI}>
                              {t("{name} · {language}", { name: voice.name, language: languageName(voice.lang) })}
                            </option>
                          ))}
                        </optgroup>}
                        <optgroup label={t("Other installed voices")}>
                          {otherVoices.map((voice) => (
                            <option key={voice.voiceURI} value={voice.voiceURI}>
                              {t("{name} · {language}", { name: voice.name, language: languageName(voice.lang) })}
                            </option>
                          ))}
                        </optgroup>
                      </select>
                    </label>
                    {effectiveVoice ? <p className="rw-muted" role="status">
                      {t("Using {name} · {language}", { name: effectiveVoice.name, language: languageName(effectiveVoice.lang) })}
                      {matchingVoices.length === 0 && " " + t("No matching voice is installed; an available voice will be used.")}
                    </p> : speechAvailable && <p className="rw-muted" role="status">
                      {t("No installed voices were reported. The operating system will try its default voice.")}
                    </p>}
                    <label className="rw-field">
                      {t("Reading speed: {rate}×", {
                        rate: formatNumber(speech.rate),
                      })}
                      <input
                        type="range"
                        min="0.25"
                        max="2"
                        step="0.05"
                        value={speech.rate}
                        onChange={(event) =>
                          updateSpeech({ rate: Number(event.target.value) })
                        }
                      />
                    </label>
                    <p className="rw-footnote">
                      {voices.some((voice) => voice.voiceURI.startsWith("kokoro:"))
                        ? t("Downloaded natural voices work offline. System voices may require an installed language pack.")
                        : t("Voices are provided by your operating system. Some voices may require an installed language pack.")}
                    </p>
                  </div>
                </details>
              </div>
            )}
            {panel === "bookmark" && (
              <form
                className="rw-panel-content"
                onSubmit={(event) => void saveBookmark(event)}
              >
                <p className="rw-muted">
                  {settings.readOnly
                    ? t(
                        "Read-only mode is enabled. Your saved bookmark and notes cannot be changed.",
                      )
                    : t("Save the page, and the ideas you want to return to.")}
                </p>
                <label className="rw-field">
                  {t("Title")}
                  <input
                    value={bookmarkTitle}
                    onChange={(event) =>
                      editBookmark({ title: event.target.value })
                    }
                    maxLength={4096}
                    readOnly={settings.readOnly}
                    required
                  />
                </label>
                <label className="rw-field">
                  {t("Your notes")}
                  <textarea
                    value={bookmarkNote}
                    onChange={(event) =>
                      editBookmark({ note: event.target.value })
                    }
                    placeholder={t("What caught your attention?")}
                    rows={8}
                    maxLength={100_000}
                    readOnly={settings.readOnly}
                  />
                </label>
                {selection && !settings.readOnly && (
                  <button
                    className="rw-text-button"
                    type="button"
                    onClick={() =>
                      editBookmark({
                        note: [bookmarkNote, selection]
                          .filter(Boolean)
                          .join("\n\n"),
                      })
                    }
                  >
                    {t("Add selected passage to notes")}
                  </button>
                )}
                <label className="rw-field">
                  {t("Tags")}
                  <input
                    value={bookmarkTags}
                    onChange={(event) =>
                      editBookmark({ tags: event.target.value })
                    }
                    placeholder={t("Separate tags with commas")}
                    maxLength={4096}
                    readOnly={settings.readOnly}
                  />
                </label>
                <button
                  className="rw-primary rw-wide"
                  type="submit"
                  disabled={bookmarkBusy || settings.readOnly}
                >
                  <BookmarkIcon size={17} />
                  {bookmarkBusy ? t("Saving…") : t("Save to reading list")}
                </button>
                {savedBookmark && (
                  <button
                    className="rw-text-button rw-danger"
                    type="button"
                    disabled={bookmarkBusy || settings.readOnly}
                    onClick={() => void removeBookmark()}
                  >
                    {t("Remove bookmark")}
                  </button>
                )}
                <section
                  className="rw-passages"
                  aria-label={t("Highlights and comments")}
                >
                  <div className="rw-section-heading">
                    <h3>{t("Highlights and comments")}</h3>
                    <span className="rw-passage-count">
                      {formatNumber(
                        Math.min(passages.length, MAX_DOCUMENT_HIGHLIGHTS),
                      )}
                    </span>
                  </div>
                  <p className="rw-muted">
                    {t(
                      "Select a passage in the article, then save it here. Highlights and comments stay with your reading list.",
                    )}
                  </p>
                  {passageSelection && (
                    <blockquote className="rw-passage-quote" dir="auto">
                      {passageSelection.exact}
                    </blockquote>
                  )}
                  <button
                    className="rw-primary rw-wide"
                    type="button"
                    disabled={
                      !passageSelection ||
                      bookmarkBusy ||
                      settings.readOnly ||
                      !frameReady
                    }
                    onClick={() => void saveSelectedPassage()}
                  >
                    {t("Highlight selected passage")}
                  </button>
                  {passageSelectionUnavailable && (
                    <p className="rw-footnote" role="status">
                      {t(
                        "This selection cannot be anchored. Select up to 16,384 characters of article text.",
                      )}
                    </p>
                  )}
                  {passages.length > MAX_DOCUMENT_HIGHLIGHTS && (
                    <p className="rw-footnote" role="status">
                      {t(
                        "Only the first {count} highlights can be displayed on this page.",
                        {
                          count: MAX_DOCUMENT_HIGHLIGHTS,
                        },
                      )}
                    </p>
                  )}
                  {!passages.length && (
                    <p className="rw-footnote">
                      {t("No highlighted passages on this page yet.")}
                    </p>
                  )}
                  <ol className="rw-passage-list">
                    {passages
                      .slice(0, MAX_DOCUMENT_HIGHLIGHTS)
                      .map(({ bookmark, highlight }) => {
                        const key = `${bookmark.id}:${highlight.id}`;
                        const status = passageStatuses[key];
                        const comment = commentDrafts[key] ?? highlight.comment;
                        return (
                          <li className="rw-passage-card" key={key}>
                            <blockquote className="rw-passage-quote" dir="auto">
                              {highlight.exact}
                            </blockquote>
                            {status !== "found" && (
                              <p
                                className="rw-footnote rw-passage-status"
                                role="status"
                              >
                                {status === "ambiguous"
                                  ? t(
                                      "Several passages match. This highlight was not placed.",
                                    )
                                  : status === "unavailable"
                                    ? t(
                                        "This passage is unavailable or the article has changed.",
                                      )
                                    : t("Locating passage…")}
                              </p>
                            )}
                            <button
                              className="rw-text-button"
                              type="button"
                              disabled={status !== "found" || !frameReady}
                              onClick={() => {
                                const token = passageWire.current.toWire.get(
                                  bookmark.id,
                                );
                                if (!token) return;
                                send({
                                  command: "highlight-jump",
                                  id: highlight.id,
                                  bookmarkId: token,
                                  url: passageDocument?.url ?? "",
                                });
                              }}
                            >
                              {t("Jump to passage")}
                            </button>
                            <label className="rw-field">
                              {t("Passage comment")}
                              <textarea
                                value={comment}
                                dir="auto"
                                rows={3}
                                maxLength={16_384}
                                readOnly={settings.readOnly}
                                placeholder={t(
                                  "What would you like to remember about this passage?",
                                )}
                                onChange={(event) =>
                                  setCommentDrafts((previous) => ({
                                    ...previous,
                                    [key]: event.target.value,
                                  }))
                                }
                              />
                            </label>
                            <div className="rw-passage-actions">
                              <button
                                className="rw-text-button"
                                type="button"
                                disabled={
                                  bookmarkBusy ||
                                  settings.readOnly ||
                                  comment === highlight.comment
                                }
                                onClick={() =>
                                  void changePassage(
                                    bookmark.id,
                                    highlight.id,
                                    false,
                                  )
                                }
                              >
                                {t("Save comment")}
                              </button>
                              <button
                                className="rw-text-button rw-danger"
                                type="button"
                                disabled={bookmarkBusy || settings.readOnly}
                                onClick={() =>
                                  void changePassage(
                                    bookmark.id,
                                    highlight.id,
                                    true,
                                  )
                                }
                              >
                                {t("Delete highlight")}
                              </button>
                            </div>
                          </li>
                        );
                      })}
                  </ol>
                </section>
              </form>
            )}
          </aside>
        )}
        <section
          className="rw-page"
          aria-label={t("Article content")}
          aria-busy={loading}
        >
          {!tab.bookId && (
            <div className="rw-empty-page">
              <header className="rw-empty-heading">
                <h1>{t("New tab")}</h1>
                {props.onOpenLibrary && (
                  <button
                    className="rw-primary"
                    type="button"
                    onClick={props.onOpenLibrary}
                  >
                    <BookOpen size={17} />
                    {t("Open library")}
                  </button>
                )}
              </header>
              {!availableBooks.length && (
                <p className="rw-muted">
                  {t("Import an archive or download one from Discover to read offline.")}
                </p>
              )}
              {panel !== "search" && availableBooks.length > 0 && searchContent}
              {availableBookmarks.length > 0 && (
                <section className="rw-quick-bookmarks">
                  <div className="rw-section-heading">
                    <h2>{t("Reading list")}</h2>
                    <BookmarkIcon size={17} aria-hidden="true" />
                  </div>
                  {availableBookmarks
                    .slice(0, 6)
                    .map((item) => (
                      <button
                        type="button"
                        key={item.id}
                        onClick={() =>
                          navigate({
                            bookId: item.bookId,
                            path: item.path,
                            title: item.title,
                            scroll: 0,
                          })
                        }
                      >
                        <span>{item.title}</span>
                        <ArrowRight size={16} aria-hidden="true" />
                      </button>
                    ))}
                </section>
              )}
              {availableBooks.length > 0 && (
                <section
                  className="rw-quick-books"
                  aria-label={t("Available books")}
                >
                  <div className="rw-section-heading">
                    <h2>{t("Available books")}</h2>
                    <span>
                      {t("{count} available", { count: availableBooks.length })}
                    </span>
                  </div>
                  {availableBooks.map((item) => (
                    <button
                      className="rw-book-card"
                      key={item.id}
                      type="button"
                      onClick={() =>
                        navigate({
                          bookId: item.id,
                          path: item.mainPath,
                          title: item.title,
                          scroll: 0,
                        })
                      }
                    >
                      <span>
                        <strong>{item.title}</strong>
                        <small>
                          {languageName(item.language)}
                          {item.articleCount
                            ? t(" · {count} articles", {
                                count: item.articleCount,
                              })
                            : ""}
                        </small>
                      </span>
                      <ArrowUpRight size={20} aria-hidden="true" />
                    </button>
                  ))}
                </section>
              )}
            </div>
          )}
          {tab.bookId && !frame && (
            <div className="rw-loading-state" role="status">
              <BookOpen size={38} strokeWidth={1.2} aria-hidden="true" />
              <h2>
                {loading
                  ? t("Opening article…")
                  : t("This page could not be opened.")}
              </h2>
              <p>
                {loading
                  ? book?.title
                  : t(
                      "The error above contains details. You can reload the article or choose another book.",
                    )}
              </p>
            </div>
          )}
          {frame && (
            <iframe
              key={frame.contextId}
              ref={bindFrame}
              className={`rw-archive-frame${frameReady || heldPaint === frame.frameId ? "" : " is-pending"}`}
              title={t("Article: {title}", {
                title: tab.title || book?.title || t("Archive"),
              })}
              sandbox={`allow-scripts allow-same-origin allow-forms${settings.readOnly ? "" : " allow-modals allow-downloads"}`}
              // Element fullscreen and Picture-in-Picture are delegated only to
              // this trusted archive origin, so the shell and other origins stay
              // denied, and no script/navigation/sensor policy changes.
              allow={`camera ${frame.location.origin}; geolocation 'none'; microphone 'none'; fullscreen ${frame.location.origin}; picture-in-picture ${frame.location.origin}`}
              referrerPolicy="no-referrer"
              onLoad={(event) => {
                const node = event.currentTarget;
                const active = activeFrame.current;
                if (
                  !active ||
                  !canNavigateReaderStorage() ||
                  active.privacyGeneration !==
                    latest.current.privacyGeneration ||
                  // about:blank loads inherit the shell; they are not articles.
                  node.contentDocument !== null ||
                  active.frameId !== frame.frameId ||
                  frameRef.current !== node ||
                  frameNavigation.current?.node !== node ||
                  frameNavigation.current.frameId !== active.frameId
                )
                  return;
                resetFind();
                if (initializedFrame.current !== frame.frameId) {
                  initializedFrame.current = frame.frameId;
                  admitLoadedDocument(node, active);
                  return;
                }
                // Still waiting for a bridge to vouch: nothing was handed out,
                // so there is nothing to prove or retire.
                if (pendingInit.current === active.sessionId) return;
                // A later load of an initialised frame is a child navigation
                // when another document has announced itself. Otherwise the
                // session survives solely on its own bridge's proof.
                if (documentReplaced(active)) {
                  clearDocumentProbe();
                  resessionFrame(node, active, frameReady);
                } else probeDocument(node, active, frameReady);
              }}
              data-frame-id={frame.frameId}
            />
          )}
          {preview && (
            <section
              className="rw-article-preview"
              aria-label={t("Article preview")}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  previewSequence.current++;
                  setPreview(null);
                  frameRef.current?.focus();
                }
              }}
            >
              <header>
                <span>{t("Article preview")}</span>
                <IconButton
                  label={t("Close preview")}
                  onClick={() => {
                    previewSequence.current++;
                    setPreview(null);
                  }}
                >
                  <X size={15} />
                </IconButton>
              </header>
              <div aria-live="polite">
                <h2>{preview.article?.title || t("Loading article…")}</h2>
                {preview.article?.thumbnail && (
                  <img
                    className="rw-preview-thumbnail"
                    src={preview.article.thumbnail}
                    alt=""
                    decoding="async"
                    onError={() => {
                      const thumbnail = preview.article?.thumbnail;
                      setPreview((current) =>
                        current?.article?.thumbnail === thumbnail && current?.article
                          ? { ...current, article: { ...current.article, thumbnail: null } }
                          : current,
                      );
                    }}
                  />
                )}
                {preview.article && (
                  <p>
                    {preview.article.excerpt || t("No preview text available.")}
                  </p>
                )}
              </div>
              <footer>
                <button
                  type="button"
                  className="rw-primary"
                  onClick={() =>
                    navigate({
                      bookId: preview.bookId,
                      path: preview.requestedPath,
                      title: preview.article?.title || t("Article"),
                      scroll: 0,
                    })
                  }
                >
                  {t("Open article")}
                </button>
                <button
                  type="button"
                  className="rw-text-button"
                  onClick={() => {
                    openDestination({
                      bookId: preview.bookId,
                      path: preview.requestedPath,
                      title: preview.article?.title || t("Article"),
                      scroll: 0,
                    }, true);
                    previewSequence.current++;
                    setPreview(null);
                  }}
                >
                  {t("Open in new tab")}
                </button>
              </footer>
            </section>
          )}
          {frame && loading && (
            <div className="rw-loading-line" role="status">
              <span>{t("Loading article…")}</span>
            </div>
          )}
        </section>
      </div>
      <footer className="rw-statusbar" inert={readingControlsHidden}
        aria-hidden={readingControlsHidden || undefined}>
        <div>
          <button
            type="button"
            disabled={!frame}
            onClick={() => setPermissionsOpen(true)}
            title={t("Archive permissions")}
          >
            <ShieldCheck size={13} aria-hidden="true" />
            {t("Offline archive")}
          </button>
          {frame?.status.camera === "granted" && (
            <button
              type="button"
              title={t("Revoke camera")}
              disabled={!frameReady}
              onClick={() => void changeSensorDecision("camera")}
            >
              <Camera size={13} />
              <span>{t("Revoke camera")}</span>
            </button>
          )}
          {frame?.status.location === "granted" && (
            <button
              type="button"
              title={t("Revoke location")}
              disabled={!frameReady}
              onClick={() => void changeSensorDecision("location")}
            >
              <LocateFixed size={13} />
              <span>{t("Revoke location")}</span>
            </button>
          )}
          <button
            type="button"
            disabled={!frame}
            onClick={scrollToTop}
            title={t("Back to top")}
          >
            <ArrowUp size={13} aria-hidden="true" />
            <span>{t("Back to top")}</span>
          </button>
        </div>
        <span role="status" aria-live="polite">
          {notice ||
            (selection
              ? t("{count} characters selected", { count: selection.length })
              : "")}
        </span>
        <button type="button" onClick={() => setShortcuts(true)}>
          {t("Shortcuts")}
          <CircleHelp size={13} />
        </button>
      </footer>
      {linkContext && !consent && !external && !attachment &&
        !props.suspended && props.interactive && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={linkContext.resource ? t("Save resource") : t("Link actions")}
          className="rw-link-actions"
          dismissOnBackdrop
          onClose={() => setLinkContext(null)}
        >
          <p>
            {linkContext.resource
              ? t("Save the original archive resource without converting it to PDF.")
              : linkContext.path !== null
              ? props.settings.openNewTabInBackground
                ? t("Open this link in a background tab. Your current page stays open.")
                : t("Open this link in a new tab and switch to it.")
              : t("This link will open in your default browser or another application. It may connect to the internet.")}
          </p>
          <div className="rw-external-url" dir="ltr">{linkContext.url}</div>
          <div className="rw-dialog-actions">
            <button type="button" onClick={() => setLinkContext(null)}>
              {t("Cancel")}
            </button>
            <button
              type="button"
              onClick={async (event) => {
                if (!event.isTrusted) return;
                const context = currentLinkContext();
                setLinkContext(null);
                if (!context) return;
                try {
                  await navigator.clipboard.writeText(context.url);
                  if (alive.current && currentLinkContext()) setNotice(t("Copied"));
                } catch (error) {
                  latest.current.onError(error);
                }
              }}
            >
              {t("Copy link")}
            </button>
            {linkContext.resource && (
              <button
                type="button"
                className="rw-primary"
                disabled={settings.readOnly || exportBusy}
                onClick={async (event) => {
                  if (!event.isTrusted) return;
                  const context = currentLinkContext();
                  if (!context || context.path === null || latest.current.settings.readOnly || exportBusy) return;
                  setLinkContext(null);
                  setExportBusy(true);
                  try {
                    const leaf = new URL(context.url).pathname.split("/").pop() || "resource";
                    const name = decodeURIComponent(leaf).replace(/[/\\:*?"<>|\u0000-\u001f]/g, "_") || "resource";
                    if (!context.session.installationId) throw new Error(t("The archive changed; reopen the article before exporting."));
                    await saveResource(context.session.location.bookId, context.session.installationId, context.path, name);
                  } catch (error) {
                    if (alive.current) latest.current.onError(error);
                  } finally {
                    if (alive.current) setExportBusy(false);
                  }
                }}
              >
                {t("Save resource")}
              </button>
            )}
            <button
              type="button"
              className="rw-primary"
              autoFocus
              onClick={(event) => {
                if (!event.isTrusted) return;
                const context = currentLinkContext();
                setLinkContext(null);
                if (!context) return;
                if (context.path === null) {
                  setExternal({
                    url: context.url,
                  });
                } else {
                  openDestination({
                    bookId: context.session.location.bookId,
                    path: context.path,
                    title: t("Article"),
                    scroll: 0,
                  }, true);
                }
              }}
            >
              {linkContext.path !== null ? t("Open in new tab") : t("Open external link")}
              <ArrowUpRight size={16} aria-hidden="true" />
            </button>
          </div>
        </Dialog>
      )}
      {attachment && !consent && !external && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={t("Open attachment in another application?")}
          busy={attachmentBusy}
          onClose={() => setAttachment(null)}
        >
          <p>
            {t(
              "This extracts an untrusted archive attachment to a local temporary copy and opens your system’s default application. That application may access the internet. Only open files you trust.",
            )}
          </p>
          <div className="rw-external-url" dir="ltr">
            {attachment.path}
          </div>
          <p className="rw-muted">{attachment.mime}</p>
          {attachmentError && (
            <p className="rw-inline-error" role="alert">
              {errorText(attachmentError)}
            </p>
          )}
          <div className="rw-dialog-actions">
            <button
              type="button"
              autoFocus
              disabled={attachmentBusy}
              onClick={() => setAttachment(null)}
            >
              {t("Cancel")}
            </button>
            <button
              type="button"
              disabled={attachmentBusy || exportBusy || settings.readOnly}
              onClick={(event) => {
                if (!event.isTrusted) return;
                void exportArticle();
              }}
            >
              {t("Save file instead")}
            </button>
            <button
              type="button"
              className="rw-primary"
              disabled={attachmentBusy || settings.readOnly}
              onClick={(event) => {
                if (event.isTrusted) void openAttachment();
              }}
            >
              {attachmentBusy ? t("Opening…") : t("Open attachment")}
            </button>
          </div>
        </Dialog>
      )}
      {photoRequest && (
        <Dialog
          title={t("Add a photo")}
          busy={photoStage === "picking"}
          onClose={cancelPhoto}
          canRestoreFocus={canRestoreDialogFocus}
          className="rw-permission-dialog"
        >
          <div className="rw-permission-symbol">
            <Camera size={32} strokeWidth={1.4} />
          </div>
          <p>
            {t("“{book}” asks for a photo.", { book: book?.title || tab.title })}
          </p>
          {photoStage === "camera" ? (
            <p className="rw-muted" role="status">
              {t("Take the photo in the camera window, or cancel.")}
            </p>
          ) : (
            <p className="rw-muted">
              {t("Only the photo you take or choose is given to this page. Kiwix does not keep it.")}
            </p>
          )}
          {photoError && <p className="inline-error" role="alert">{photoError}</p>}
          <div className="rw-dialog-actions">
            <button type="button" ref={photoCancelButton} onClick={cancelPhoto}>
              {t("Cancel")}
            </button>
            <button
              type="button"
              autoFocus={!cameraAvailable}
              disabled={photoStage !== "choose"}
              onClick={() => void choosePhotoFile()}
            >
              {photoStage === "picking" ? t("Please wait…") : t("Choose a file")}
            </button>
            {cameraAvailable && (
              <button
                className="rw-primary"
                type="button"
                autoFocus
                disabled={photoStage !== "choose"}
                onClick={() => void openPhotoCamera()}
              >
                {t("Use camera")}
              </button>
            )}
          </div>
        </Dialog>
      )}
      {consent && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={
            consent.sensor === "camera"
              ? t("Allow camera access?")
              : t("Allow location access?")
          }
          busy={consentBusy}
          onClose={() => {
            setConsentBusy(true);
            void completeSensor(consent, false);
          }}
          className="rw-permission-dialog"
        >
          <div className="rw-permission-symbol">
            {consent.sensor === "camera" ? (
              <Camera size={32} strokeWidth={1.4} />
            ) : (
              <LocateFixed size={32} strokeWidth={1.4} />
            )}
          </div>
          <p>
            {consent.sensor === "camera"
              ? t("“{book}” wants to use your camera.", {
                  book: book?.title || tab.title,
                })
              : t("“{book}” wants to know your location.", {
                  book: book?.title || tab.title,
                })}
          </p>
          <p className="rw-muted">
            {consent.sensor === "camera"
              ? t(
                  "Only this active archive can request the camera. Your system may also ask for permission. You can revoke access below the article at any time.",
                )
              : t(
                  "Your system will provide your location only to this active archive. You can revoke access below the article at any time.",
                )}
          </p>
          <p className="rw-footnote">
            {t(
              "This choice is remembered for this archive only while the app is running.",
            )}
          </p>
          <div className="rw-dialog-actions">
            <button
              type="button"
              autoFocus
              disabled={consentBusy}
              onClick={() => {
                setConsentBusy(true);
                void completeSensor(consent, false);
              }}
            >
              {t("Block")}
            </button>
            <button
              className="rw-primary"
              type="button"
              disabled={consentBusy}
              onClick={() => {
                setConsentBusy(true);
                void completeSensor(consent, true);
              }}
            >
              {consentBusy ? t("Please wait…") : t("Allow")}
            </button>
          </div>
        </Dialog>
      )}
      {external && !consent && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={
            externalLinksBlocked
              ? t("External links are blocked")
              : t("Open outside your offline library?")
          }
          busy={externalBusy}
          onClose={() => setExternal(null)}
        >
          <div className="rw-external-symbol">
            <ExternalLink size={28} strokeWidth={1.4} />
          </div>
          <p>
            {externalLinksBlocked
              ? t(
                  "External links are disabled in Settings. This address has not been opened.",
                )
              : t(
                  "This link will open in your default browser or another application. It may connect to the internet.",
                )}
          </p>
          <div className="rw-external-url" dir="ltr">
            {external.url}
          </div>
          <div className="rw-dialog-actions">
            <button
              type="button"
              autoFocus
              disabled={externalBusy}
              onClick={() => setExternal(null)}
            >
              {externalLinksBlocked ? t("Close") : t("Stay here")}
            </button>
            {!externalLinksBlocked && (
              <button
                className="rw-primary"
                type="button"
                disabled={externalBusy}
                onClick={() => void openExternal()}
              >
                {externalBusy ? t("Opening…") : t("Open external link")}
                <ArrowUpRight size={16} />
              </button>
            )}
          </div>
        </Dialog>
      )}
      {shortcuts && !consent && !external && !attachment && (
        <Dialog
          canRestoreFocus={canRestoreDialogFocus}
          title={t("Keyboard shortcuts")}
          onClose={() => setShortcuts(false)}
        >
          <p className="rw-muted">
            {t("Use Command on macOS, or Control on Windows and Linux.")}
          </p>
          <dl className="rw-shortcut-list">
            {[
              [t("Search books"), t("⌘ / Ctrl K")],
              [t("Find in article"), t("⌘ / Ctrl F")],
              [t("Save article as PDF / original media"), t("⌘ / Ctrl S")],
              [t("Bookmark and notes"), t("⌘ / Ctrl D")],
              ...(isIOS ? [] : [[t("Print / save as PDF"), t("⌘ / Ctrl P")]]),
              [t("Zoom in / out"), t("⌘ / Ctrl + / −")],
              [t("Reset zoom"), t("⌘ / Ctrl 0")],
              [t("Read-aloud speed down / up"), isDesktop
                ? t("< / > in app controls; ⌘ / Ctrl Shift , / . in article")
                : t("< / > in app controls")],
              [t("Back / forward"), t("Alt ← / →")],
              // macOS reserves F11 for Show Desktop; its menu uses Control-Command-F.
              [t("Fullscreen"), isMacos ? t("⌃⌘F") : t("F11")],
              [t("Close panel or find"), t("Esc")],
            ].map(([label, key]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>
                  <kbd>{key}</kbd>
                </dd>
              </div>
            ))}
          </dl>
        </Dialog>
      )}
      {permissionsOpen &&
        frame &&
        !consent &&
        !external &&
        !shortcuts &&
        !attachment && (
          <Dialog
            canRestoreFocus={canRestoreDialogFocus}
            title={t("Archive permissions")}
            onClose={() => setPermissionsOpen(false)}
          >
            <p>
              {t(
                "Permissions apply only to “{book}” while it is the active archive.",
                { book: book?.title || tab.title },
              )}
            </p>
            <div className="rw-permission-list">
              <div>
                <Camera size={20} />
                <span>
                  <strong>{t("Camera")}</strong>
                  <small>
                    {frame.status.camera === "granted"
                      ? t("Allowed for this archive")
                      : frame.status.camera === "denied"
                        ? t("Blocked for this archive")
                        : t("Not requested")}
                  </small>
                </span>
                <button
                  type="button"
                  disabled={!frameReady || frame.status.camera === "notAsked"}
                  onClick={() =>
                    void changeSensorDecision(
                      "camera",
                      frame.status.camera === "denied",
                    )
                  }
                >
                  {frame.status.camera === "granted"
                    ? t("Revoke")
                    : t("Ask again")}
                </button>
              </div>
              <div>
                <LocateFixed size={20} />
                <span>
                  <strong>{t("Location")}</strong>
                  <small>
                    {frame.status.location === "granted"
                      ? t("Allowed for this archive")
                      : frame.status.location === "denied"
                        ? t("Blocked for this archive")
                        : t("Not requested")}
                  </small>
                </span>
                <button
                  type="button"
                  disabled={!frameReady || frame.status.location === "notAsked"}
                  onClick={() =>
                    void changeSensorDecision(
                      "location",
                      frame.status.location === "denied",
                    )
                  }
                >
                  {frame.status.location === "granted"
                    ? t("Revoke")
                    : t("Ask again")}
                </button>
              </div>
            </div>
            <p className="rw-footnote">
              {t(
                "An archive must ask before using a sensor. Blocking is remembered until you choose Ask again. Revoking or resetting stops active devices and reloads the article in a fresh isolated origin.",
              )}
            </p>
            <div className="rw-external-url" dir="ltr">
              {frame.location.origin}
            </div>
          </Dialog>
        )}
    </div>
  );
}
