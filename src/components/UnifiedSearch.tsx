import { useMemo, useState } from "react";
import {
  Bookmark as BookmarkIcon,
  Clock,
  PanelsTopLeft,
  Search,
} from "lucide-react";
import { t, formatDate } from "../i18n";
import type { Book, Bookmark, HistoryEntry, Tab, TabLocation } from "../types";
import { Modal, PagedResults } from "./UI";
import { recentHistoryEntries } from "../recent-history";

export interface UnifiedSearchProps {
  books: Book[];
  tabs: Tab[];
  bookmarks: Bookmark[];
  history: HistoryEntry[];
  onSelectTab: (id: string) => void;
  onOpen: (location: TabLocation) => void;
  onClose: () => void;
}
export default function UnifiedSearch({
  books,
  tabs,
  bookmarks,
  history,
  onSelectTab,
  onOpen,
  onClose,
}: UnifiedSearchProps) {
  const [query, setQuery] = useState("");
  const [sections, setSections] = useState({
    tabs: true,
    bookmarks: true,
    history: true,
  });
  const recentHistory = useMemo(() => recentHistoryEntries(history), [history]);
  const bookById = useMemo(
    () => new Map(books.map((book) => [book.id, book])),
    [books],
  );
  const needle = query.trim().toLocaleLowerCase();
  // Haystacks are rebuilt only when their source lists change, not per keystroke.
  const tabIndex = useMemo(
    () =>
      tabs.map((tab) => ({
        item: tab,
        haystack:
          `${tab.title} ${tab.bookId ? bookById.get(tab.bookId)?.title || "" : ""} ${tab.path}`
            .toLocaleLowerCase(),
      })),
    [tabs, bookById],
  );
  const bookmarkIndex = useMemo(
    () =>
      bookmarks
        .map((bookmark) => ({
          item: bookmark,
          haystack: `${bookmark.title} ${bookmark.bookId ? bookById.get(bookmark.bookId)?.title || "" : ""} ${bookmark.path} ${bookmark.note} ${bookmark.tags.join(" ")}`
            .toLocaleLowerCase(),
        }))
        .sort((a, b) => a.item.title.localeCompare(b.item.title)),
    [bookmarks, bookById],
  );
  const historyIndex = useMemo(
    () =>
      recentHistory
        .map((entry) => ({
          item: entry,
          haystack: `${entry.title} ${entry.bookId ? bookById.get(entry.bookId)?.title || "" : ""} ${entry.path}`
            .toLocaleLowerCase(),
        }))
        .sort((a, b) => b.item.visitedAt.localeCompare(a.item.visitedAt)),
    [recentHistory, bookById],
  );
  const matchingTabs = useMemo(
    () =>
      sections.tabs
        ? tabIndex.filter((entry) => entry.haystack.includes(needle)).map((entry) => entry.item)
        : [],
    [sections.tabs, tabIndex, needle],
  );
  const matchingBookmarks = useMemo(
    () =>
      sections.bookmarks
        ? bookmarkIndex.filter((entry) => entry.haystack.includes(needle)).map((entry) => entry.item)
        : [],
    [sections.bookmarks, bookmarkIndex, needle],
  );
  const matchingHistory = useMemo(
    () =>
      sections.history
        ? historyIndex.filter((entry) => entry.haystack.includes(needle)).map((entry) => entry.item)
        : [],
    [sections.history, historyIndex, needle],
  );
  const count =
    matchingTabs.length + matchingBookmarks.length + matchingHistory.length;
  function open(location: TabLocation) {
    onOpen(location);
    onClose();
  }
  return (
    <Modal title={t("Search")} onClose={onClose}>
      <div className="unified-search">
        <div className="toolbar compact-toolbar">
          <label className="search-field">
            <Search size={18} />
            <input
              type="search"
              autoFocus
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("Find an open tab or a saved article…")}
              aria-label={t("Search local navigation")}
            />
          </label>
          <details className="secondary-controls">
            <summary>
              {t("Search sections")}
              {(!sections.tabs || !sections.bookmarks || !sections.history) && (
                <> · {[
                  sections.tabs && t("Open tabs"),
                  sections.bookmarks && t("Reading list"),
                  sections.history && t("Recently read"),
                ].filter(Boolean).join(", ") || t("No search sections selected")}</>
              )}
            </summary>
            <div className="unified-sections" aria-label={t("Search sections")}>
              {(
                [
                  ["tabs", t("Open tabs"), PanelsTopLeft],
                  ["bookmarks", t("Reading list"), BookmarkIcon],
                  ["history", t("Recently read"), Clock],
                ] as const
              ).map(([key, label, Icon]) => (
                <button
                  key={key}
                  aria-pressed={sections[key]}
                  className={sections[key] ? "active" : ""}
                  onClick={() =>
                    setSections((previous) => ({
                      ...previous,
                      [key]: !previous[key],
                    }))
                  }
                >
                  <Icon size={15} />
                  {label}
                </button>
              ))}
            </div>
          </details>
        </div>
        <p className="muted" role="status">
          {t("{count} results", { count })}
        </p>
        <div className="unified-results">
          {!!matchingTabs.length && (
            <section>
              <h3>
                <PanelsTopLeft size={16} />
                {t("Open tabs")}
              </h3>
              <PagedResults items={matchingTabs} criteria={needle} label={t("Open tabs")}>
                {(tab) => (
                  <button
                    className="unified-result"
                    key={tab.id}
                    onClick={() => {
                      onSelectTab(tab.id);
                      onClose();
                    }}
                  >
                    <strong>{tab.title || t("New tab")}</strong>
                    <span>
                      {tab.bookId
                        ? bookById.get(tab.bookId)?.title ||
                          t("Archive not in library")
                        : t("New tab")}
                    </span>
                    <small>{t("Switch to tab")}</small>
                  </button>
                )}
              </PagedResults>
            </section>
          )}
          {!!matchingBookmarks.length && (
            <section>
              <h3>
                <BookmarkIcon size={16} />
                {t("Reading list")}
              </h3>
              <PagedResults items={matchingBookmarks} criteria={needle} label={t("Reading list")}>
                {(item) => (
                  <button
                    className="unified-result"
                    key={item.id}
                    disabled={!bookById.get(item.bookId)?.available}
                    onClick={() =>
                      open({
                        bookId: item.bookId,
                        path: item.path,
                        title: item.title,
                        scroll: 0,
                      })
                    }
                  >
                    <strong>{item.title}</strong>
                    <span>
                      {bookById.get(item.bookId)?.title ||
                        t("Archive not in library")}
                    </span>
                    {item.note && <p>{item.note}</p>}
                    <small>
                      {bookById.get(item.bookId)?.available
                        ? t("Open article in a new tab")
                        : t("Archive unavailable")}
                    </small>
                  </button>
                )}
              </PagedResults>
            </section>
          )}
          {!!matchingHistory.length && (
            <section>
              <h3>
                <Clock size={16} />
                {t("Recently read")}
              </h3>
              <PagedResults items={matchingHistory} criteria={needle} label={t("Recently read")}>
                {(item) => (
                  <button
                    className="unified-result"
                    key={item.id}
                    disabled={!bookById.get(item.bookId)?.available}
                    onClick={() =>
                      open({
                        bookId: item.bookId,
                        path: item.path,
                        title: item.title,
                        scroll: item.scroll,
                      })
                    }
                  >
                    <strong>{item.title}</strong>
                    <span>
                      {bookById.get(item.bookId)?.title ||
                        t("Archive not in library")}{" "}
                      · {formatDate(item.visitedAt)}
                    </span>
                    <small>
                      {bookById.get(item.bookId)?.available
                        ? t("Open article in a new tab")
                        : t("Archive unavailable")}
                    </small>
                  </button>
                )}
              </PagedResults>
            </section>
          )}
          {!count && (
            <p className="unified-empty">
              {t(
                "No matching tabs or articles. Try another search or enable another section.",
              )}
            </p>
          )}
        </div>
      </div>
    </Modal>
  );
}
