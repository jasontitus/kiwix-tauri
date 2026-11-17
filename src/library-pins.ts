/** Mirrors `Settings::MAX_PINNED_BOOKS` in src-tauri/src/store.rs. */
export const MAX_PINNED_BOOKS = 3;

/**
 * Pinned archives lead in pin order, then every other archive keeps the
 * library's current sort. Unpinned archives therefore return to exactly the
 * position the active sort gives them.
 */
export function orderLibraryBooks<T extends { id: string }>(
  books: readonly T[],
  pinned: readonly string[],
): T[] {
  if (!pinned.length) return [...books];
  const leading: T[] = [];
  const used = new Set<string>();
  for (const id of pinned) {
    if (used.has(id)) continue;
    const book = books.find((entry) => entry.id === id);
    if (book) {
      leading.push(book);
      used.add(id);
    }
  }
  const rest = books.filter((book) => !used.has(book.id));
  return [...leading, ...rest];
}

/** Persisted settings can outlive a removed archive; keep the usable order. */
export function usablePinnedBooks(
  pinned: readonly string[],
  books: readonly { id: string }[],
): string[] {
  if (!pinned.length) return [];
  const available = new Set(books.map((book) => book.id));
  const seen = new Set<string>();
  return pinned.filter((id) => {
    if (!available.has(id) || seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}
