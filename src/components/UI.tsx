import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createRoot } from "react-dom/client";
import { AlertCircle, BookOpen, LoaderCircle, X } from "lucide-react";
import { t, formatBytes, formatNumber } from "../i18n";
import type { Book, CatalogEntry, Settings, TabLocation } from "../types";
import {
  ERROR_SITES,
  isSuspendedError,
  type ErrorSeverity,
} from "../error-visibility";
import { diagnostics } from "../diagnostics";
import { api, getLibraryFilters, isIOS, saveLibraryFilters } from "../api";
import "./library-catalog.css";

export interface SurfaceProps {
  books: Book[];
  settings: Settings;
  onRefresh: () => void;
  onOpen: (location: TabLocation) => void;
  onError: (error: unknown, severity?: ErrorSeverity) => void;
}
// Presentation only: recognize native filesystem roots, not archive-relative
// article locations or arbitrary secrets. Keep original errors/export bytes intact.
const filesystemRoot = String.raw`(?:file:(?:\\?\/){2,}|(?:\\?\/)(?:private|var|Users|home|tmp|Applications|Library|System|Volumes|storage|data|mnt|etc|opt|usr|root|run|dev|bin|sbin|cores)(?=\\?\/|[\s:;,"'\x60)\]]|$)|[A-Za-z]:[\\/])`;
const quotedFilesystemPath = new RegExp(String.raw`(\\?["'\x60])${filesystemRoot}[^\r\n]*?\1`, "g");
const bareFilesystemPath = new RegExp(
  String.raw`(^|[\s(=:\[])${filesystemRoot}[^\r\n:;,"'\x60)\]]*?(?=[:;,"'\x60)\]\r\n]|[ \t]+(?:is|was|has|requires|exceeds|without|outside|does|cannot|could|failed)\b|$)`,
  "g",
);
export function pathFileName(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "").split("/").pop() || t("File");
}
export function errorText(error: unknown): string {
  // A suspended command is a lifecycle gate that the API boundary retries on
  // resume. It has no failure text, so no caller can paint a card for it.
  if (isSuspendedError(error)) return "";
  const message = error instanceof Error ? error.message : String(error);
  // Never paint a bare serialization/internal token in the red band. These
  // values carry no action for a reader and usually mean a caller passed the
  // wrong shape across a boundary.
  if (["data", "error", "undefined", "null", "object", "[object object]"].includes(message.trim().toLowerCase()))
    return t("That action did not finish. Try again.");
  const nearby = (() => {
    switch (message.trim()) {
    case "BLE_CONNECT_TIMEOUT":
    case "BLE_CONNECT_FAILED":
    case "BLE_DISCONNECTED":
    case "NEARBY_BLUETOOTH_CONNECT_TIMEOUT":
    case "NEARBY_BLUETOOTH_DISCONNECTED":
    case "NEARBY_BLUETOOTH_MTU_CHANGED":
    case "NEARBY_BLUETOOTH_INVALID_PACKET":
    case "NEARBY_BLUETOOTH_INVALID_SUBSCRIPTION":
    case "NEARBY_BLUETOOTH_SUBSCRIBE_TIMEOUT":
      return t("The other device stopped responding. Keep Nearby open on both devices, move them closer, and try again.");
    case "BLE_PEER_UNAVAILABLE":
    case "NEARBY_BLUETOOTH_STALE_PEER":
      return t("That device is no longer nearby. Refresh the list and try again.");
    case "BLE_FOREGROUND_REQUIRED":
    case "NEARBY_BLUETOOTH_FOREGROUND_REQUIRED":
      return t("Keep Kiwix open on this device while sharing.");
    case "BLE_ADVERTISE_FAILED":
    case "BLE_SERVICE_FAILED":
    case "NEARBY_BLUETOOTH_ADVERTISE_FAILED":
    case "NEARBY_BLUETOOTH_SERVICE_FAILED":
      return t("Bluetooth sharing could not start. Turn Bluetooth off and on, then try again.");
    case "NEARBY_BLUETOOTH_PERIPHERAL_UNSUPPORTED":
      return t("This device cannot share over Bluetooth. Use the same local network instead.");
    case "NEARBY_BLUETOOTH_LOCATION_SERVICES_REQUIRED":
      return t("Turn on location services to find nearby devices over Bluetooth.");
    case "NEARBY_BLUETOOTH_CONNECTION_LIMIT":
      return t("One nearby connection is already active. Finish or stop it before choosing another device.");
    case "JOIN_ASSOCIATION_UNAVAILABLE_OR_WIFI_INFO_DENIED":
      return t("Approve the Wi-Fi join prompt, or ask the sender to keep the temporary network open, then try again.");
    case "JOIN_USER_DENIED":
    case "JOIN_ENTITLEMENT_OR_SYSTEM_DENIED":
    case "NEARBY_PERMISSION_DENIED":
    case "HOTSPOT_PERMISSION_DENIED":
    case "NEARBY_LOCATION_SERVICES_REQUIRED":
      return t("Allow Wi-Fi and location access for Nearby, then try again.");
    case "LOCAL_NETWORK_PERMISSION_DENIED":
    case "LOCAL_NETWORK_PERMISSION_CANCELLED":
      return t("Allow local network access for Nearby, then try again.");
    case "LOCAL_NETWORK_PERMISSION_TIMEOUT":
    case "LOCAL_NETWORK_PERMISSION_BUSY":
      return t("Local network access is still waiting for approval. Try again.");
    case "JOIN_ROUTE_LOST":
    case "JOIN_BACKGROUND":
    case "NEARBY_JOIN_UNAVAILABLE":
    case "NEARBY_HOTSPOT_ROUTE_LOST":
    case "NEARBY_ENDPOINT_NOT_ON_HOTSPOT":
      return t("The temporary Wi-Fi connection ended or changed. Keep both apps open and try again.");
    case "NEARBY_JOIN_TIMEOUT":
    case "HOTSPOT_START_PENDING":
    case "HOTSPOT_REQUEST_PENDING":
      return t("The temporary Wi-Fi is still starting. Wait a moment and try again.");
    case "HOTSPOT_INCOMPATIBLE_MODE":
    case "HOTSPOT_UNSUPPORTED":
    case "NEARBY_HOTSPOT_UNSUPPORTED":
      return t("This device cannot create or join a temporary Wi-Fi network. Use the same Wi-Fi network.");
    case "HOTSPOT_UNAVAILABLE":
    case "HOTSPOT_CONFIGURATION_UNAVAILABLE":
    case "HOTSPOT_START_CANCELLED":
    case "HOTSPOT_RELEASE_FAILED":
      return t("The temporary Wi-Fi could not start or was cancelled. Try again.");
    case "NEARBY_HOTSPOT_EXACT_INTERFACE_UNAVAILABLE":
      return t("The temporary Wi-Fi is still starting. Wait a moment and try again.");
    case "NEARBY_HOTSPOT_NOT_OWNED":
      return t("The temporary Wi-Fi is not active. Start sharing with Use temporary Wi-Fi again.");
    case "NEARBY_VISIBLE_ACTIVITY_REQUIRED":
    case "VISIBLE_ACTIVITY_REQUIRED":
      return t("Keep Kiwix open on this device while connecting.");
    case "NEARBY_LAN_REQUIRED":
      return t("This device is not connected to a Wi-Fi network. Share all needs both devices on the same Wi-Fi; use Use temporary Wi-Fi instead.");
    case "MOBILE_NETWORK_BLOCKED":
      return t("This needs an Internet connection. Connect to Wi-Fi or allow mobile data, then try again.");
    case "NEARBY_BLUETOOTH_NOT_STARTED":
    case "BLE_OWNER_BUSY":
    case "BLE_OWNER_RETIRED":
    case "BLE_INVALID_REQUEST":
      return t("Nearby sharing is no longer active. Start it again.");
    default:
      return null;
    }
  })();
  if (nearby) return nearby;
  if (/checksum|corrupt|source changed|integrity|verification/i.test(message))
    return t("The sender's archive failed verification. Stop sharing and start again, or download it again from the catalog.");
  if (/Apple QUIC|QUIC connection|left the local link|LocalSwarm (?:route|socket|listener)|LocalSwarm connect timed out/i.test(message))
    return t("The sender stopped sharing or is no longer reachable on this Wi-Fi. Keep both apps open on the same network, then retry the download.");
  if (/(?:^|\[)provider_incoming_choose_original_or_copy(?:\]|$)/.test(message))
    return t("This file cannot be opened from its current location. Add a copy to your library, or choose the original file in Files.");
  if (!isIOS) return message;
  const replacement = `[${t("File")}]`;
  return message
    .replace(quotedFilesystemPath, replacement)
    .replace(bareFilesystemPath, (_path, prefix: string) => prefix + replacement);
}
export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <header className="page-heading">
      <div>
        <h1>{title}</h1>
        {description && <p className="page-description">{description}</p>}
      </div>
      {actions && <div className="heading-actions">{actions}</div>}
    </header>
  );
}
export function Empty({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <h2>{title}</h2>
      <p>{description}</p>
      {children && <div className="empty-actions">{children}</div>}
    </div>
  );
}
export function Loading({ label = t("Loading…") }: { label?: string }) {
  return (
    <div className="loading-state" role="status">
      <LoaderCircle className="spin" size={20} />
      {label}
    </div>
  );
}
export function ErrorNotice({
  message,
  onRetry,
}: {
  message: string;
  onRetry?: () => void;
}) {
  return (
    <div className="error-notice" role="alert">
      <AlertCircle size={18} />
      <span>{errorText(message)}{onRetry ? " " : null}</span>
      {onRetry && <button onClick={onRetry}>{t("Try again")}</button>}
    </div>
  );
}
/** Keep only one page of matching rows mounted. Filtering and sorting belong to the caller. */
export function PagedResults<T>({ items, criteria, label, className, children }: {
  items: readonly T[];
  criteria: string;
  label: string;
  className?: string;
  children: (item: T) => ReactNode;
}) {
  const size = 50;
  const [selection, setSelection] = useState({ criteria, page: 0 });
  const last = Math.max(0, Math.ceil(items.length / size) - 1);
  const page = selection.criteria === criteria ? Math.min(selection.page, last) : 0;
  if (selection.criteria !== criteria || selection.page !== page)
    setSelection({ criteria, page });
  const start = page * size;
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  function select(next: number) {
    setSelection({ criteria, page: next });
    // Move keyboard and scroll position back to the start of this section.
    ref.current?.focus({ preventScroll: true });
    ref.current?.scrollIntoView({ block: "start" });
  }
  return <>
    <div ref={ref} id={id} className={className} tabIndex={-1} role="group" aria-label={label}>
      {items.slice(start, start + size).map(children)}
    </div>
    {last > 0 && <nav className="pagination" aria-label={label}>
      <button type="button" disabled={page === 0} aria-controls={id}
        onClick={() => select(page - 1)}>{t("Previous")}</button>
      <span role="status">{t("{start}–{end} of {total}", {
        start: start + 1, end: Math.min(start + size, items.length), total: items.length,
      })}</span>
      <button type="button" disabled={page === last} aria-controls={id}
        onClick={() => select(page + 1)}>{t("Next")}</button>
    </nav>}
  </>;
}

