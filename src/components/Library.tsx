import { useEffect, useMemo, useRef, useState } from "react";
import { displayPath } from "../display-path";
import {
  ArrowDownWideNarrow,
  BookOpen,
  Check,
  Clock,
  Copy,
  FolderOpen,
  Grid2X2,
  List,
  Info,
  Mic,
  MoreHorizontal,
  Pin,
  Plus,
  Search,
  Share2,
  ShieldAlert,
  Shuffle,
  Trash2,
} from "lucide-react";
import {
  api,
  capturePrivacyContext,
  chooseFiles,
  createSearchRequest,
  isPrivacyContextCurrent,
  isPrivacyPolicyContextCurrent,
  isMobile,
  isAndroid,
  isIOS,
  saveBytes,
  subscribeReaderPreferences,
  type PrivacyContext,
  type SearchRequest,
} from "../api";
import {
  t,
  formatBytes,
  formatDate,
  formatNumber,
  languageName,
  getLocale,
} from "../i18n";
import { ERROR_SITES } from "../error-visibility";
import { diagnostics } from "../diagnostics";
import {
  MAX_PINNED_BOOKS,
  orderLibraryBooks,
  usablePinnedBooks,
} from "../library-pins";
import type {
  Book,
  DeviceArchive,
  DeviceScan,
  DownloadJob,
  ReaderLocation,
  SearchResults,
  TitleSuggestion,
} from "../types";
import IntegrityPanel, { IntegrityBadge, useIntegritySnapshot } from "./IntegrityPanel";
import KokoroDownloadPrompt from "./KokoroDownloadPrompt";
import { SwipeRow } from "./SwipeRow";
import SelectedStorageActions from "./SelectedStorageActions";
import {
  ArchiveTags,
  BookCover,
  BookFacts,
  Empty,
  ErrorNotice,
  Loading,
  PagedResults,
  Modal,
  PageHeader,
  type SurfaceProps,
  ArchiveFilterBar,
  EditionBadges,
  archiveCategories,
  archiveLanguages,
  archiveSortFields,
  compareArchives,
  matchesArchiveFilters,
  errorText,
  pathFileName,
  useArchiveFilters,
} from "./UI";
function deviceVolume(volume: string) {
  return volume === "primary" ? t("Internal storage") : volume;
}
const SUGGESTION_PAGE_SIZE = 15;
const SUGGESTION_DEBOUNCE_MS = 120;
/** Appended result pages stop here so one search cannot grow without bound. */
const MAX_SEARCH_HITS = 500;
interface Props extends SurfaceProps {
  importing: boolean;
  onImport: () => void;
  onScan: () => void;
  onCopyImport?: () => void;
  onCopyFolder?: () => void;
  onDiscover: () => void;
  onHistory: () => void;
  onNearby: () => void;
  onExternalSelected: (book: Book, privacy: PrivacyContext) => void;
  /** One-shot native/mobile entry command: focus the library filter, focus the
   * archive selection, or open article search with a prefilled query. */
  command?: { id: number; kind: "search" | "scope"; query?: string; record?: boolean } | null;
  onCommandHandled?: (id: number) => void;
}
export default function Library({
  books,
  settings,
  importing,
  onOpen,
  onError,
  onRefresh,
  onImport,
  onScan,
  onCopyImport,
  onCopyFolder,
  onDiscover,
  onHistory,
  onNearby,
  onExternalSelected,
  command,
  onCommandHandled,
}: Props) {
  const [query, setQuery] = useState("");
  const [filterOpen, setFilterOpen] = useState(false);
  const [actionsOpen, setActionsOpen] = useState(false);
  const filterButton = useRef<HTMLButtonElement>(null);
  const filterInput = useRef<HTMLInputElement>(null);
  const scopeSection = useRef<HTMLDetailsElement>(null);
  const handledCommand = useRef(0);
  const [filters, setFilters] = useArchiveFilters(
    settings,
    "libraryFilters",
    onError,
  );
  const [availability, setAvailability] = useState("all");
  const [sort, setSort] = useState("title");
  const [direction, setDirection] = useState<"asc" | "desc">("asc");
  const [view, setView] = useState<"grid" | "list">("grid");
  const [pinBusy, setPinBusy] = useState(false);
  const pinPending = useRef(false);
  const [detail, setDetail] = useState<Book | null>(null);
  const detailRef = useRef(detail);
  detailRef.current = detail;
  const integrity = useIntegritySnapshot(settings.privateMode, settings.readOnly, onError);
  const [removal, setRemoval] = useState<{
    book: Book; deleteFile: boolean; message: string; privacy: PrivacyContext;
  } | null>(null);
  const [removalBusy, setRemovalBusy] = useState(false);
  const [removalError, setRemovalError] = useState("");
  const removalPending = useRef(false);
  const removalCancel = useRef<HTMLButtonElement>(null);
  const [sharing, setSharing] = useState<DownloadJob | null>(null);
  const [shareBusy, setShareBusy] = useState(false);
  const [shareError, setShareError] = useState("");
  const [shareCopied, setShareCopied] = useState(false);
  const shareRequest = useRef(0);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    setRemoval(null);
    setRemovalError("");
  }, [detail?.installationId, settings.privateMode, settings.readOnly]);
  useEffect(() => {
    shareRequest.current++;
    setSharing(null);
    setShareError("");
    setShareCopied(false);
    setShareBusy(false);
  }, [settings.privateMode, settings.readOnly]);
  useEffect(() => {
    if (removal) removalCancel.current?.focus({ preventScroll: true });
  }, [removal]);
  const [selected, setSelected] = useState<string[]>([]);
  const [externalPicker, setExternalPicker] = useState(false);
  const [externalBusy, setExternalBusy] = useState(false);
  const [externalError, setExternalError] = useState<string | null>(null);
  const [externalReplace, setExternalReplace] = useState<string | null>(null);
  const [deviceScan, setDeviceScan] = useState<DeviceScan | null>(null);
  const [deviceScanBusy, setDeviceScanBusy] = useState(false);
  const [deviceScanError, setDeviceScanError] = useState("");
  const [deviceScanOpen, setDeviceScanOpen] = useState(false);
  const [deviceOpenBusy, setDeviceOpenBusy] = useState<string | null>(null);
  useEffect(() => {
    setDeviceScan(null);
    setDeviceScanError("");
    setDeviceOpenBusy(null);
  }, [settings.privateMode, settings.readOnly]);
  async function fetchDeviceScan(privacy: PrivacyContext) {
    const result = await api<DeviceScan>("mobile_device_scan", {
      privacyGeneration: privacy.generation,
    });
    if (!isPrivacyPolicyContextCurrent(privacy)) return;
    setDeviceScan(result);
    setDeviceScanOpen(true);
  }
  async function scanDevice() {
    const privacy = capturePrivacyContext();
    if (!privacy || deviceScanBusy) return;
    setDeviceScanBusy(true);
    setDeviceScanError("");
    try {
      await fetchDeviceScan(privacy);
    } catch (error) {
      if (isPrivacyPolicyContextCurrent(privacy)) setDeviceScanError(errorText(error));
    } finally {
      setDeviceScanBusy(false);
    }
  }
  async function grantScanFolder() {
    const privacy = capturePrivacyContext();
    if (!privacy || deviceScanBusy) return;
    setDeviceScanBusy(true);
    setDeviceScanError("");
    try {
      const granted = await api<{ granted?: boolean } | null>(
        "mobile_device_grant_folder",
        { privacyGeneration: privacy.generation },
      );
      if (!granted || !isPrivacyPolicyContextCurrent(privacy)) return;
      await fetchDeviceScan(privacy);
    } catch (error) {
      if (isPrivacyPolicyContextCurrent(privacy)) setDeviceScanError(errorText(error));
    } finally {
      setDeviceScanBusy(false);
    }
  }
  async function openFound(entry: DeviceArchive) {
    const privacy = capturePrivacyContext();
    if (!privacy || deviceOpenBusy) return;
    setDeviceOpenBusy(entry.id);
    setDeviceScanError("");
    try {
      const book = await api<Book | null>("mobile_device_open_found", {
        token: entry.token,
        privacyGeneration: privacy.generation,
      });
      if (!book || !isPrivacyPolicyContextCurrent(privacy)) return;
      setDetail(null);
      onExternalSelected(book, privacy);
    } catch (error) {
      if (isPrivacyPolicyContextCurrent(privacy)) setDeviceScanError(errorText(error));
    } finally {
      setDeviceOpenBusy(null);
    }
  }
  async function openExternal(folder: boolean) {
    const privacy = capturePrivacyContext();
    if (!privacy || externalBusy) return;
    setExternalBusy(true);
    setExternalError(null);
    try {
      const book = await api<Book | null>("mobile_open_external_archive", {
        folder, privacyGeneration: privacy.generation, replaceId: externalReplace,
      });
      if (book) {
        setExternalPicker(false);
        setDetail(null);
        setExternalReplace(null);
        if (isPrivacyPolicyContextCurrent(privacy)) onExternalSelected(book, privacy);
      }
    } catch (error) {
      if (isPrivacyPolicyContextCurrent(privacy)) setExternalError(errorText(error));
    } finally { setExternalBusy(false); }
  }
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [speechAvailable, setSpeechAvailable] = useState(false);
  const [speechBusy, setSpeechBusy] = useState(false);
  // The recognizer is an installed activity, not a permission-gated in-app
  // recorder. Hide the action when package visibility finds no handler.
  useEffect(() => {
    if (!isAndroid) return;
    let cancelled = false;
    void api<{ available?: unknown }>("mobile_voice_search_available")
      .then((result) => {
        if (!cancelled) setSpeechAvailable(result?.available === true);
      })
      .catch(() => {
        if (!cancelled) setSpeechAvailable(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const [mode, setMode] = useState<"combined" | "title" | "fulltext">("combined");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [busy, setBusy] = useState(false);
  const [snippetSaving, setSnippetSaving] = useState(false);
  const searchId = useRef(0);
  const searchRequest = useRef<SearchRequest | null>(null);
  const searchError = useRef(onError);
  searchError.current = onError;
  const searchParameters = useRef<{
    parameters: {
      bookIds: string[];
      query: string;
      mode: "combined" | "title" | "fulltext";
    };
    privacy: PrivacyContext;
  } | null>(null);
  const [suggestions, setSuggestions] = useState<TitleSuggestion[]>([]);
  const [suggestionToken, setSuggestionToken] = useState<string | null>(null);
  const [suggestionOpen, setSuggestionOpen] = useState(false);
  const [searchInputFocused, setSearchFocused] = useState(false);
  const [searchSubmitted, setSearchSubmitted] = useState(false);
  const searchInput = useRef<HTMLInputElement>(null);
  // Suggestions follow the focused search field but never supersede an
  // explicit search: search and suggest share one request, so a suggestion
  // would cancel the running search, and a list opened over its results (or
  // its error) hides them. The next keystroke brings suggestions back.
  const searchFocused = searchInputFocused && !busy && !searchSubmitted;
  const [suggestionActive, setSuggestionActive] = useState(-1);
  const [suggestionLoading, setSuggestionLoading] = useState(false);
  const suggestionId = useRef(0);
  const suggestionPrefix = useRef("");
  const suggestionScope = useRef<string[]>([]);
  const suggestionMore = useRef(false);
  const bookTitles = useMemo(
    () => new Map(books.map((book) => [book.id, book.title])),
    [books],
  );
  function closeSuggestions() {
    suggestionId.current++;
    setSuggestions([]);
    setSuggestionToken(null);
    setSuggestionOpen(false);
    setSuggestionActive(-1);
    setSuggestionLoading(false);
  }
  function invalidateSearch() {
    searchId.current++;
    void searchRequest.current?.cancel().catch((error) => searchError.current(error));
    searchParameters.current = null;
    closeSuggestions();
    setResults(null);
    setBusy(false);
  }
  useEffect(() => {
    invalidateSearch();
  }, [settings.showSearchSnippets]);
  async function changeSearchSnippets(showSearchSnippets: boolean) {
    invalidateSearch();
    setSnippetSaving(true);
    try {
      await api("save_settings", { settings: { showSearchSnippets } });
      onRefresh();
    } catch (error) {
      onError(error);
    } finally {
      setSnippetSaving(false);
    }
  }
  useEffect(() => {
    const request = createSearchRequest();
    searchRequest.current = request;
    const unsubscribe = subscribeReaderPreferences(() => {
      const current = searchParameters.current;
      if (current && !isPrivacyContextCurrent(current.privacy)) {
        invalidateSearch();
      }
    });
    return () => {
      searchId.current++;
      if (searchRequest.current === request) searchRequest.current = null;
      void request.dispose().catch((error) => searchError.current(error));
      unsubscribe();
    };
  }, []);
  // A command can arrive on mount, so this runs after the effect that owns the
  // search request; otherwise its search would find no request and be dropped.
  useEffect(() => {
    if (!command || command.id <= handledCommand.current) return;
    handledCommand.current = command.id;
    if (command.kind === "search") {
      setFilterOpen(true);
      requestAnimationFrame(() => filterInput.current?.focus());
    } else {
      setSearchOpen(true);
      if (command.query) {
        setSearch(command.query);
        // A platform entry (PROCESS_TEXT or kiwix://search) arrives with the
        // query already chosen; run the same privacy-checked search a form
        // submit would instead of making the user press Enter again.
        // A desktop "Look up in Kiwix" request can come from any web page
        // (kiwix://search), so it runs without joining recent searches.
        void performSearch(command.query, command.record !== false);
      }
      requestAnimationFrame(() => {
        scopeSection.current?.scrollIntoView({ block: "nearest" });
        scopeSection.current?.querySelector("summary")?.focus();
      });
    }
    onCommandHandled?.(command.id);
  }, [command, onCommandHandled]);
  // Suggestions need an explicit per-archive scope: with no selection there is
  // no bounded suggestion source (search still covers all available archives).
  useEffect(() => {
    if (!searchOpen || !searchFocused) {
      closeSuggestions();
      return;
    }
    const prefix = search.trim();
    if (!prefix || !selected.length) {
      closeSuggestions();
      return;
    }
    closeSuggestions();
    // A search submitted before the debounce elapses retires this timer
    // through closeSuggestions, before React re-runs this effect.
    const scheduled = suggestionId.current;
    const timer = window.setTimeout(() => {
      if (scheduled === suggestionId.current) void loadSuggestions(prefix, [...selected]);
    }, SUGGESTION_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [search, selected, searchOpen, searchFocused]);
  const locale = getLocale();
  const languages = useMemo(
    () =>
      [...new Set(books.flatMap((b) => archiveLanguages(b.language)))].map(
        (value) => ({ value, label: languageName(value) }),
      ),
    [books, locale],
  );
  const categories = useMemo(
    () =>
      [...new Set(books.flatMap((b) => archiveCategories(b.tags)))].map(
        (value) => ({ value, label: value }),
      ),
    [books],
  );
  const hasActiveFilters = !!query || availability !== "all" ||
    filters.languages.length > 0 || filters.categories.length > 0 || filters.content.length > 0;
  function clearFilters() {
    setQuery("");
    setAvailability("all");
    setFilters({ languages: [], categories: [], content: [] });
  }
  const searchIndex = useMemo(
    () =>
      new Map(
        books.map((book) => [
          book.id,
          `${book.title} ${book.description} ${book.tags.join(" ")} ${book.creator} ${book.publisher} ${book.language}`
            .toLocaleLowerCase(),
        ]),
      ),
    [books],
  );
  const pinned = useMemo(
    () => usablePinnedBooks(settings.pinnedBookIds, books),
    [settings.pinnedBookIds, books],
  );
  const pinLimitReached = pinned.length >= MAX_PINNED_BOOKS;
  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const ordered = useMemo(
    () =>
      orderLibraryBooks(
        [...books].sort(
          (a, b) =>
            (direction === "asc" ? 1 : -1) * compareArchives(a, b, sort) ||
            a.id.localeCompare(b.id),
        ),
        pinned,
      ),
    [books, sort, direction, pinned, locale],
  );
  const needle = query.toLocaleLowerCase();
  const filtered = useMemo(
    () =>
      ordered.filter(
        (b) =>
          (!needle || (searchIndex.get(b.id) ?? "").includes(needle)) &&
          matchesArchiveFilters(b, filters) &&
          (availability === "all" ||
            b.available === (availability === "available")),
      ),
    [ordered, searchIndex, needle, filters, availability],
  );
  const open = (book: Book) =>
    onOpen({ bookId: book.id, path: "", title: book.title, scroll: 0 });
  async function setPinned(book: Book, nextPinned: boolean) {
    if (pinPending.current || settings.privateMode || settings.readOnly) return;
    // The backend owns the cap and the current pin order; this only avoids a
    // pointless round trip when this window already shows a full list.
    if (nextPinned && !pinnedSet.has(book.id) && pinLimitReached) return;
    pinPending.current = true;
    setPinBusy(true);
    try {
      await api("set_book_pinned", { bookId: book.id, pinned: nextPinned });
      onRefresh();
    } catch (error) {
      onError(error);
    } finally {
      pinPending.current = false;
      setPinBusy(false);
    }
  }
  /** The removal and detail dialogs paint `removalError` themselves; without
   * either open (a swipe from the list) the shell's surface is the one owner. */
  function removalFailed(error: unknown, shownInline: boolean) {
    if (!shownInline) {
      onError(error);
      return;
    }
    setRemovalError(errorText(error));
    onError(error, "blocking", false);
  }
  async function remove(book: Book, deleteFile: boolean) {
    const privacy = capturePrivacyContext();
    if (!privacy || removalPending.current ||
        (settings.readOnly && book.sourceReference?.persistent !== false) ||
        (deleteFile && (!book.available || !!book.sourceReference))) return;
    removalPending.current = true;
    setRemovalBusy(true);
    setRemovalError("");
    try {
      const paths = deleteFile
        ? await api<string[]>("archive_files", { id: book.id })
        : [];
      if (deleteFile && !paths.length)
        throw new Error(t("No archive files were found. Nothing was deleted."));
      const message = deleteFile
        ? (settings.trashDeletedFiles
            ? t(
                "Move “{title}” to the system trash and remove it from your library?",
                { title: book.title },
              )
            : t(
                "Permanently delete “{title}” and remove it from your library? This cannot be undone.",
                { title: book.title },
              )) +
          "\n\n" +
          t("These archive files will be affected:") +
          "\n" +
          (isIOS ? paths.map(pathFileName) : paths.map(displayPath)).join("\n")
        : t(
            "Remove “{title}” from your library? The archive file will remain on disk, and any open tab for it will close.",
            { title: book.title },
          );
      if (alive.current && isPrivacyContextCurrent(privacy))
        setRemoval({ book, deleteFile, message, privacy });
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(privacy))
        removalFailed(e, detailRef.current?.installationId === book.installationId);
    } finally {
      removalPending.current = false;
      if (alive.current) setRemovalBusy(false);
    }
  }
  async function confirmRemoval() {
    if (!removal || removalPending.current) return;
    if (!isPrivacyContextCurrent(removal.privacy)) {
      setRemoval(null);
      removalFailed(new Error(t("Browsing session expired after a privacy transition")),
        detailRef.current?.installationId === removal.book.installationId);
      return;
    }
    removalPending.current = true;
    setRemovalBusy(true);
    setRemovalError("");
    try {
      await api("remove_book", {
        id: removal.book.id, installationId: removal.book.installationId,
        deleteFile: removal.deleteFile, privacyGeneration: removal.privacy.generation,
      });
      if (!alive.current) return;
      setSelected((previous) => previous.filter((id) => id !== removal.book.id));
      setRemoval(null);
      setDetail(null);
      onRefresh();
    } catch (e) {
      // The removal dialog stays open and shows the failure.
      if (alive.current) removalFailed(e, true);
    } finally {
      removalPending.current = false;
      if (alive.current) setRemovalBusy(false);
    }
  }
  async function shareNearby(book: Book) {
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.privateMode || privacy.readOnly || shareBusy) return;
    const request = ++shareRequest.current;
    setShareBusy(true);
    try {
      await api("localswarm_seed", { bookId: book.id, privacyGeneration: privacy.generation });
      if (!alive.current || request !== shareRequest.current || !isPrivacyContextCurrent(privacy)) return;
      setDetail(null);
      onNearby();
    } catch (value) {
      if (alive.current && request === shareRequest.current && isPrivacyContextCurrent(privacy)) onError(value);
    } finally {
      if (alive.current && request === shareRequest.current) setShareBusy(false);
    }
  }
  async function shareBook(book: Book) {
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.privateMode || privacy.readOnly || settings.readOnly || shareBusy) return;
    const request = ++shareRequest.current;
    setShareBusy(true);
    setShareError("");
    setShareCopied(false);
    setSharing(null);
    // Once the share dialog is open it shows the failure itself.
    let shown = false;
    try {
      let job = await api<DownloadJob>("share_torrent", { bookId: book.id });
      if (!alive.current || request !== shareRequest.current || !isPrivacyContextCurrent(privacy)) return;
      setSharing(job);
      shown = true;
      setDetail(null);
      onRefresh();
      // Hashing a large archive can take far longer than any fixed wait, and
      // the worker keeps going: follow it until it settles. Closing the dialog,
      // a privacy change or leaving the library retires the poll; the accepted
      // torrent job keeps running server-side, but no invoke or state update
      // outlives this tree.
      while (!shareSettled(job)) {
        // Promise.withResolvers is unavailable in this project's TS lib target.
        await new Promise<void>((resolve) => window.setTimeout(resolve, 750));
        if (!alive.current || request !== shareRequest.current || !isPrivacyContextCurrent(privacy)) return;
        job = await api<DownloadJob>("torrent_job", { id: job.id });
        if (!alive.current || request !== shareRequest.current || !isPrivacyContextCurrent(privacy)) return;
        setSharing(job);
      }
      if (!job.magnet) {
        setShareError(job.error
          ? errorText(job.error)
          : t("Torrent metadata was not created. Try sharing again."));
      }
    } catch (e) {
      if (request === shareRequest.current && alive.current) {
        if (shown) {
          setShareError(errorText(e));
          onError(e, "blocking", false);
        } else onError(e);
      }
    } finally {
      if (request === shareRequest.current && alive.current) setShareBusy(false);
    }
  }
  /** A share has its magnet link or has stopped without one. */
  function shareSettled(job: DownloadJob) {
    return !!job.magnet || ["error", "completed", "paused", "cancelled"].includes(job.status);
  }
  async function toggleShare(job: DownloadJob) {
    const enabled = !job.seeding;
    const privacy = capturePrivacyContext();
    if (!privacy || settings.readOnly || !isPrivacyContextCurrent(privacy)) return;
    const request = shareRequest.current;
    setShareBusy(true);
    setShareError("");
    try {
      await api("torrent_seeding", { id: job.id, enabled });
      // The command only acknowledges, so read the state the manager now holds
      // instead of predicting what stopping or resuming did to the job.
      const updated = await api<DownloadJob>("torrent_job", { id: job.id });
      if (!alive.current || request !== shareRequest.current ||
        !isPrivacyContextCurrent(privacy)) return;
      setSharing((current) => current && current.id === updated.id ? updated : current);
      onRefresh();
    } catch (e) {
      if (alive.current && request === shareRequest.current) setShareError(errorText(e));
    } finally {
      if (alive.current && request === shareRequest.current) setShareBusy(false);
    }
  }
  async function copyMagnet(job: DownloadJob) {
    if (!job.magnet) return;
    try {
      await navigator.clipboard.writeText(job.magnet);
      if (alive.current) setShareCopied(true);
    } catch (e) {
      if (alive.current) setShareError(errorText(e));
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
      if (alive.current && isPrivacyContextCurrent(privacy)) setShareError(errorText(e));
    }
  }
  async function relink(book: Book) {
    if (isMobile || book.sourceReference) {
      setExternalReplace(book.id);
      setDetail(null);
      setExternalError(null);
      setExternalPicker(true);
      return;
    }
    try {
      const paths = await chooseFiles();
      if (paths[0]) {
        const replacement = await api<Book>("relink_book", {
          id: book.id,
          path: paths[0],
        });
        setDetail(replacement);
        onRefresh();
      }
    } catch (e) {
      onError(e);
    }
  }
  async function random(book: Book) {
    try {
      const location = await api<ReaderLocation>("random_article", {
        bookId: book.id,
      });
      onOpen({ bookId: location.bookId, path: location.path, title: location.title, scroll: 0 });
    } catch (e) {
      onError(e);
    }
  }
  async function runSearch(e: React.FormEvent) {
    e.preventDefault();
    await performSearch(search);
  }
  async function speakSearch() {
    if (speechBusy) return;
    const privacy = capturePrivacyContext();
    if (!privacy) return;
    setSpeechBusy(true);
    try {
      const result = await api<{ cancelled?: unknown; query?: unknown }>(
        "mobile_voice_search",
        { language: getLocale() },
      );
      // A privacy cutover while the recognizer was open retires its answer.
      if (!isPrivacyContextCurrent(privacy)) return;
      if (result?.cancelled === true) return;
      const query =
        typeof result?.query === "string"
          ? result.query.trim().slice(0, 4096)
          : "";
      if (!query) return;
      setSearch(query);
      setSearchOpen(true);
      await performSearch(query);
    } catch (error) {
      if (isPrivacyContextCurrent(privacy))
        onError(error, ERROR_SITES["library.speech-search"]);
    } finally {
      setSpeechBusy(false);
    }
  }
  async function performSearch(query: string, record = true) {
    if (!query.trim() || snippetSaving) return;
    closeSuggestions();
    const privacy = capturePrivacyContext();
    const request = searchRequest.current;
    if (!privacy || !request) return;
    const id = ++searchId.current;
    setSearchSubmitted(true);
    const parameters = { bookIds: [...selected], query, mode };
    searchParameters.current = { parameters, privacy };
    const isCurrent = () =>
      id === searchId.current && isPrivacyContextCurrent(privacy);
    setBusy(true);
    setResults(null);
    try {
      const found = await request.run({
        ...parameters,
        offset: 0,
        limit: 50,
      }, privacy);
      if (!isCurrent()) return;
      setResults(found);
      if (record) void api("record_search", {
        query: parameters.query,
        privacyGeneration: privacy.generation,
      }).catch((error) => {
        if (isCurrent()) onError(error, ERROR_SITES["library.record-search"]);
      });
    } catch (e) {
      if (isCurrent()) onError(e);
    } finally {
      if (isCurrent()) setBusy(false);
    }
  }
  async function moreResults() {
    const request = searchRequest.current;
    if (!results || !searchParameters.current || busy || !request) return;
    const { parameters, privacy } = searchParameters.current;
    if (!isPrivacyContextCurrent(privacy)) return;
    const id = searchId.current;
    const isCurrent = () =>
      id === searchId.current && isPrivacyContextCurrent(privacy);
    setBusy(true);
    try {
      const found = await request.run({
        ...parameters,
        offset: results.hits.length,
        limit: 50,
      }, privacy);
      if (isCurrent())
        setResults((previous) => {
          if (!previous) return found;
          const hits = [...previous.hits, ...found.hits].slice(0, MAX_SEARCH_HITS);
          return {
            ...found,
            hits,
            hasMore: found.hasMore && hits.length < MAX_SEARCH_HITS,
          };
        });
    } catch (e) {
      if (isCurrent()) onError(e);
    } finally {
      if (isCurrent()) setBusy(false);
    }
  }
  async function loadSuggestions(prefix: string, bookIds: string[]) {
    const request = searchRequest.current;
    const privacy = capturePrivacyContext();
    if (!request || !privacy || !prefix || !bookIds.length) {
      closeSuggestions();
      return;
    }
    const id = ++suggestionId.current;
    suggestionPrefix.current = prefix;
    suggestionScope.current = bookIds;
    suggestionMore.current = false;
    setSuggestionLoading(true);
    setSuggestionToken(null);
    setSuggestionActive(-1);
    try {
      const page = await request.suggest(
        { bookIds, prefix, limit: SUGGESTION_PAGE_SIZE },
        privacy,
      );
      if (id !== suggestionId.current) return;
      setSuggestions(page.suggestions);
      setSuggestionToken(page.nextToken);
      setSuggestionOpen(page.suggestions.length > 0);
    } catch (error) {
      // Superseded, retired, or expired requests never surface as errors: the
      // next keystroke starts a fresh suggestion list. Record the failure for
      // Troubleshooting without painting a band.
      diagnostics.record(error, ERROR_SITES["library.suggestion-refresh"]);
      if (id === suggestionId.current) {
        setSuggestions([]);
        setSuggestionToken(null);
        setSuggestionOpen(false);
      }
    } finally {
      if (id === suggestionId.current) setSuggestionLoading(false);
    }
  }
  async function loadMoreSuggestions() {
    const request = searchRequest.current;
    const privacy = capturePrivacyContext();
    const token = suggestionToken;
    if (!request || !privacy || !token || suggestionMore.current) return;
    const id = suggestionId.current;
    suggestionMore.current = true;
    setSuggestionLoading(true);
    try {
      const page = await request.suggest(
        {
          bookIds: suggestionScope.current,
          prefix: suggestionPrefix.current,
          limit: SUGGESTION_PAGE_SIZE,
          token,
        },
        privacy,
      );
      if (id !== suggestionId.current) return;
      setSuggestions((previous) => [...previous, ...page.suggestions]);
      setSuggestionToken(page.nextToken);
      setSuggestionOpen((open) => open || page.suggestions.length > 0);
    } catch (error) {
      diagnostics.record(error, ERROR_SITES["library.suggestion-refresh"]);
      if (id === suggestionId.current) setSuggestionToken(null);
    } finally {
      suggestionMore.current = false;
      if (id === suggestionId.current) setSuggestionLoading(false);
    }
  }
  function openSuggestion(suggestion: TitleSuggestion) {
    closeSuggestions();
    onOpen({
      bookId: suggestion.bookId,
      path: suggestion.path,
      title: suggestion.title,
      scroll: 0,
    });
  }
  function onSuggestionScroll(event: React.UIEvent<HTMLUListElement>) {
    const list = event.currentTarget;
    if (
      suggestionToken &&
      !suggestionLoading &&
      list.scrollHeight - list.scrollTop - list.clientHeight < 24
    ) {
      void loadMoreSuggestions();
    }
  }
  function onSearchKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Escape" && suggestionOpen) {
      event.preventDefault();
      event.stopPropagation();
      closeSuggestions();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (!suggestions.length) return;
      event.preventDefault();
      if (!suggestionOpen) {
        setSuggestionOpen(true);
        setSuggestionActive(
          event.key === "ArrowDown" ? 0 : suggestions.length - 1,
        );
        return;
      }
      const delta = event.key === "ArrowDown" ? 1 : -1;
      const next = suggestionActive + delta;
      if (next >= suggestions.length) {
        if (delta === 1 && suggestionToken) void loadMoreSuggestions();
        return;
      }
      if (next < 0) {
        setSuggestionActive(-1);
        return;
      }
      setSuggestionActive(next);
      document
        .getElementById(`library-suggestion-${next}`)
        ?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (event.key === "Enter" && suggestionOpen && suggestionActive >= 0) {
      const suggestion = suggestions[suggestionActive];
      if (suggestion) {
        event.preventDefault();
        openSuggestion(suggestion);
      }
    }
  }
  return (
    <section className="page library-page">
      <PageHeader
        title={t("Library")}
        description={books.length ? formatNumber(books.length) : undefined}
        actions={
          <>
            {(books.length > 0 || hasActiveFilters) && (
              <button
                ref={filterButton}
                className="icon-button"
                aria-label={t("Filter your library")}
                aria-expanded={filterOpen}
                aria-controls="library-filters"
                onClick={() => setFilterOpen(!filterOpen)}
              ><Search size={19} /></button>
            )}
            <button className="icon-button" aria-label={t("Library actions")}
              onClick={() => setActionsOpen(true)}>
              <MoreHorizontal size={21} />
            </button>
          </>
        }
      />
      <KokoroDownloadPrompt storageKey="kiwix-kokoro-first-launch-v1"
        enabled={!settings.privateMode && !settings.readOnly} />
      {pinLimitReached && (
        <p className="muted pin-limit" id="library-pin-limit">
          {t("You can pin up to {count} archives. Unpin one to pin another.", {
            count: MAX_PINNED_BOOKS,
          })}
        </p>
      )}
      {hasActiveFilters && (
        <div className="library-filter-status">
          <span>{t("{count} archives", { count: filtered.length })}</span>
          <button className="text-button" onClick={clearFilters}>{t("Clear filters")}</button>
        </div>
      )}
      {filterOpen && (
      <div id="library-filters" className="library-filter-panel">
      <div className="toolbar">
        <label className="search-field">
          <Search size={17} />
          <input
            ref={filterInput}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("Filter your library…")}
            aria-label={t("Filter your library")}
          />
        </label>
        <span className="filter-count">
          {t("{count} archives", { count: filtered.length })}
        </span>
        <select
          value={availability}
          onChange={(e) => setAvailability(e.target.value)}
          aria-label={t("Availability")}
        >
          <option value="all">{t("All archives")}</option>
          <option value="available">{t("Available")}</option>
          <option value="missing">{t("Missing files")}</option>
        </select>
        <label className="sort-field">
          <ArrowDownWideNarrow size={16} />
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value)}
            aria-label={t("Sort archives")}
          >
            {archiveSortFields().map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
            <option value="addedAt">{t("Added")}</option>
          </select>
        </label>
        <button
          onClick={() => setDirection(direction === "asc" ? "desc" : "asc")}
          aria-label={t("Sort direction")}
        >
          {direction === "asc" ? t("Ascending") : t("Descending")}
        </button>
        <div className="segmented">
          <button
            className={view === "grid" ? "active" : ""}
            onClick={() => setView("grid")}
            aria-label={t("Grid view")}
            aria-pressed={view === "grid"}
          >
            <Grid2X2 size={17} />
          </button>
          <button
            className={view === "list" ? "active" : ""}
            onClick={() => setView("list")}
            aria-label={t("List view")}
            aria-pressed={view === "list"}
          >
            <List size={17} />
          </button>
        </div>
      </div>
      <ArchiveFilterBar
        filters={filters}
        onChange={setFilters}
        languages={languages}
        categories={categories}
      />
        <button className="library-filters-done" onClick={() => {
          setFilterOpen(false);
          filterButton.current?.focus();
        }}>{t("Done")}</button>
      </div>
      )}
      {isAndroid && (
        <details
          className="device-scan panel"
          open={deviceScanOpen}
          onToggle={(event) => setDeviceScanOpen(event.currentTarget.open)}
        >
          <summary>
            {t("On this device")}
            {deviceScan && <> · {t("{count} archives", { count: deviceScan.entries.length })}</>}
          </summary>
          <div className="device-scan-body">
            <p className="muted">
              {t("Find ZIM files already on this device. Files open where they live; nothing is copied into the app unless you choose Import a copy.")}
            </p>
            {settings.privateMode ? (
              <p className="notice" role="status">
                {t("Device scanning is unavailable in private mode. Leave private mode to look for files on this device.")}
              </p>
            ) : (
              <>
                <div className="toolbar compact-toolbar">
                  <button
                    className="primary"
                    disabled={deviceScanBusy || importing}
                    onClick={() => void scanDevice()}
                  >
                    {deviceScanBusy ? t("Scanning…") : t("Scan device")}
                  </button>
                  <button
                    disabled={deviceScanBusy || importing}
                    onClick={() => void grantScanFolder()}
                  >
                    {t("Add folder access")}
                  </button>
                </div>
                {settings.readOnly && (
                  <p className="muted">
                    {t("This session only: folder access is not saved in read-only mode.")}
                  </p>
                )}
                {deviceScanError && (
                  <div className="inline-error" role="alert">
                    <p>{deviceScanError}</p>
                    <p>{t("If this location cannot be read in place, choose the original with Add files or Add folder, or explicitly choose Import a managed copy.")}</p>
                  </div>
                )}
                {deviceScan && !!deviceScan.entries.length && (
                  <ul className="device-archives">
                    {deviceScan.entries.map((entry) => (
                      <li key={entry.id} className={entry.readable ? "" : "unavailable"}>
                        <div className="device-archive-copy">
                          <strong>{entry.name}</strong>
                          <p className="muted">
                            {formatBytes(entry.size)}{" "}
                            <span aria-hidden="true">·</span> {deviceVolume(entry.volume)}
                            {entry.location && (
                              <> <span aria-hidden="true">·</span> {entry.location}</>
                            )}
                          </p>
                          <div className="device-archive-badges">
                            {entry.sources.map((source) => (
                              <span key={source} className="badge">
                                {source === "tree"
                                  ? t("Granted folder")
                                  : source === "storage"
                                    ? t("Device storage")
                                    : t("MediaStore")}
                              </span>
                            ))}
                            {entry.split && <span className="badge">{t("Split archive")}</span>}
                            {entry.inLibrary && <span className="badge">{t("In library")}</span>}
                            {(!entry.readable || entry.problem) && (
                              <span className="badge warning">{t("Unreadable")}</span>
                            )}
                          </div>
                          {entry.problem && <p className="muted">{entry.problem}</p>}
                        </div>
                        <div className="device-archive-actions">
                          <button
                            className="primary"
                            disabled={!entry.readable || deviceOpenBusy === entry.id}
                            onClick={() => void openFound(entry)}
                          >
                            {deviceOpenBusy === entry.id ? t("Opening…") : t("Open in place")}
                          </button>
                          {(!entry.readable || entry.problem) && onCopyImport && (
                            <button disabled={settings.privateMode || settings.readOnly || importing} onClick={onCopyImport}>
                              {t("Import a copy")}
                            </button>
                          )}
                        </div>
                      </li>
                    ))}
                  </ul>
                )}
                {deviceScan && !deviceScan.entries.length && (
                  <p className="muted">
                    {t("No ZIM files were found in the locations this app can read.")}
                  </p>
                )}
                {deviceScan && !!deviceScan.limitations.length && (
                  <ul className="device-scan-limitations muted">
                    {deviceScan.limitations.map((limitation) => (
                      <li key={limitation}>{limitation}</li>
                    ))}
                  </ul>
                )}
              </>
            )}
          </div>
        </details>
      )}
      {!books.length ? (
        <Empty
          title={t("Welcome to Kiwix")}
          description={t(
            "Open a ZIM archive from this device, or fetch one from the catalog. Everything you add stays readable offline, and this welcome disappears as soon as your library has an archive.",
          )}
        >
          <button
            className="primary"
            disabled={(!isMobile && settings.readOnly) || importing}
            onClick={onImport}
          >
            <FolderOpen size={16} />
            {t("Open file")}
          </button>
          <button onClick={onDiscover}>{t("Browse catalog")}</button>
        </Empty>
      ) : !filtered.length ? (
        <Empty
          title={t("No archives match")}
          description={t("Try another search or adjust your filters.")}
        />
      ) : (
        <PagedResults items={filtered} criteria={JSON.stringify([query, filters, availability, sort, direction, locale])}
          label={t("Library")} className={`book-collection ${view}`}>
          {(book) => (
            <SwipeRow
              key={`${book.id}:${book.installationId}`}
              actionLabel={t("Remove from library")}
              onAction={() => void remove(book, false)}
              disabled={removalBusy || !!removal || (settings.readOnly && book.sourceReference?.persistent !== false)}
            >
            <article
              data-swipe-interactive
              className={`book-card ${!book.available ? "unavailable" : ""}`}
              onClick={(event) => {
                if (
                  !(event.target as HTMLElement).closest("button, input, label")
                )
                  book.available ? open(book) : relink(book);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setDetail(book);
              }}
            >
              <div className="book-card-cover">
                <button
                  className="cover-open"
                  onClick={() => (book.available ? open(book) : relink(book))}
                  aria-label={
                    book.available
                      ? t("Open {title}", { title: book.title })
                      : t("Locate {title}", { title: book.title })
                  }
                >
                  <BookCover book={book} />
                </button>
                {(searchOpen || selected.length > 0) && (
                <label className="book-select">
                  <input
                    type="checkbox"
                    checked={selected.includes(book.id)}
                    onChange={(e) => {
                      setSelected((previous) =>
                        e.target.checked
                          ? [...previous, book.id]
                          : previous.filter((id) => id !== book.id),
                      );
                      invalidateSearch();
                    }}
                    aria-label={t("Include {title} in search", {
                      title: book.title,
                    })}
                  />
                </label>
                )}
              </div>
              <div className="book-card-body">
                <button
                  className="text-button book-title"
                  onClick={() => (book.available ? open(book) : relink(book))}
                >
                  {book.title}
                </button>
                <p className="book-description">{book.description}</p>
                <p className="book-card-meta">
                  {languageName(book.language)}{" "}
                  <span aria-hidden="true">·</span> {formatBytes(book.size)}
                </p>
                <IntegrityBadge book={book} snapshot={integrity.snapshot} />
                <div className="book-card-footer">
                  <div className="book-card-badges">
                    {pinnedSet.has(book.id) && (
                      <span className="badge pinned">
                        <Pin size={11} aria-hidden="true" />
                        {t("Pinned")}
                      </span>
                    )}
                    {!book.available && <span className="badge warning">{t("File missing")}</span>}
                  </div>
                  <div className="book-card-actions">
                    <button
                      className={`icon-button pin-button${pinnedSet.has(book.id) ? " active" : ""}`}
                      aria-label={pinnedSet.has(book.id)
                        ? t("Unpin {title} from the top of your library", { title: book.title })
                        : t("Pin {title} to the top of your library", { title: book.title })}
                      aria-pressed={pinnedSet.has(book.id)}
                      aria-disabled={!pinnedSet.has(book.id) && pinLimitReached ? true : undefined}
                      aria-describedby={!pinnedSet.has(book.id) && pinLimitReached
                        ? "library-pin-limit"
                        : undefined}
                      title={!pinnedSet.has(book.id) && pinLimitReached
                        ? t("You can pin up to {count} archives. Unpin one to pin another.", {
                            count: MAX_PINNED_BOOKS,
                          })
                        : pinnedSet.has(book.id)
                          ? t("Unpin {title} from the top of your library", { title: book.title })
                          : t("Pin {title} to the top of your library", { title: book.title })}
                      disabled={pinBusy || settings.privateMode || settings.readOnly}
                      onClick={(event) => {
                        event.stopPropagation();
                        void setPinned(book, !pinnedSet.has(book.id));
                      }}
                    >
                      <Pin
                        size={17}
                        fill={pinnedSet.has(book.id) ? "currentColor" : "none"}
                        aria-hidden="true"
                      />
                    </button>
                    <button
                      className="icon-button"
                      aria-label={t("Details and actions for {title}", {
                        title: book.title,
                      })}
                      title={t("Details and actions for {title}", {
                        title: book.title,
                      })}
                      onClick={() => {
                        setDetail(book);
                      }}
                    >
                      <Info size={19} />
                    </button>
                  </div>
                </div>
              </div>
            </article>
            </SwipeRow>
          )}
        </PagedResults>
      )}
      {!!books.length && (
        <details ref={scopeSection} className="search-archives secondary-controls panel" open={searchOpen}
          onToggle={(event) => {
            if (busy && !event.currentTarget.open) {
              event.currentTarget.open = true;
              return;
            }
            setSearchOpen(event.currentTarget.open);
          }}>
          <summary>
            {t("Search inside your archives")}
            {busy ? <> · {t("Searching…")}</> : results && <> · {t("About {count} results", { count: results.total })}</>}
          </summary>
          <div className="section-heading">
            <div>
              <p>
                {selected.length
                  ? t("Searching {count} selected archives", {
                      count: selected.length,
                    })
                  : t("Search all available archives, or select archives above.")}
              </p>
            </div>
            {!!selected.length && (
              <div className="toolbar compact-toolbar">
                <button onClick={() => {
                  setSelected([]);
                  invalidateSearch();
                }}>
                  {t("Clear selection")}
                </button>
                <SelectedStorageActions
                  books={books.filter((book) => selected.includes(book.id))}
                  disabled={settings.readOnly || settings.privateMode || importing || removalBusy}
                  onChanged={() => {
                    setSelected([]);
                    invalidateSearch();
                    onRefresh();
                  }}
                  onError={(error) => onError(error)}
                />
              </div>
            )}
          </div>
          <form className="toolbar compact-toolbar" onSubmit={runSearch}>
            <div className="search-combo">
              <label className="search-field">
                <Search size={17} />
                <input
                  ref={searchInput}
                  value={search}
                  onChange={(e) => {
                    setSearch(e.target.value);
                    setSearchSubmitted(false);
                    invalidateSearch();
                  }}
                  onKeyDown={onSearchKeyDown}
                  onFocus={() => setSearchFocused(true)}
                  onBlur={() => {
                    setSearchFocused(false);
                    closeSuggestions();
                  }}
                  placeholder={t("Search articles")}
                  aria-label={t("Search articles")}
                  role="combobox"
                  aria-autocomplete="list"
                  aria-expanded={suggestionOpen}
                  aria-controls={suggestionOpen ? "library-search-suggestions" : undefined}
                  aria-activedescendant={
                    suggestionOpen && suggestionActive >= 0
                      ? `library-suggestion-${suggestionActive}`
                      : undefined
                  }
                />
              </label>
              {suggestionOpen && (
                <ul
                  id="library-search-suggestions"
                  className="suggestion-list"
                  role="listbox"
                  aria-label={t("Article titles")}
                  onScroll={onSuggestionScroll}
                >
                  {suggestions.map((suggestion, index) => (
                    <li
                      key={`${suggestion.bookId}:${suggestion.path}`}
                      id={`library-suggestion-${index}`}
                      role="option"
                      aria-selected={index === suggestionActive}
                      className={index === suggestionActive ? "active" : undefined}
                      onPointerDown={(event) => {
                        // Keep DOM focus on the combobox through a tap, including
                        // touch/pen. Scrolling the list still cancels the click.
                        if (event.isPrimary && event.button === 0) event.preventDefault();
                      }}
                      onMouseEnter={() => setSuggestionActive(index)}
                      onClick={() => openSuggestion(suggestion)}
                    >
                      <BookOpen size={15} aria-hidden="true" />
                      <span className="suggestion-title">{suggestion.title}</span>
                      <span className="suggestion-book">
                        {bookTitles.get(suggestion.bookId)}
                      </span>
                    </li>
                  ))}
                  {suggestionLoading && (
                    <li className="suggestion-status" role="presentation">
                      {t("Loading…")}
                    </li>
                  )}
                </ul>
              )}
            </div>
            <select
              value={mode}
              onChange={(e) => {
                setMode(e.target.value as typeof mode);
                invalidateSearch();
              }}
              aria-label={t("Search mode")}
            >
              <option value="combined">{t("Article titles")} + {t("Full text")}</option>
              <option value="fulltext">{t("Full text")}</option>
              <option value="title">{t("Article titles")}</option>
            </select>
            <label>
              <input
                type="checkbox"
                checked={settings.showSearchSnippets}
                disabled={settings.readOnly || snippetSaving}
                onChange={(event) => void changeSearchSnippets(event.target.checked)}
              />
              {t("Show search snippets")}{" "}
              <small>{t("Changes apply to your next search.")}</small>
            </label>
            <button className="primary" disabled={busy || snippetSaving || !search.trim()}>
              {t("Search")}
            </button>
            {speechAvailable && (
              <button
                type="button"
                disabled={busy || speechBusy}
                aria-label={t("Speak to search")}
                title={t("Speak to search")}
                onClick={() => void speakSearch()}
              >
                <Mic size={16} aria-hidden="true" />{" "}
                {speechBusy ? t("Listening…") : t("Speak to search")}
              </button>
            )}
            {busy && (
              <button type="button" onClick={invalidateSearch}>{t("Cancel")}</button>
            )}
          </form>
          {busy && <Loading />}
          {results && (
            <div aria-live="polite">
              <p className="muted">
                {t("About {count} results", { count: results.total })}
                {results.total > results.hits.length &&
                  " · " +
                    t("Showing the first {count}", {
                      count: results.hits.length,
                    })}
              </p>
              {results.errors.map((error) => (
                <p className="inline-error" key={error.bookId}>
                  {bookTitles.get(error.bookId)}:{" "}
                  {errorText(error.error)}
                </p>
              ))}
              {results.hits.map((hit, i) => (
                <button
                  className="search-result"
                  key={`${hit.bookId}:${hit.path}:${i}`}
                  onClick={() => onOpen({ bookId: hit.bookId, path: hit.path, title: hit.title, scroll: 0 })}
                >
                  <strong>{hit.title}</strong>
                  <span>{bookTitles.get(hit.bookId)}</span>
                  {hit.snippet && <p>{hit.snippet}</p>}
                </button>
              ))}
              {!results.hits.length && (
                <p>{t("No matching articles found.")}</p>
              )}
              {!results.hits.length && !!results.corrections?.length && (
                <div className="toolbar compact-toolbar" role="group" aria-label={t("Spelling suggestions")}>
                  {results.corrections.map((correction) => (
                    <button type="button" key={correction.correctedQuery} disabled={busy || snippetSaving}
                      onClick={() => {
                        // The results, this button included, are replaced:
                        // keep focus on the field that now holds the query.
                        searchInput.current?.focus();
                        setSearch(correction.correctedQuery);
                        void performSearch(correction.correctedQuery);
                      }}>
                      {t("Search instead for “{query}”", { query: correction.correctedQuery })}
                    </button>
                  ))}
                </div>
              )}
              {results.hasMore && (
                <button disabled={busy} onClick={moreResults}>
                  {busy ? t("Loading…") : t("Show more results")}
                </button>
              )}
            </div>
          )}
        </details>
      )}
      {actionsOpen && (
        <Modal title={t("Library actions")} onClose={() => setActionsOpen(false)}>
          <div className="library-actions-list">
            <button onClick={() => {
              setActionsOpen(false);
              onHistory();
            }}><Clock size={17} />{t("Recently read")}</button>
            <button disabled={(!isMobile && settings.readOnly) || importing} onClick={() => {
              setActionsOpen(false);
              onImport();
            }}><Plus size={17} />{t("Add files")}</button>
            <button disabled={(!isMobile && settings.readOnly) || importing} onClick={() => {
              setActionsOpen(false);
              onScan();
            }}><FolderOpen size={17} />{t("Add folder")}</button>
            {isMobile && onCopyImport && <button disabled={settings.privateMode || settings.readOnly || importing} onClick={() => {
              setActionsOpen(false);
              onCopyImport();
            }}><Copy size={17} />{t("Import a managed copy")}</button>}
            {isMobile && onCopyFolder && <button disabled={settings.privateMode || settings.readOnly || importing} onClick={() => {
              setActionsOpen(false);
              onCopyFolder();
            }}><Copy size={17} />{t("Import a folder copy")}</button>}
            {isMobile && <button onClick={() => {
              setActionsOpen(false);
              setExternalReplace(null);
              setExternalError(null);
              setExternalPicker(true);
            }}><BookOpen size={17} />{t("Open a file")}</button>}
          </div>
        </Modal>
      )}
      {externalPicker && (
        <Modal title={t("Open external archive")} onClose={() => { if (!externalBusy) setExternalPicker(false); }}>
          <p>{t("Read a ZIM directly from USB, removable storage or a document provider without copying it into this app.")}</p>
          <p className="muted">{t("Keep the drive connected and the provider available while reading. Removing the library reference never deletes the external original.")}</p>
          {(settings.privateMode || settings.readOnly) && <p className="notice" role="status">{t("This session only: reading permission and the library reference are not saved in private or read-only mode.")}</p>}
          <p className="muted">{t("For a split ZIM, explicitly select its folder. The folder must contain only one complete archive; access is limited to its immediate archive parts.")}</p>
          {externalError && <div className="inline-error" role="alert">
            <p>{externalError}</p>
            <p>{t("Direct reading is unavailable. Reconnect the drive, restore provider access, or explicitly import a managed copy instead. Providers that offer only streams cannot be opened directly.")}</p>
          </div>}
          <div className="modal-actions">
            <button className="primary" disabled={externalBusy} onClick={() => void openExternal(false)}>{externalBusy ? t("Opening…") : t("Choose ZIM file")}</button>
            <button disabled={externalBusy} onClick={() => void openExternal(true)}>{t("Choose split archive folder")}</button>
            {onCopyImport && <button disabled={externalBusy || importing || settings.privateMode || settings.readOnly} onClick={() => { setExternalPicker(false); onCopyImport(); }}>{t("Import a managed copy")}</button>}
          </div>
        </Modal>
      )}
      {removal && (
        <Modal title={t("Remove from library")}
          initialFocus={removalCancel}
          describedBy="archive-removal-message"
          presentation="confirmation"
          onClose={() => {
          if (!removalPending.current) { setRemoval(null); setRemovalError(""); }
        }}>
          {removalError && <ErrorNotice message={removalError} />}
          {removalBusy && <Loading label={t("In progress")} />}
            <section aria-label={t("Confirmation required")}>
              <p className="archive-removal-message" id="archive-removal-message">{removal.message}</p>
              <div className="modal-actions archive-removal-actions">
                <button ref={removalCancel} disabled={removalBusy} onClick={() => { setRemoval(null); setRemovalError(""); }}>{t("Cancel")}</button>
                <button className={removal.deleteFile ? "danger" : "primary"} disabled={removalBusy}
                  onClick={() => void confirmRemoval()}>
                  {removal.deleteFile
                    ? settings.trashDeletedFiles ? t("Move archive to trash") : t("Permanently delete archive")
                    : t("Remove from library")}
                </button>
              </div>
            </section>
        </Modal>
      )}
      {detail && !removal && (
        <Modal title={detail.title} onClose={() => { if (!removalPending.current) setDetail(null); }}>
          {removalError && <ErrorNotice message={removalError} />}
          {removalBusy && <Loading label={t("In progress")} />}
          <div className="archive-detail-actions">
            {detail.available ? (
              <button className="primary" disabled={removalBusy} onClick={() => open(detail)}>
                <BookOpen size={16} />{t("Read archive")}
              </button>
            ) : (
              <button className="primary" disabled={removalBusy} onClick={() => relink(detail)}>
                <FolderOpen size={16} />{t("Locate missing archive")}
              </button>
            )}
            <button disabled={removalBusy || shareBusy || settings.privateMode || settings.readOnly || !detail.available}
              onClick={() => void shareNearby(detail)}>
              <Share2 size={16} />{shareBusy ? t("Working…") : t("Share")}
            </button>
            <button disabled={removalBusy || (settings.readOnly && detail.sourceReference?.persistent !== false)}
              onClick={() => void remove(detail, false)}>
              {t("Remove from library")}
            </button>
            <button
              disabled={removalBusy || shareBusy || settings.privateMode || settings.readOnly || !detail.available}
              onClick={() => void shareBook(detail)}
            >
              <Share2 size={16} />{t("Share as BitTorrent")}
            </button>
            <button
              aria-pressed={pinnedSet.has(detail.id)}
              aria-disabled={!pinnedSet.has(detail.id) && pinLimitReached ? true : undefined}
              aria-describedby={!pinnedSet.has(detail.id) && pinLimitReached
                ? "library-pin-limit"
                : undefined}
              disabled={pinBusy || settings.privateMode || settings.readOnly}
              onClick={() => void setPinned(detail, !pinnedSet.has(detail.id))}
            >
              <Pin
                size={16}
                fill={pinnedSet.has(detail.id) ? "currentColor" : "none"}
                aria-hidden="true"
              />
              {pinnedSet.has(detail.id) ? t("Unpin from top") : t("Pin to top")}
            </button>
          </div>
          {!pinnedSet.has(detail.id) && pinLimitReached && (
            <p className="muted" role="status">
              {t("You can pin up to {count} archives. Unpin one to pin another.", {
                count: MAX_PINNED_BOOKS,
              })}
            </p>
          )}
          <SelectedStorageActions
            books={[detail]}
            disabled={settings.readOnly || settings.privateMode || importing || removalBusy}
            onChanged={() => {
              setDetail(null);
              setSelected([]);
              invalidateSearch();
              onRefresh();
            }}
            onError={(error) => onError(error)}
          />
          <p className="detail-description">{detail.description}</p>
          <BookFacts book={detail} />
          <EditionBadges book={detail} />
          <dl className="facts">
            <div>
              <dt>{t("Language")}</dt>
              <dd>{languageName(detail.language)}</dd>
            </div>
            <div>
              <dt>{t("Added")}</dt>
              <dd>{formatDate(detail.addedAt)}</dd>
            </div>
            <div>
              <dt>{t("File")}</dt>
              <dd className="file-path">{isIOS
                ? pathFileName(detail.sourceReference?.displayName ?? detail.path)
                : detail.sourceReference?.displayName ?? displayPath(detail.path)}</dd>
            </div>
            <div>
              <dt>{t("Search indexes")}</dt>
              <dd>
                {[
                  detail.hasTitleIndex ? t("Titles") : null,
                  detail.hasFulltextIndex ? t("Full text") : null,
                ]
                  .filter(Boolean)
                  .join(" · ") || t("None")}
              </dd>
            </div>
          </dl>
          <ArchiveTags tags={detail.tags} />
          {detail.available && (
            <>
              <p className="muted">
                {t(
                  "Archives can run JavaScript and may request camera or location. If you do not trust this source, open it in restricted mode; some apps and maps will not work.",
                )}
              </p>
              <div className="modal-actions">
                <button
                  onClick={() => onOpen({
                    bookId: detail.id,
                    path: "",
                    title: detail.title,
                    scroll: 0,
                    restricted: true,
                  })}
                >
                  <ShieldAlert size={16} />{t("Open in restricted mode")}
                </button>
                <button onClick={() => random(detail)}>
                  <Shuffle size={16} />{t("Random article")}
                </button>
                {!isMobile && (
                  <button onClick={() => { void api("reveal_book", { id: detail.id }).catch(onError); }}>
                    <FolderOpen size={16} />{t("Reveal containing folder")}
                  </button>
                )}
              </div>
            </>
          )}
          <IntegrityPanel book={detail} books={books} snapshot={integrity.snapshot} refresh={integrity.refresh} privateMode={settings.privateMode} readOnly={settings.readOnly} onError={onError} />
          {detail.tags.some((tag) => tag.trim().toLowerCase() === "_sw:yes") && (
            <p className="inline-error" role="status">{t("This archive requires legacy Service Worker replay, which this reader does not support. Open it with another compatible reader.")}</p>
          )}
          <div className="destructive-actions">
            {!detail.sourceReference && (detail.managed ? (
              <button
                className="danger"
                disabled={settings.readOnly || !detail.available}
                onClick={() => remove(detail, true)}
              >
                <Trash2 size={16} />
                {settings.trashDeletedFiles
                  ? t("Move archive to trash")
                  : t("Permanently delete archive")}
              </button>
            ) : (
              <p className="muted" role="note">
                {t("This archive is outside Kiwix's storage, so it cannot be deleted here. Remove it from your library, then delete the original file in your file manager.")}
              </p>
            ))}
          </div>
        </Modal>
      )}
      {sharing && (
        <Modal title={t("Share as BitTorrent")} onClose={() => {
          shareRequest.current++;
          setSharing(null);
          setShareError("");
          setShareCopied(false);
          setShareBusy(false);
        }}>
          {shareError && <ErrorNotice message={shareError} />}
          {!sharing.magnet ? (
            !shareError && !shareSettled(sharing) && (
              <Loading label={sharing.status === "creating"
                ? t("Creating torrent metadata… Large archives can take several minutes.")
                : t("Checking the shared item…")} />
            )
          ) : (
            <>
              <p className="muted">
                {t("Anyone with this magnet link or .torrent file can download the archive directly from this device while sharing is active. The library file is never modified.")}
              </p>
              <label>
                <span>{t("Magnet link")}</span>
                <textarea
                  className="share-magnet"
                  dir="ltr"
                  readOnly
                  rows={3}
                  value={sharing.magnet}
                  onFocus={(event) => event.currentTarget.select()}
                />
              </label>
              <div className="modal-actions">
                <button type="button" disabled={shareBusy} onClick={() => void copyMagnet(sharing)}>
                  {shareCopied ? <Check size={16} /> : <Copy size={16} />}
                  {shareCopied ? t("Copied") : t("Copy magnet link")}
                </button>
                <button type="button" disabled={shareBusy} onClick={() => void saveTorrent(sharing)}>
                  <Share2 size={16} />{t("Save .torrent file")}
                </button>
                <button
                  type="button"
                  className={sharing.seeding ? "danger" : "primary"}
                  disabled={shareBusy}
                  onClick={() => void toggleShare(sharing)}
                >
                  {sharing.seeding ? t("Stop sharing") : t("Share again")}
                </button>
              </div>
              <p className="muted">
                {sharing.seeding
                  ? t("Sharing now. Uploaded: {size}", { size: formatBytes(sharing.uploaded || 0) })
                  : t("Sharing is stopped. The archive remains in your library.")}
              </p>
            </>
          )}
        </Modal>
      )}
    </section>
  );
}
