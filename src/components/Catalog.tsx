import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine,
  ArrowUp,
  BookOpen,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  Grid2X2,
  List,
  Search,
} from "lucide-react";
import { api, capturePrivacyContext, getConfirmedSettings, isPrivacyContextCurrent } from "../api";
import {
  t,
  formatBytes,
  formatDate,
  formatNumber,
  languageName,
  getLocale,
} from "../i18n";
import type { Book, CatalogCacheStatus, CatalogEntry, CatalogPage, DownloadAdmission, DownloadJob, DownloadTarget, Settings } from "../types";
import { deviceCatalogLanguages } from "../catalog-languages";
import {
  BookCover,
  Empty,
  ErrorNotice,
  Loading,
  Modal,
  PageHeader,
  confirmAction,
  errorText,
  ArchiveFilterBar,
  BookFacts,
  EditionBadges,
  archiveSortFields,
  compareArchives,
  emptyArchiveFilters,
  useArchiveFilters,
  type ArchiveFilters,
  type FacetChoice,
} from "./UI";
interface Props {
  settings: Settings;
  privacyGeneration: number;
  books: Book[];
  onError: (e: unknown) => void;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
  onOpenBook: (book: Book) => void;
}
type CatalogFacets = {
  languages: FacetChoice[];
  categories: FacetChoice[];
  cache?: CatalogCacheStatus;
};
function catalogURL(source: string) {
  const url = new URL(source || "https://library.kiwix.org/catalog/v2/entries");
  url.searchParams.set("count", "48");
  return url.href;
}