export function Modal({
  title,
  children,
  onClose,
  initialFocus,
  describedBy,
  presentation = "panel",
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  initialFocus?: RefObject<HTMLElement | null>;
  describedBy?: string;
  /**
   * Phones present panels as bottom sheets (src/platform.css). A confirmation
   * is a question with two answers, so it keeps the centred dialog every
   * platform shows on larger screens instead of putting its buttons on the
   * edge a thumb rests against.
   */
  presentation?: "panel" | "confirmation";
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.showModal();
    initialFocus?.current?.focus({ preventScroll: true });
    return () => previous?.focus();
  }, [initialFocus]);
  return (
    <dialog
      ref={ref}
      className={presentation === "confirmation" ? "modal modal-confirmation" : "modal"}
      aria-labelledby={id}
      aria-describedby={describedBy}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <header className="modal-heading">
        <h2 id={id}>{title}</h2>
        <button
          className="icon-button"
          onClick={onClose}
          aria-label={t("Close")}
        >
          <X size={20} />
        </button>
      </header>
      {children}
    </dialog>
  );
}

export interface ConfirmOptions {
  /** Dialog heading; defaults to an existing generic heading. */
  title?: string;
  /** Action label; defaults to the app's generic affirmative label. */
  confirmLabel?: string;
  /** Style the affirmative action as destructive. */
  danger?: boolean;
}

