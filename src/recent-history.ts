import type { HistoryEntry } from "./types";

export function recentHistoryEntries(history: HistoryEntry[]): HistoryEntry[] {
  const latest = new Map<string, HistoryEntry>();
  for (const entry of history) {
    const fragment = entry.path.indexOf("#");
    const resource = fragment < 0 ? entry.path : entry.path.slice(0, fragment);
    const query = resource.indexOf("?");
    const pathname = query < 0 ? resource : resource.slice(0, query);
    const suffix = query < 0 ? "" : resource.slice(query);
    // Match archive lookup: split literal delimiters before decoding the path
    // once. Encoded ?/# remain path text; query text and path case stay intact.
    let decoded = pathname;
    let validUtf8 = true;
    try {
      decoded = decodeURIComponent(pathname.replace(/%(?![a-f\d]{2})/gi, "%25"));
    } catch {
      // Invalid UTF-8 cannot identify a decoded archive path. Keep its raw
      // identity separate, even if it resembles another decoded pathname.
      validUtf8 = false;
    }
    const key = JSON.stringify([entry.bookId, validUtf8, decoded, suffix]);
    const previous = latest.get(key);
    if (!previous || entry.visitedAt > previous.visitedAt) latest.set(key, entry);
  }
  return [...latest.values()];
}