// Only producer metadata declares this requirement. Names, URLs, and the
// Zimit scraper family do not establish that an archive needs legacy replay.
function requiresLegacyServiceWorker(entry: CatalogEntry): boolean {
  return entry.tags.some((tag) => tag.trim().toLowerCase() === "_sw:yes");
}
function archiveName(value: string): string {
  return value.trim().replace(/\.zim$/i, "").toLowerCase();
}
/// The catalog entry a library book already satisfies, if any. An exact
/// name+date match is the installed edition; another date of the same name is
/// an older or newer edition, never a duplicate of this one.
/// `installed` is built once per library, so resolving a card is a map lookup
/// instead of a full library scan with a regex per book.
function installedEdition(
  entry: CatalogEntry,
  installed: Map<string, Book[]>,
): { book: Book; exact: boolean } | null {
  const name = archiveName(entry.name);
  if (!name) return null;
  const matching = installed.get(name);
  if (!matching || !matching.length) return null;
  const exact = matching.find((book) => book.date === entry.date);
  if (exact) return { book: exact, exact: true };
  return {
    book: [...matching].sort((a, b) => b.date.localeCompare(a.date))[0],
    exact: false,
  };
}
function CompatibilityWarning({ entry }: { entry: CatalogEntry }) {
  return requiresLegacyServiceWorker(entry) ? (
    <p className="warning-text" role="note">
      {t(
        "This archive requires legacy Service Worker replay, which this reader does not support. Download it only for use with another compatible reader.",
      )}
    </p>
  ) : null;
}
function CatalogCacheNotice({
  cache,
  facets = false,
  temporary,
  onRetry,
}: {
  cache?: CatalogCacheStatus;
  facets?: boolean;
  temporary: boolean;
  onRetry: () => void;
}) {
  if (!cache) return null;
  const stale = cache.stale || cache.status === "offline";
  return (
    <p className={stale || cache.error ? "warning-text" : "muted"} role="status">
      {!stale && !cache.error && (
        facets
          ? (cache.status === "revalidated" ? t("Catalog filter choices revalidated.") : t("Catalog filter choices refreshed."))
          : (cache.status === "revalidated" ? t("This catalog page was revalidated.") : t("This catalog page was refreshed."))
      )}
      {stale &&
        (facets
          ? t("Could not refresh catalog filter choices. Showing cached choices that may be out of date.")
          : t("Could not refresh the catalog. Showing a cached page that may be out of date."))}
      {!stale && cache.error &&
        (facets ? t("Catalog filter cache warning:") : t("Catalog cache warning:"))}
      {!cache.stored && (
        <>
          {" "}
          {facets
            ? t("These filter choices are not saved for offline browsing.")
            : t("This catalog page is not saved for offline browsing.")}
          {temporary && <>{" "}{t("Results are temporary in private or read-only mode.")}</>}
        </>
      )}
      {cache.error && <>{" "}{errorText(cache.error)}</>}
      {(stale || cache.error) && (
        <>
          {" "}
          <button type="button" onClick={onRetry}>{t("Try again")}</button>
        </>
      )}
    </p>
  );
}
export default function Catalog(props: Props) {
  // Retire query, pagination, and pending navigation before a new mode can
  // automatically fetch and persist the previous private browsing state.
  return (
    <CatalogSession
      key={`${props.settings.privateMode}:${props.settings.readOnly}`}
      {...props}
    />
  );
}
function CatalogSession({ settings, privacyGeneration, onError, onDownload, books, onOpenBook }: Props) {
  const [query, setQuery] = useState("");
  const [filters, setStoredFilters] = useArchiveFilters(
    settings,
    "catalogFilters",
    onError,
  );
  const defaultFilterApplied = useRef(false);
  const setFilters = useCallback((next: ArchiveFilters) => {
    defaultFilterApplied.current = true;
    setStoredFilters(next, { catalogFiltersCustomized: true });
  }, [setStoredFilters]);
  const [facets, setFacets] = useState<CatalogFacets>({ languages: [], categories: [] });
  useEffect(() => {
    if (defaultFilterApplied.current) return;
    if (filters.languages.length || filters.categories.length || filters.content.length) return;
    if (!facets.languages.length) return;
    defaultFilterApplied.current = true;
    if (getConfirmedSettings()?.catalogFiltersCustomized ?? settings.catalogFiltersCustomized) return;
    const languages = deviceCatalogLanguages(
      navigator.languages ?? [],
      facets.languages,
      getLocale(),
    );
    if (languages.length) setStoredFilters({ ...filters, languages });
  }, [facets.languages, filters, setStoredFilters, settings.catalogFiltersCustomized]);
  const [facetError, setFacetError] = useState("");
  const [facetRetry, setFacetRetry] = useState(0);
  const locale = getLocale();
  const [sort, setSort] = useState("title");
  const [direction, setDirection] = useState<"asc" | "desc">("asc");
  const [view, setView] = useState<"grid" | "list">("grid");
  const [source, setSource] = useState(settings.catalogUrl);
  const [url, setUrl] = useState(() => catalogURL(settings.catalogUrl));
  const [trail, setTrail] = useState<string[]>([]);
  const [page, setPage] = useState<CatalogPage | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const [detail, setDetail] = useState<CatalogEntry | null>(null);
  const [starting, setStarting] = useState<string | null>(null);
  const [startError, setStartError] = useState("");
  const alive = useRef(true);
  const startPending = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  const request = useRef(0);
  useEffect(() => {
    const id = ++request.current;
    const privacy = capturePrivacyContext();
    setBusy(true);
    setError("");
    setPage(null);
    if (!privacy || privacy.generation !== privacyGeneration) {
      setError(t("Browsing session expired after a privacy transition"));
      setBusy(false);
      return;
    }
    api<CatalogPage>("catalog", { url, locale, privacyGeneration: privacy.generation })
      .then((value) => {
        if (id !== request.current) return;
        if (isPrivacyContextCurrent(privacy)) setPage(value);
        else setError(t("Browsing session expired after a privacy transition"));
      })
      .catch((e) => {
        if (id === request.current)
          setError(isPrivacyContextCurrent(privacy)
            ? errorText(e)
            : t("Browsing session expired after a privacy transition"));
      })
      .finally(() => {
        if (id === request.current) setBusy(false);
      });
    return () => {
      request.current++;
    };
  }, [url, locale, retry, privacyGeneration]);
  useEffect(() => {
    setDetail(null);
    setStartError("");
  }, [privacyGeneration]);
  useEffect(() => {
    setSource(settings.catalogUrl);
  }, [settings.catalogUrl]);
  const filterURL = useMemo(() => {
    try {
      const next = new URL(catalogURL(source));
      if (query.trim()) next.searchParams.set("q", query.trim());
      if (filters.languages.length)
        next.searchParams.set("lang", filters.languages.join(","));
      if (filters.categories.length)
        next.searchParams.set("category", filters.categories.join(","));
      const tags = filters.content.filter(
        (tag) =>
          filters.content.filter(
            (other) => other.split(":")[0] === tag.split(":")[0],
          ).length === 1,
      );
      if (tags.length) next.searchParams.set("tag", tags.join(";"));
      return next.href;
    } catch {
      return null;
    }
  }, [source, query, filters]);
  useEffect(() => {
    const timer = window.setTimeout(() => {
      if (filterURL) {
        setUrl(filterURL);
        setTrail([]);
      }
    }, 350);
    return () => window.clearTimeout(timer);
  }, [filterURL]);
  useEffect(() => {
    let alive = true;
    const privacy = capturePrivacyContext();
    setFacetError("");
    setFacets({ languages: [], categories: [] });
    if (!privacy || privacy.generation !== privacyGeneration) {
      setFacetError(t("Browsing session expired after a privacy transition"));
      return;
    }
    void api<CatalogFacets>("catalog_facets", {
      url: source,
      locale,
      privacyGeneration: privacy.generation,
    })
      .then((value) => {
        if (!alive) return;
        if (isPrivacyContextCurrent(privacy)) setFacets(value);
        else setFacetError(t("Browsing session expired after a privacy transition"));
      })
      .catch((error) => {
        if (alive)
          setFacetError(isPrivacyContextCurrent(privacy)
            ? errorText(error)
            : t("Browsing session expired after a privacy transition"));
      });
    return () => {
      alive = false;
    };
  }, [source, locale, facetRetry, privacyGeneration]);
  function filter(event: React.FormEvent) {
    event.preventDefault();
    if (!filterURL) {
      onError(t("The catalog source URL is invalid."));
      return;
    }
    setTrail([]);
    if (filterURL === url) setRetry((previous) => previous + 1);
    else setUrl(filterURL);
  }
  const entries = useMemo(
    () =>
      [...(page?.entries || [])].sort(
        (a, b) =>
          (direction === "asc" ? 1 : -1) * compareArchives(a, b, sort) ||
          a.id.localeCompare(b.id),
      ),
    [page, sort, direction],
  );
  async function preview(entry: CatalogEntry) {
    try {
      if (settings.externalLinks === "deny") {
        onError(t("External links are disabled in Settings."));
        return;
      }
      const acquisition = new URL(entry.url);
      const filename = decodeURIComponent(
        acquisition.pathname.split("/").pop() || "",
      );
      const name = filename.replace(/\.meta4$/i, "").replace(/\.zim$/i, "");
      if (!name || name === filename)
        throw new Error(
          t(
            "This catalog entry does not provide an archive name for online preview.",
          ),
        );
      const viewer = new URL("/viewer", source);
      if (viewer.hostname === "opds.library.kiwix.org")
        viewer.hostname = "library.kiwix.org";
      viewer.hash = encodeURIComponent(name);
      if (viewer.protocol !== "https:")
        throw new Error(t("Online preview requires HTTPS."));
      if (
        !(await confirmAction(
          t(
            "Open this online preview in your browser? It connects to {host} and is not available offline.",
            { host: viewer.host },
          ) +
            "\n\n" +
            viewer.href,
          { confirmLabel: t("Open in browser") },
        ))
      )
        return;
      await api("open_external", { url: viewer.href, confirmed: true });
    } catch (error) {
      onError(error);
    }
  }
  async function download(
    entry: CatalogEntry,
    transport: "http" | "torrent" = "http",
  ) {
    const privacy = capturePrivacyContext();
    if (startPending.current || settings.privateMode || settings.readOnly ||
      !privacy || privacy.privateMode || privacy.readOnly) return;
    if (
      requiresLegacyServiceWorker(entry) &&
      !(await confirmAction(
        t(
          "This archive requires legacy Service Worker replay and cannot be read correctly here. Download it anyway for another compatible reader?",
        ),
      ))
    )
      return;
    if (!alive.current || !isPrivacyContextCurrent(privacy)) return;
    startPending.current = true;
    setStartError("");
    setStarting(entry.id);
    const current = () => alive.current && isPrivacyContextCurrent(privacy);
    try {
      let url = entry.url;
      if (transport === "torrent") {
        url = entry.torrentUrl || await api<string>("catalog_torrent", { url: entry.torrentResolverUrl });
        if (!current()) return;
      }
      const job = await api<DownloadJob>("start_download", {
        url,
        directory: settings.downloadDirectory || undefined,
        expectedSize: entry.size,
      });
      const present = current();
      onDownload({ kind: "download", id: job.id }, { privacy, focus: present });
      if (present) setDetail(null);
    } catch (e) {
      if (current()) {
        setStartError(errorText(e));
        onError(e);
      }
    } finally {
      startPending.current = false;
      if (alive.current) setStarting(null);
    }
  }
  const sources = [...new Set([settings.catalogUrl, ...settings.opdsSources])];
  // Normalizing every installed archive name once per library keeps the regex
  // and lowercase work out of the per-card render path.
  const installedArchives = useMemo(() => {
    const index = new globalThis.Map<string, Book[]>();
    for (const book of books) {
      if (!book.available) continue;
      const name = archiveName(book.name);
      if (!name) continue;
      const matching = index.get(name);
      if (matching) matching.push(book);
      else index.set(name, [book]);
    }
    return index;
  }, [books]);
  const activeFilterCount =
    filters.languages.length + filters.categories.length + filters.content.length;
  const sourceSelector =
    sources.length > 1 ? (
      <label className="field catalog-source">
        {t("Catalog source")}
        <select
          value={source}
          onChange={(e) => {
            setSource(e.target.value);
            setTrail([]);
          }}
        >
          {sources.map((url, index) => (
            <option key={url} value={url}>
              {index === 0 ? t("Default catalog") : url}
            </option>
          ))}
        </select>
      </label>
    ) : null;
  const installedDetail = detail ? installedEdition(detail, installedArchives) : null;
  return (
    <section className="page catalog-page">
      {startError && <p className="inline-error" role="alert">{startError}</p>}
      <PageHeader title={t("Discover")} />
      <form className="catalog-filters" onSubmit={filter}>
        <div className="toolbar compact-toolbar">
          <label className="search-field">
            <Search size={18} aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("Search books")}
              aria-label={t("Search books")}
            />
          </label>
          <button className="primary" disabled={busy}>
            <Search size={16} aria-hidden="true" />
            {t("Search")}
          </button>
        </div>
        <div className="catalog-controls-row">
          <details className="secondary-controls catalog-options">
            <summary>
              <span>{t("Filters and sort")}</span>
              {activeFilterCount > 0 && (
                <span className="catalog-active-filters">
                  {t("{count} active filters", { count: activeFilterCount })}
                </span>
              )}
            </summary>
            <div className="catalog-options-content">
              {sourceSelector}
              <ArchiveFilterBar
                filters={filters}
                onChange={setFilters}
                languages={facets.languages}
                categories={facets.categories}
              />
              <div className="toolbar catalog-sort">
                <label>
                  {t("Sort this page")}
                  <select
                    value={sort}
                    onChange={(event) => setSort(event.target.value)}
                    aria-label={t("Sort archives")}
                  >
                    {archiveSortFields()
                      .filter(([field]) => field !== "available")
                      .map(([value, label]) => (
                        <option value={value} key={value}>
                          {label}
                        </option>
                      ))}
                  </select>
                </label>
                <button
                  type="button"
                  onClick={() => setDirection(direction === "asc" ? "desc" : "asc")}
                >
                  {direction === "asc" ? t("Ascending") : t("Descending")}
                </button>
                <div className="segmented">
                  <button
                    type="button"
                    className={view === "grid" ? "active" : ""}
                    onClick={() => setView("grid")}
                    aria-label={t("Grid view")}
                    aria-pressed={view === "grid"}
                  >
                    <Grid2X2 size={17} />
                  </button>
                  <button
                    type="button"
                    className={view === "list" ? "active" : ""}
                    onClick={() => setView("list")}
                    aria-label={t("List view")}
                    aria-pressed={view === "list"}
                  >
                    <List size={17} />
                  </button>
                </div>
              </div>
            </div>
          </details>
          {activeFilterCount > 0 && (
            <button
              type="button"
              className="text-button catalog-clear-filters"
              onClick={() => setFilters(emptyArchiveFilters())}
            >
              {t("Clear filters")}
            </button>
          )}
          <p className="muted catalog-result-count" aria-live="polite">
            {!busy && !error && page && (
              page.total === null
                ? t("{count} archives on this page", { count: page.entries.length })
                : t("{count} archives found", { count: page.total })
            )}
          </p>
        </div>
        {facetError && (
          <p className="inline-error" role="status">
            {t("Could not load catalog filter choices.")} {facetError}
            <button
              type="button"
              onClick={() => setFacetRetry((previous) => previous + 1)}
            >
              {t("Try again")}
            </button>
          </p>
        )}
        <CatalogCacheNotice
          cache={facets.cache}
          facets
          temporary={settings.privateMode || settings.readOnly}
          onRetry={() => setFacetRetry((previous) => previous + 1)}
        />
      </form>
      {error ? (
        <ErrorNotice
          message={t("Could not load the catalog.") + "\n" + error}
          onRetry={() => setRetry((n) => n + 1)}
        />
      ) : busy ? (
        <Loading label={t("Fetching the catalog…")} />
      ) : (
        page && (
          <>
            <CatalogCacheNotice
              cache={page.cache}
              temporary={settings.privateMode || settings.readOnly}
              onRetry={() => setRetry((previous) => previous + 1)}
            />
            {entries.some((entry) => entry.torrentUrl || entry.torrentResolverUrl) && (
              <p className="muted catalog-torrent-note">
                {t("BitTorrent shares pieces with peers while downloading. Continued seeding after completion requires your permission.")}
              </p>
            )}
            {!page.entries.length ? (
              <Empty
                title={t("No archives found")}
                description={t("Try a broader search or a different language.")}
              />
            ) : (
              <div
                className={`catalog-grid ${view === "list" ? "catalog-list" : ""}`}
              >
                {entries.map((entry) => {
                  const installed = installedEdition(entry, installedArchives);
                  return (
                  <article className="catalog-card" key={entry.id}>
                    <button
                      className="catalog-cover"
                      onClick={() => setDetail(entry)}
                      aria-label={t("Details for {title}", {
                        title: entry.title,
                      })}
                    >
                      <BookCover book={entry} />
                    </button>
                    <div className="catalog-card-content">
                      <button
                        className="text-button book-title"
                        onClick={() => setDetail(entry)}
                      >
                        {entry.title}
                      </button>
                      {installed && (
                        <span className="badge">
                          {installed.exact
                            ? t("Installed")
                            : t("Installed edition {date}", {
                                date: formatDate(installed.book.date),
                              })}
                        </span>
                      )}
                      <p className="muted catalog-card-meta">
                        {languageName(entry.language)}{" "}
                        <span aria-hidden="true">·</span>{" "}
                        {formatBytes(entry.size)}
                      </p>
                      <div className="catalog-card-footer">
                        {installed?.exact ? (
                          <button
                            className="primary"
                            aria-label={t("Open {title}", { title: entry.title })}
                            onClick={() => onOpenBook(installed.book)}
                          >
                            <BookOpen size={16} />
                            {t("Open")}
                          </button>
                        ) : (
                          <button
                            className="primary"
                            aria-label={t("Download {title}", {
                              title: entry.title,
                            })}
                            disabled={starting !== null || settings.privateMode || settings.readOnly}
                            onClick={() => download(entry)}
                          >
                            <ArrowDownToLine size={16} />
                            {starting === entry.id
                              ? t("Starting…")
                              : t("Download")}
                          </button>
                        )}
                        {!installed?.exact && (entry.torrentUrl || entry.torrentResolverUrl) && (
                          <button
                            className="primary"
                            aria-label={`${t("Download with BitTorrent")}: ${entry.title}`}
                            disabled={starting !== null || settings.privateMode || settings.readOnly}
                            onClick={() => download(entry, "torrent")}
                          >
                            <ArrowDownToLine size={16} aria-hidden="true" />
                            {starting === entry.id ? t("Starting…") : t("BitTorrent")}
                          </button>
                        )}
                      </div>
                      <div className="catalog-card-summary">
                        <p className="catalog-description">{entry.description}</p>
                        <EditionBadges book={entry} />
                        <CompatibilityWarning entry={entry} />
                      </div>
                    </div>
                  </article>
                  );
                })}
              </div>
            )}
            <nav className="pagination" aria-label={t("Catalog pages")}>
              <button
                disabled={!trail.length}
                onClick={() => {
                  setUrl(trail[trail.length - 1]);
                  setTrail((previous) => previous.slice(0, -1));
                }}
              >
                <ChevronLeft size={16} />
                {t("Previous")}
              </button>
              <span>
                {t("Page {number}", { number: formatNumber(trail.length + 1) })}
              </span>
              <button
                disabled={!page.next}
                onClick={() => {
                  if (page.next) {
                    setTrail((previous) => [...previous, url]);
                    setUrl(new URL(page.next, url).href);
                  }
                }}
              >
                {t("Next")}
                <ChevronRight size={16} />
              </button>
              <button
                type="button"
                onClick={() =>
                  document
                    .getElementById("main-content")
                    ?.scrollTo({ top: 0, behavior: "smooth" })
                }
              >
                <ArrowUp size={16} />
                {t("Back to top")}
              </button>
            </nav>
          </>
        )
      )}
      {detail && (
        <Modal title={detail.title} onClose={() => setDetail(null)}>
          <p className="detail-description">{detail.description}</p>
          <BookFacts book={detail} />
          <p className="muted">{languageName(detail.language)}</p>
          <EditionBadges book={detail} />
          <CompatibilityWarning entry={detail} />
          {installedDetail?.exact ? (
            <p className="muted">{t("This edition is in your library.")}</p>
          ) : (
            <p className="muted">
              {t("Full archive download: {size}. Downloading here adds a separate archive; it does not replace an installed edition. To replace an edition, use Archive editions in Settings.", {
                size: detail.size > 0 ? formatBytes(detail.size) : t("Size unknown"),
              })}
            </p>
          )}
          {(detail.torrentUrl || detail.torrentResolverUrl) && !installedDetail?.exact && (
            <p className="muted">
              {t("BitTorrent shares pieces with peers while downloading. Continued seeding after completion requires your permission.")}
            </p>
          )}
          {startError && <p className="inline-error" role="alert">{startError}</p>}
          <div className="tag-list">
            {detail.tags.map((tag) => (
              <span key={tag} className="badge">
                {tag}
              </span>
            ))}
          </div>
          <p className="muted file-path">{detail.url}</p>
          <div className="modal-actions">
            <button
              onClick={() => preview(detail)}
              disabled={settings.externalLinks === "deny"}
            >
              <ExternalLink size={17} />
              {t("Online preview")}
            </button>
            {installedDetail?.exact ? (
              <button
                className="primary"
                onClick={() => {
                  const book = installedDetail.book;
                  setDetail(null);
                  onOpenBook(book);
                }}
              >
                <BookOpen size={17} />
                {t("Open")}
              </button>
            ) : (
              <button
                className="primary"
                disabled={starting !== null || settings.privateMode || settings.readOnly}
                onClick={() => download(detail)}
              >
                <ArrowDownToLine size={17} />
                {starting === detail.id
                  ? t("Starting download…")
                  : t("Download archive")}
              </button>
            )}
            {!installedDetail?.exact && (detail.torrentUrl || detail.torrentResolverUrl) && (
              <button
                className="primary"
                disabled={starting !== null || settings.privateMode || settings.readOnly}
                onClick={() => download(detail, "torrent")}
              >
                <ArrowDownToLine size={17} />
                {starting === detail.id ? t("Starting download…") : t("Download with BitTorrent")}
              </button>
            )}
          </div>
        </Modal>
      )}
    </section>
  );
}