// One question at a time: native confirms blocked, so later questions wait.
let confirmQueue: Array<() => void> = [];
let confirmShowing = false;

function showNextConfirm() {
  if (confirmShowing) return;
  const start = confirmQueue.shift();
  if (!start) return;
  confirmShowing = true;
  start();
}

/**
 * Promise-based confirmation for the Tauri webview, which has no native
 * blocking confirm on iOS or Android. Each question renders in its own React
 * root on document.body, so callers outside the React tree (window close
 * handlers) can await it too.
 */
export function confirmAction(
  message: string,
  options: ConfirmOptions = {},
): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    confirmQueue.push(() => {
      const host = document.createElement("div");
      document.body.append(host);
      const root = createRoot(host);
      let settled = false;
      const finish = (value: boolean) => {
        if (settled) return;
        settled = true;
        // Unmount after the dialog's own event dispatch has finished, then
        // resolve so focus restoration completes before the caller continues.
        queueMicrotask(() => {
          try {
            root.unmount();
          } finally {
            host.remove();
            resolve(value);
            confirmShowing = false;
            showNextConfirm();
          }
        });
      };
      root.render(
        <ConfirmDialog
          message={message}
          title={options.title ?? t("Confirmation required")}
          confirmLabel={options.confirmLabel ?? t("Continue")}
          danger={options.danger ?? false}
          onClose={finish}
        />,
      );
    });
    showNextConfirm();
  });
}

