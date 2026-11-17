import { useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Bookmark as BookmarkIcon,
  Clock,
  FileUp,
  Search,
  Trash2,
} from "lucide-react";
import { api, capturePrivacyContext, isPrivacyContextCurrent, readTextFile, saveText } from "../api";
import { t, formatDate } from "../i18n";
import type { Book, Bookmark, HistoryEntry, TabLocation } from "../types";
import { Empty, Modal, PageHeader, confirmAction } from "./UI";
import { SwipeRow } from "./SwipeRow";
import PrivateReadingSync from "./PrivateReadingSync";
import { recentHistoryEntries } from "../recent-history";
interface Props {
  books: Book[];
  bookmarks: Bookmark[];
  history: HistoryEntry[];
  kind: "bookmarks" | "history";
  readOnly: boolean;
  privateMode: boolean;
  privacyGeneration: number;
  onConnect: () => void;
  onOpen: (location: TabLocation) => void;
  onRefresh: () => void;
  onError: (error: unknown) => void;
}

export default function Collections({
  books,
  bookmarks,
  history,
  kind,
  readOnly,
  privateMode,
  privacyGeneration,
  onConnect,
  onOpen,
  onRefresh,
  onError,
}: Props) {
  const readOnlyRef = useRef(readOnly);
  readOnlyRef.current = readOnly;
  const [query, setQuery] = useState("");
  const [tag, setTag] = useState("");
  const [edit, setEdit] = useState<Bookmark | null>(null);
  const [sort, setSort] = useState<
    "recent" | "oldest" | "alphabetical" | "reverse-alphabetical"
  >("recent");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const tags = useMemo(
    () => [...new Set(bookmarks.flatMap((b) => b.tags))].sort(),
    [bookmarks],
  );
  const recentHistory = useMemo(() => recentHistoryEntries(history), [history]);
  // Archive titles resolve through one index; the previous per-item scans made
  // both lists quadratic in the library size.
  const bookById = useMemo(() => {
    const index = new globalThis.Map<string, Book>();
    for (const book of books) if (!index.has(book.id)) index.set(book.id, book);
    return index;
  }, [books]);
  const items = useMemo(
    () =>
      bookmarks
        .filter(
          (b) =>
            (!tag || b.tags.includes(tag)) &&
            `${b.title} ${b.note} ${b.tags.join(" ")} ${bookById.get(b.bookId)?.title || ""}`
              .toLocaleLowerCase()
              .includes(query.toLocaleLowerCase()),
        )
        .sort((a, b) =>
          sort === "alphabetical"
            ? a.title.localeCompare(b.title)
            : sort === "reverse-alphabetical"
              ? b.title.localeCompare(a.title)
              : sort === "oldest"
                ? a.createdAt.localeCompare(b.createdAt)
                : b.createdAt.localeCompare(a.createdAt),
        ),
    [bookmarks, bookById, query, tag, sort],
  );
  const visits = useMemo(
    () =>
      recentHistory
        .filter((h) =>
          `${h.title} ${bookById.get(h.bookId)?.title || ""}`
            .toLocaleLowerCase()
            .includes(query.toLocaleLowerCase()),
        )
        .sort((a, b) =>
          sort === "alphabetical"
            ? a.title.localeCompare(b.title)
            : sort === "reverse-alphabetical"
              ? b.title.localeCompare(a.title)
              : sort === "oldest"
                ? a.visitedAt.localeCompare(b.visitedAt)
                : b.visitedAt.localeCompare(a.visitedAt),
        ),
    [recentHistory, bookById, query, sort],
  );
  async function remove(item: Bookmark) {
    const privacy = capturePrivacyContext();
    if (!privacy || readOnlyRef.current || busy) return;
    if (
      !(await confirmAction(
        t(
          "Remove “{title}” from your reading list? Its note and tags will also be removed.",
          { title: item.title },
        ),
        { confirmLabel: t("Remove"), danger: true },
      ))
    )
      return;
    if (readOnlyRef.current || !isPrivacyContextCurrent(privacy)) return;
    setBusy(true);
    try {
      await api("remove_bookmark", { id: item.id, privacyGeneration: privacy.generation });
      if (!isPrivacyContextCurrent(privacy)) return;
      setEdit(null);
      onRefresh();
    } catch (e) {
      if (isPrivacyContextCurrent(privacy)) onError(e);
    } finally {
      setBusy(false);
    }
  }
  async function removeHistory(item: HistoryEntry) {
    const privacy = capturePrivacyContext();
    if (!privacy || readOnlyRef.current || busy) return;
    setBusy(true);
    try {
      await api("remove_history_entry", { id: item.id, privacyGeneration: privacy.generation });
      if (isPrivacyContextCurrent(privacy)) onRefresh();
    } catch (e) {
      if (isPrivacyContextCurrent(privacy)) onError(e);
    } finally {
      setBusy(false);
    }
  }
  async function clearList() {
    const privacy = capturePrivacyContext();
    if (!privacy || readOnlyRef.current || busy || !bookmarks.length) return;
    if (
      !(await confirmAction(
        t(
          "Remove all {count} saved articles, including their notes and tags? This cannot be undone.",
          { count: bookmarks.length },
        ),
        { confirmLabel: t("Clear reading list"), danger: true },
      ))
    )
      return;
    if (readOnlyRef.current || !isPrivacyContextCurrent(privacy)) return;
    setBusy(true);
    try {
      // The service exposes one bookmark at a time; keep the same privacy
      // generation for the whole batch and stop at the first failure. Each
      // deletion commits separately, so a failed response can follow a
      // committed write.
      let failed = false;
      let failure: unknown;
      for (const item of bookmarks) {
        if (readOnlyRef.current || !isPrivacyContextCurrent(privacy)) break;
        try {
          await api("remove_bookmark", {
            id: item.id,
            privacyGeneration: privacy.generation,
          });
        } catch (e) {
          failed = true;
          failure = e;
          break;
        }
      }
      if (failed && isPrivacyContextCurrent(privacy)) onError(failure);
    } finally {
      // Each deletion commits separately, and a failed response can follow a
      // committed write. Reconcile partial progress before the next attempt.
      if (isPrivacyContextCurrent(privacy)) onRefresh();
      setBusy(false);
    }
  }
  async function exportList(format: "json" | "xml") {
    if (readOnlyRef.current) return;
    setBusy(true);
    try {
      const text = await api<string>("export_bookmarks", { format });
      if (readOnlyRef.current) return;
      await saveText(`kiwix-reading-list.${format}`, text);
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  }
  async function exportHistory() {
    if (readOnlyRef.current) return;
    setBusy(true);
    try {
      const text = await api<string>("export_history");
      if (readOnlyRef.current) return;
      await saveText("kiwix-history.json", text);
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  }
  async function importList() {
    if (readOnlyRef.current) return;
    setBusy(true);
    setNotice("");
    try {
      const text = await readTextFile();
      if (text === null) return;
      const count = await api<number>("import_bookmarks", { text });
      setNotice(t("Imported {count} reading list entries.", { count: count }));
      onRefresh();
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  }
  async function save(e: React.FormEvent) {
    e.preventDefault();
    if (!edit || readOnlyRef.current || busy) return;
    setBusy(true);
    try {
      await api("save_bookmark", {
        bookmark: {
          id: edit.id,
          bookId: edit.bookId,
          path: edit.path,
          title: edit.title,
          note: edit.note,
          tags: [
            ...new Set(edit.tags.map((value) => value.trim()).filter(Boolean)),
          ],
        },
      });
      // The submitted draft owns only its own completion. Typing while the
      // write is pending, or opening another editor, must preserve that draft.
      setEdit((current) => current === edit ? null : current);
      onRefresh();
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="page">
      <PageHeader
        title={kind === "bookmarks" ? t("Reading list") : t("Recently read")}
        actions={<button onClick={onConnect}>{t("Connect my devices")}</button>}
      />
      <details className="secondary-controls">
        <summary>{t("Reading across my devices")}</summary>
        <PrivateReadingSync
          presentation="reading"
          privateMode={privateMode}
          readOnly={readOnly}
          privacyGeneration={privacyGeneration}
          onConnect={onConnect}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      </details>
      <div className="toolbar compact-toolbar">
        <label className="search-field">
          <Search size={17} />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={
              kind === "bookmarks"
                ? t("Search titles, notes and tags…")
                : t("Search history…")
            }
            aria-label={
              kind === "bookmarks"
                ? t("Search reading list")
                : t("Search history")
            }
          />
        </label>
        <details className="secondary-controls">
          <summary>
            {t("Filters & actions")}
            {kind === "bookmarks" && tag && <> · {tag}</>}
            {sort !== "recent" && (
              <> · {sort === "oldest"
                ? t("Oldest first")
                : sort === "alphabetical"
                  ? t("Title: A to Z")
                  : t("Title: Z to A")}</>
            )}
          </summary>
          <div className="toolbar">
            {kind === "bookmarks" && (
              <select
                value={tag}
                onChange={(e) => setTag(e.target.value)}
                aria-label={t("Filter by tag")}
              >
                <option value="">{t("All tags")}</option>
                {tags.map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            )}
            <select
              value={sort}
              onChange={(event) => setSort(event.target.value as typeof sort)}
              aria-label={
                kind === "bookmarks" ? t("Sort reading list") : t("Sort history")
              }
            >
              <option value="recent">{t("Most recent first")}</option>
              <option value="oldest">{t("Oldest first")}</option>
              <option value="alphabetical">{t("Title: A to Z")}</option>
              <option value="reverse-alphabetical">{t("Title: Z to A")}</option>
            </select>
          </div>
          <div className="toolbar">
            {kind === "bookmarks" ? (
              <>
                <button disabled={busy || readOnly} onClick={importList}>
                  <FileUp size={16} />
                  {t("Import reading list")}
                </button>
                <button
                  disabled={busy || readOnly || !bookmarks.length}
                  onClick={() => exportList("json")}
                >
                  <ArrowDownToLine size={16} />
                  {t("Export JSON")}
                </button>
                <button
                  disabled={busy || readOnly || !bookmarks.length}
                  onClick={() => exportList("xml")}
                >
                  <ArrowDownToLine size={16} />
                  {t("Export XML")}
                </button>
                <button
                  className="danger"
                  disabled={busy || readOnly || !bookmarks.length}
                  onClick={() => void clearList()}
                >
                  <Trash2 size={16} />
                  {t("Clear reading list")}
                </button>
              </>
            ) : (
              <>
                <button
                  disabled={busy || readOnly || !history.length}
                  onClick={() => void exportHistory()}
                >
                  <ArrowDownToLine size={16} />
                  {t("Export history")}
                </button>
                <button
                  className="danger"
                  disabled={busy || readOnly || !history.length}
                  onClick={async () => {
                    const privacy = capturePrivacyContext();
                    if (!privacy || readOnlyRef.current || busy) return;
                    if (
                      !(await confirmAction(
                        t("Clear all reading history? This cannot be undone."),
                        { confirmLabel: t("Clear history"), danger: true },
                      ))
                    )
                      return;
                    if (readOnlyRef.current || !isPrivacyContextCurrent(privacy)) return;
                    setBusy(true);
                    try {
                      await api("clear_history", { privacyGeneration: privacy.generation });
                      if (isPrivacyContextCurrent(privacy)) onRefresh();
                    } catch (e) {
                      if (isPrivacyContextCurrent(privacy)) onError(e);
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  <Trash2 size={16} />
                  {t("Clear history")}
                </button>
              </>
            )}
          </div>
        </details>
      </div>
      {notice && (
        <p className="success-text" role="status">
          {notice}
        </p>
      )}
      {kind === "bookmarks" ? (
        !items.length ? (
          <Empty
            title={
              bookmarks.length
                ? t("No matching saved articles")
                : t("No saved articles")
            }
            description={
              bookmarks.length
                ? t("Try another search or tag.")
                : t("Save articles with the bookmark button in the reader.")
            }
          />
        ) : (
          <div className="reading-list">
            {items.map((item) => {
              const book = bookById.get(item.bookId);
              return (
                <SwipeRow key={item.id} actionLabel={t("Remove saved article")}
                  onAction={() => void remove(item)} disabled={readOnly || busy}>
                <article className="reading-card panel">
                  <BookmarkIcon size={21} />
                  <div className="reading-card-main">
                    <button
                      className="text-button"
                      disabled={!book?.available}
                      onClick={() =>
                        onOpen({
                          bookId: item.bookId,
                          path: item.path,
                          title: item.title,
                          scroll: 0,
                        })
                      }
                    >
                      <h2>{item.title}</h2>
                    </button>
                    <p className="muted">
                      {book?.title || t("Archive not in library")}{" "}
                      <span aria-hidden="true">·</span>{" "}
                      {formatDate(item.createdAt)}
                    </p>
                    {!book?.available && (
                      <span className="badge warning">
                        {t("Archive unavailable")}
                      </span>
                    )}
                    {item.note && <p className="saved-note">{item.note}</p>}
                    <div className="tag-list">
                      {item.tags.map((value) => (
                        <button
                          className="badge"
                          key={value}
                          onClick={() => setTag(value)}
                        >
                          {value}
                        </button>
                      ))}
                    </div>
                  </div>
                  <button
                    disabled={readOnly}
                    onClick={() => setEdit({ ...item, tags: [...item.tags] })}
                  >
                    {t("Edit note and tags")}
                  </button>
                </article>
                </SwipeRow>
              );
            })}
          </div>
        )
      ) : !visits.length ? (
        <Empty
          title={
            history.length
              ? t("No matching history")
              : t("No reading history")
          }
          description={
            history.length
              ? t("Try another search.")
              : t(
                  "Articles you read will appear here when reading history is enabled in Settings.",
                )
          }
        />
      ) : (
        <div className="history-list">
          {visits.map((item) => {
            const book = bookById.get(item.bookId);
            return (
              <SwipeRow key={item.id} actionLabel={t("Remove")}
                onAction={() => void removeHistory(item)} disabled={readOnly || busy}>
              <article className="history-item">
                <Clock size={18} />
                <div>
                  <button
                    className="text-button"
                    disabled={!book?.available}
                    onClick={() =>
                      onOpen({
                        bookId: item.bookId,
                        path: item.path,
                        title: item.title,
                        scroll: item.scroll,
                      })
                    }
                  >
                    <strong>{item.title}</strong>
                  </button>
                  <p>
                    {book?.title || t("Archive not in library")}
                    {!book?.available && " · " + t("Archive unavailable")}
                  </p>
                </div>
                <time dateTime={item.visitedAt}>
                  {formatDate(item.visitedAt)}
                </time>
                <button className="icon-button danger" disabled={readOnly || busy}
                  aria-label={t("Remove")} title={t("Remove")}
                  onClick={() => void removeHistory(item)}>
                  <Trash2 size={16} />
                </button>
              </article>
              </SwipeRow>
            );
          })}
        </div>
      )}
      {edit && (
        <Modal title={t("Edit saved article")} onClose={() => setEdit(null)}>
          <form onSubmit={save}>
            <label className="field">
              {t("Title")}
              <input
                value={edit.title}
                required
                onChange={(e) => setEdit({ ...edit, title: e.target.value })}
              />
            </label>
            <label className="field">
              {t("Your note")}
              <textarea
                rows={6}
                value={edit.note}
                onChange={(e) => setEdit({ ...edit, note: e.target.value })}
                placeholder={t("What would you like to remember?")}
              />
            </label>
            <label className="field">
              {t("Tags, separated by commas")}
              <input
                value={edit.tags.join(", ")}
                onChange={(e) =>
                  setEdit({
                    ...edit,
                    tags: e.target.value.split(",").map((v) => v.trimStart()),
                  })
                }
                placeholder={t("research, travel, favorites")}
              />
            </label>
            <div className="modal-actions">
              <button
                type="button"
                className="danger"
                onClick={() => remove(edit)}
                disabled={busy || readOnly}
              >
                <Trash2 size={16} />
                {t("Remove saved article")}
              </button>
              <button className="primary" disabled={busy || readOnly}>
                {busy ? t("Saving…") : t("Save changes")}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </section>
  );
}