function ConfirmDialog({
  message,
  title,
  confirmLabel,
  danger,
  onClose,
}: {
  message: string;
  title: string;
  confirmLabel: string;
  danger: boolean;
  onClose: (value: boolean) => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const messageId = useId();
  return (
    <Modal
      title={title}
      onClose={() => onClose(false)}
      initialFocus={cancel}
      describedBy={messageId}
      presentation="confirmation"
    >
      <p className="confirm-message" id={messageId}>
        {message}
      </p>
      <div className="modal-actions">
        <button ref={cancel} type="button" onClick={() => onClose(false)}>
          {t("Cancel")}
        </button>
        <button
          type="button"
          className={danger ? "danger" : "primary"}
          onClick={() => onClose(true)}
        >
          {confirmLabel}
        </button>
      </div>
    </Modal>
  );
}
const coverRequests = new Map<string, Promise<string | null>>();
const COVER_CACHE_LIMIT = 128;

/** One IPC round-trip per cover key, shared by every card that mounts it. */
function coverRequest(book: { id: string; thumbnailUrl?: string | null }): Promise<string | null> {
  const key = book.thumbnailUrl
    ? `${book.id}:thumbnail:${book.thumbnailUrl}`
    : `${book.id}:illustration`;
  const cached = coverRequests.get(key);
  if (cached) return cached;
  const request = (
    book.thumbnailUrl
      ? api<string | null>("catalog_thumbnail", { url: book.thumbnailUrl })
      : api<string | null>("book_illustration", { id: book.id })
  ).catch((error) => {
    /* Decorative artwork must not prevent opening an archive. */
    diagnostics.record(error, ERROR_SITES["library.cover-refresh"]);
    coverRequests.delete(key);
    return null;
  });
  coverRequests.set(key, request);
  if (coverRequests.size > COVER_CACHE_LIMIT) {
    const oldest = coverRequests.keys().next().value;
    if (oldest !== undefined) coverRequests.delete(oldest);
  }
  return request;
}

export function BookCover({
  book,
}: {
  book: {
    id: string;
    title: string;
    language: string;
    tags: string[];
    available?: boolean;
    thumbnailUrl?: string | null;
  };
}) {
  const cover = useRef<HTMLDivElement>(null);
  const [image, setImage] = useState<string | null>(null);
  useEffect(() => {
    setImage(null);
    if ((!book.available && !book.thumbnailUrl) || !cover.current) return;
    let alive = true;
    let started = false;
    const load = () => {
      if (started) return;
      started = true;
      void coverRequest(book).then((value) => {
        if (alive) setImage(value);
      });
    };
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer.disconnect();
          load();
        }
      },
      { rootMargin: "160px" },
    );
    observer.observe(cover.current);
    return () => {
      alive = false;
      observer.disconnect();
    };
  }, [book.id, book.available, book.thumbnailUrl]);
  return (
    <div
      ref={cover}
      className="book-cover"
      aria-hidden="true"
    >
      <span>{book.language.toLocaleUpperCase()}</span>
      {image ? (
        <img className="cover-illustration" src={image} alt="" />
      ) : (
        <BookOpen size={22} strokeWidth={1.2} />
      )}
    </div>
  );
}
export function BookFacts({
  book,
}: {
  book: Pick<
    Book,
    "size" | "articleCount" | "mediaCount" | "publisher" | "creator" | "date"
  >;
}) {
  return (
    <dl className="facts">
      <div>
        <dt>{t("Size")}</dt>
        <dd>{formatBytes(book.size)}</dd>
      </div>
      <div>
        <dt>{t("Articles")}</dt>
        <dd>{formatNumber(book.articleCount)}</dd>
      </div>
      <div>
        <dt>{t("Media files")}</dt>
        <dd>{formatNumber(book.mediaCount)}</dd>
      </div>
      <div>
        <dt>{t("Publisher")}</dt>
        <dd>{book.publisher || t("Not specified")}</dd>
      </div>
      <div>
        <dt>{t("Creator")}</dt>
        <dd>{book.creator || t("Not specified")}</dd>
      </div>
      <div>
        <dt>{t("Date")}</dt>
        <dd>{book.date || t("Not specified")}</dd>
      </div>
    </dl>
  );
}
export function jobStatus(status: string): string {
  const labels: Record<string, string> = {
    queued: t("Queued"),
    downloading: t("Downloading"),
    paused: t("Paused"),
    completed: t("Completed"),
    complete: t("Completed"),
    failed: t("Failed"),
    error: t("Failed"),
    cancelled: t("Cancelled"),
    canceled: t("Cancelled"),
    assembling: t("Assembling"),
    verifying: t("Verifying"),
    importing: t("Importing"),
    running: t("In progress"),
    preparing: t("Preparing"),
    resolving: t("Finding torrent peers"),
    checking: t("Checking existing data"),
    publishing: t("Publishing archive"),
    seeding: t("Seeding"),
    validating: t("Validating map recipe"),
    composing: t("Composing map"),
  };
  return labels[status] || t("Status: {status}", { status });
}
export function terminalStatus(status: string) {
  return [
    "completed",
    "complete",
    "failed",
    "error",
    "cancelled",
    "canceled",
  ].includes(status);
}

export interface ArchiveFilters {
  languages: string[];
  categories: string[];
  content: string[];
}
export interface FacetChoice {
  value: string;
  label: string;
  count?: number | null;
}
export const emptyArchiveFilters = (): ArchiveFilters => ({
  languages: [],
  categories: [],
  content: [],
});
export function archiveLanguages(language: string) {
  return language
    .split(/[,;]/)
    .map((value) => value.trim())
    .filter(Boolean);
}
export function archiveCategories(tags: string[]) {
  return tags
    .filter((tag) => tag.startsWith("_category:"))
    .map((tag) => tag.slice(10));
}
const CONTENT_PREFIXES = ["_pictures:", "_videos:", "_details:"] as const;

interface BookFacets {
  languages: Set<string>;
  categories: Set<string>;
  tags: Set<string>;
}
/** Parsed facets are derived once per book object; snapshots replace objects. */
const bookFacets = new WeakMap<object, BookFacets>();
function facetsOf(book: { language: string; tags: string[] }): BookFacets {
  let facets = bookFacets.get(book);
  if (!facets) {
    facets = {
      languages: new Set(archiveLanguages(book.language)),
      categories: new Set(archiveCategories(book.tags)),
      tags: new Set(book.tags),
    };
    bookFacets.set(book, facets);
  }
  return facets;
}

interface FilterIndex {
  languages: Set<string>;
  categories: Set<string>;
  /** Prefix → its single selected content tag, or null when unconstrained. */
  content: Record<string, string | null>;
}
/** Filter selections are derived once per filter object, not once per book. */
const filterIndexes = new WeakMap<object, FilterIndex>();
function filterIndexOf(filters: ArchiveFilters): FilterIndex {
  let index = filterIndexes.get(filters);
  if (!index) {
    const content: Record<string, string | null> = {};
    for (const prefix of CONTENT_PREFIXES) {
      const choices = filters.content.filter((tag) => tag.startsWith(prefix));
      content[prefix] = choices.length === 1 ? choices[0] : null;
    }
    index = {
      languages: new Set(filters.languages),
      categories: new Set(filters.categories),
      content,
    };
    filterIndexes.set(filters, index);
  }
  return index;
}

function intersects(values: Set<string>, selected: Set<string>) {
  if (!selected.size) return true;
  for (const value of values) {
    if (selected.has(value)) return true;
  }
  return false;
}

export function matchesArchiveFilters(
  book: { language: string; tags: string[] },
  filters: ArchiveFilters,
) {
  const facets = facetsOf(book);
  const index = filterIndexOf(filters);
  if (!intersects(facets.languages, index.languages)) return false;
  if (!intersects(facets.categories, index.categories)) return false;
  for (const prefix of CONTENT_PREFIXES) {
    const required = index.content[prefix];
    if (required !== null && !facets.tags.has(required)) return false;
  }
  return true;
}
export function contentChoices(): FacetChoice[] {
  return [
    { value: "_pictures:yes", label: t("With pictures") },
    { value: "_pictures:no", label: t("Without pictures") },
    { value: "_videos:yes", label: t("With videos") },
    { value: "_videos:no", label: t("Without videos") },
    { value: "_details:yes", label: t("Full articles") },
    { value: "_details:no", label: t("Introductions only") },
  ];
}
export function EditionBadges({
  book,
}: {
  book: { language: string; tags: string[]; date: string; name: string };
}) {
  const flavor =
    book.tags.find((tag) => tag.startsWith("_flavour:"))?.slice(9) ||
    book.name.match(/(?:^|_)(maxi|mini|nopic)(?:_|$)/)?.[1];
  const contents = contentChoices().filter((choice) =>
    book.tags.includes(choice.value),
  );
  return (
    <div className="edition-badges">
      {flavor && <span className="badge edition-flavor">{flavor}</span>}
      {contents.map((choice) => (
        <span className="badge" key={choice.value}>
          {choice.label}
        </span>
      ))}
    </div>
  );
}

/** Patch only this surface's preferences; read-only changes remain local. */
export function useArchiveFilters(
  settings: Settings,
  key: "libraryFilters" | "catalogFilters",
  onError: (e: unknown) => void,
) {
  const [filters, setFilters] = useState<ArchiveFilters>(
    () => getLibraryFilters(key) || settings[key] || emptyArchiveFilters(),
  );
  useEffect(() => {
    setFilters(
      getLibraryFilters(key) || settings[key] || emptyArchiveFilters(),
    );
  }, [key, settings.privateMode, settings.readOnly]);
  function change(next: ArchiveFilters, extra?: Record<string, unknown>) {
    setFilters(next);
    if (!settings.readOnly)
      saveLibraryFilters(key, next, { onSaved: () => {}, onError }, extra);
  }
  return [filters, change] as const;
}

function FacetPicker({
  label,
  options,
  selected,
  onChange,
}: {
  label: string;
  options: FacetChoice[];
  selected: string[];
  onChange: (values: string[]) => void;
}) {
  const [query, setQuery] = useState("");
  const picker = useRef<HTMLDetailsElement>(null);
  const summary = useRef<HTMLElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const closePicker = useCallback((restoreFocus = false) => {
    const details = picker.current;
    if (!details) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && popover.current?.contains(active))
      active.blur();
    details.open = false;
    if (restoreFocus) summary.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    const details = picker.current;
    const trigger = summary.current;
    const panel = popover.current;
    if (!details || !trigger || !panel) return;
    const viewport = window.visualViewport;
    const content = details.closest<HTMLElement>("#main-content");
    let frame = 0;
    function positionPopover() {
      if (!details!.open) return;
      // WKWebView pans fixed elements and their DOMRects when showing the
      // keyboard. Compare viewport and content in the same client coordinates,
      // then convert placement back to the panel's CSS fixed-position origin.
      const panelStyle = getComputedStyle(panel!);
      const previousBounds = panel!.getBoundingClientRect();
      const originLeft = previousBounds.left - parseFloat(panelStyle.left);
      const originTop = previousBounds.top - parseFloat(panelStyle.top);
      const viewportLeft = (viewport?.offsetLeft ?? 0) + originLeft;
      const viewportTop = (viewport?.offsetTop ?? 0) + originTop;
      const viewportRight = viewportLeft + (viewport?.width ?? window.innerWidth);
      const viewportBottom = viewportTop + (viewport?.height ?? window.innerHeight);
      const contentBounds = content?.getBoundingClientRect();
      // The visible workspace excludes app chrome, including the fixed mobile
      // navigation. A visual viewport alone still includes that covered area.
      const leftEdge = Math.max(viewportLeft, contentBounds?.left ?? viewportLeft) + 8;
      const topEdge = Math.max(viewportTop, contentBounds?.top ?? viewportTop) + 8;
      const rightEdge = Math.min(viewportRight, contentBounds?.right ?? viewportRight) - 8;
      const bottomEdge = Math.min(viewportBottom, contentBounds?.bottom ?? viewportBottom) - 8;
      const width = Math.max(0, rightEdge - leftEdge);
      const height = Math.max(0, bottomEdge - topEdge);
      // IME animation can temporarily leave no usable workspace. Geometry is
      // not dismissal intent: retain the open picker until a later resize.
      if (!width || !height) return;
      panel!.style.width = `${Math.min(310, width)}px`;
      panel!.style.maxHeight = `${height}px`;
      const anchor = trigger!.getBoundingClientRect();
      const bounds = panel!.getBoundingClientRect();
      const preferredLeft =
        getComputedStyle(details!).direction === "rtl"
          ? anchor.right - bounds.width
          : anchor.left;
      panel!.style.left = `${Math.max(leftEdge, Math.min(preferredLeft, leftEdge + width - bounds.width)) - originLeft}px`;
      panel!.style.top = `${Math.max(topEdge, Math.min(anchor.bottom + 6, topEdge + height - bounds.height)) - originTop}px`;
    }
    function schedulePosition(event?: Event) {
      if (event?.target instanceof Node && panel!.contains(event.target)) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(positionPopover);
    }
    function openPicker() {
      // Native toggle events are queued: close siblings synchronously so a
      // second summary can never leave two panels covering the results.
      document
        .querySelectorAll<HTMLDetailsElement>(".facet-picker[open]")
        .forEach((other) => {
          if (other === details) return;
          const active = document.activeElement;
          if (active instanceof HTMLElement && other.contains(active))
            active.blur();
          other.open = false;
        });
      details!.open = true;
      positionPopover();
    }
    function toggleFromSummary(event: MouseEvent) {
      event.preventDefault();
      if (details!.open) closePicker(true);
      else openPicker();
    }
    function onToggle() {
      if (details!.open) openPicker();
      else closePicker();
    }
    function onOutsideInteraction(event: Event) {
      // Safari may focus a focusable ancestor when tapping a checkbox.
      // This is not departure to another control; outside taps still dismiss.
      if (
        event.type === "focusin" &&
        event.target instanceof Node &&
        event.target.contains(details)
      )
        return;
      if (
        details!.open &&
        event.target instanceof Node &&
        !details!.contains(event.target)
      )
        closePicker();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (details!.open && event.key === "Escape" && !event.isComposing) {
        event.preventDefault();
        event.stopPropagation();
        closePicker(true);
      }
    }
    const observer = new ResizeObserver(() => schedulePosition());
    observer.observe(panel);
    if (content) observer.observe(content);
    trigger.addEventListener("click", toggleFromSummary);
    details.addEventListener("toggle", onToggle);
    document.addEventListener("pointerdown", onOutsideInteraction, true);
    document.addEventListener("focusin", onOutsideInteraction);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", schedulePosition);
    window.addEventListener("scroll", schedulePosition, true);
    viewport?.addEventListener("resize", schedulePosition);
    viewport?.addEventListener("scroll", schedulePosition);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      trigger.removeEventListener("click", toggleFromSummary);
      details.removeEventListener("toggle", onToggle);
      document.removeEventListener("pointerdown", onOutsideInteraction, true);
      document.removeEventListener("focusin", onOutsideInteraction);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", schedulePosition);
      window.removeEventListener("scroll", schedulePosition, true);
      viewport?.removeEventListener("resize", schedulePosition);
      viewport?.removeEventListener("scroll", schedulePosition);
    };
  }, [closePicker]);
  const indexedOptions = useMemo(
    () =>
      options
        .map((option) => ({
          option,
          haystack: `${option.label} ${option.value}`.toLocaleLowerCase(),
        }))
        .sort((a, b) => a.option.label.localeCompare(b.option.label)),
    [options],
  );
  const visible = useMemo(() => {
    const needle = query.toLocaleLowerCase();
    return (needle
      ? indexedOptions.filter((entry) => entry.haystack.includes(needle))
      : indexedOptions
    ).map((entry) => entry.option);
  }, [indexedOptions, query]);
  return (
    <details ref={picker} className="facet-picker">
      <summary ref={summary}>
        {label}
        <span>{selected.length || t("All")}</span>
      </summary>
      <div ref={popover} className="facet-popover">
        <div className="facet-popover-heading">
          <button
            type="button"
            disabled={!selected.length}
            onClick={() => onChange([])}
          >
            {t("Clear selection")}
          </button>
          <button type="button" onClick={() => closePicker(true)}>
            {t("Close")}
          </button>
        </div>
        <input
          aria-label={t("Search {filter}", { filter: label })}
          placeholder={t("Search choices…")}
          enterKeyHint="done"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key !== "Enter" ||
              event.nativeEvent.isComposing ||
              event.nativeEvent.keyCode === 229
            )
              return;
            event.preventDefault();
            event.stopPropagation();
            // Done dismisses only the keyboard; keep the query and choices
            // available once the viewport expands again.
            event.currentTarget.blur();
            summary.current?.focus({ preventScroll: true });
          }}
        />
        <div className="facet-choices">
          {visible.map((option) => (
            <label key={option.value}>
              <input
                type="checkbox"
                checked={selected.includes(option.value)}
                onChange={(event) =>
                  onChange(
                    event.target.checked
                      ? [...selected, option.value]
                      : selected.filter((value) => value !== option.value),
                  )
                }
              />
              <span>
                {option.label}
                <small>
                  {option.value.startsWith("_") ? "" : option.value}
                </small>
              </span>
              {option.count != null && (
                <small>{formatNumber(option.count)}</small>
              )}
            </label>
          ))}
          {!visible.length && (
            <p className="muted">{t("No matching choices")}</p>
          )}
        </div>
      </div>
    </details>
  );
}
export function ArchiveFilterBar({
  filters,
  onChange,
  languages,
  categories,
}: {
  filters: ArchiveFilters;
  onChange: (filters: ArchiveFilters) => void;
  languages: FacetChoice[];
  categories: FacetChoice[];
}) {
  const groups = [
    { key: "languages" as const, label: t("Languages"), options: languages },
    { key: "categories" as const, label: t("Categories"), options: categories },
    { key: "content" as const, label: t("Content"), options: contentChoices() },
  ];
  const count = groups.reduce(
    (sum, group) => sum + filters[group.key].length,
    0,
  );
  return (
    <div className="archive-filter-bar">
      <div className="archive-filter-controls">
        {groups.map((group) => (
          <FacetPicker
            key={group.key}
            label={group.label}
            options={group.options}
            selected={filters[group.key]}
            onChange={(values) => onChange({ ...filters, [group.key]: values })}
          />
        ))}
        {!!count && (
          <button
            type="button"
            className="text-button"
            onClick={() => onChange(emptyArchiveFilters())}
          >
            {t("Clear filters")}
          </button>
        )}
      </div>
      {!!count && (
        <div className="filter-chips" aria-label={t("Selected filters")}>
          {groups.flatMap((group) =>
            filters[group.key].map((value) => (
              <button
                type="button"
                className="filter-chip"
                key={`${group.key}:${value}`}
                onClick={() =>
                  onChange({
                    ...filters,
                    [group.key]: filters[group.key].filter(
                      (item) => item !== value,
                    ),
                  })
                }
                aria-label={t("Remove filter {filter}", {
                  filter:
                    group.options.find((o) => o.value === value)?.label ||
                    value,
                })}
              >
                {group.options.find((option) => option.value === value)
                  ?.label || value}
                <X size={13} />
              </button>
            )),
          )}
        </div>
      )}
    </div>
  );
}

export const archiveSortFields = () =>
  [
    ["title", t("Title")],
    ["size", t("Size")],
    ["date", t("Publication date")],
    ["language", t("Language")],
    ["available", t("Availability")],
    ["creator", t("Creator")],
    ["publisher", t("Publisher")],
    ["articleCount", t("Articles")],
    ["mediaCount", t("Media files")],
    ["content", t("Content")],
    ["description", t("Description")],
  ] as const;
const archiveSortKeys = new WeakMap<object, Map<string, unknown>>();
function archiveSortKey(
  book: Book | CatalogEntry,
  field: string,
): unknown {
  let keys = archiveSortKeys.get(book);
  if (!keys) {
    keys = new Map();
    archiveSortKeys.set(book, keys);
  }
  if (keys.has(field)) return keys.get(field);
  const key =
    field === "content"
      ? book.tags.join(";")
      : field === "available"
        ? "available" in book
          ? book.available
          : false
        : field === "addedAt"
          ? "addedAt" in book
            ? book.addedAt
            : ""
          : book[field as keyof (Book | CatalogEntry)];
  keys.set(field, key);
  return key;
}
export function compareArchives(
  a: Book | CatalogEntry,
  b: Book | CatalogEntry,
  field: string,
) {
  const av = archiveSortKey(a, field);
  const bv = archiveSortKey(b, field);
  if (typeof av === "number" && typeof bv === "number") return av - bv;
  if (typeof av === "boolean" && typeof bv === "boolean")
    return Number(av) - Number(bv);
  return String(av ?? "").localeCompare(String(bv ?? ""), undefined, {
    numeric: true,
    sensitivity: "base",
  });
}
