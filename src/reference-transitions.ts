import type { ReferenceTransition, Tab } from "./types";

/** Replay ordered native changes once, preserving unsaved navigation and tab IDs. */
export function translateLocation<T extends { bookId: string | null }>(
  location: T,
  transitions: readonly ReferenceTransition[],
  afterRevision: number,
): T {
  let bookId = location.bookId;
  for (const transition of transitions) {
    if (transition.revision > afterRevision && transition.fromBookId === bookId)
      bookId = transition.toBookId;
  }
  return bookId === location.bookId ? location : { ...location, bookId };
}

interface IndexedTransition {
  /** Position in the source array, so replay keeps the original application order. */
  position: number;
  toBookId: string;
}

/** Index retained transitions by source book once per snapshot translation. */
function transitionIndex(
  transitions: readonly ReferenceTransition[],
  afterRevision: number,
): Map<string, IndexedTransition[]> {
  const index = new Map<string, IndexedTransition[]>();
  for (let position = 0; position < transitions.length; position++) {
    const transition = transitions[position];
    if (!(transition.revision > afterRevision)) continue;
    const entry = { position, toBookId: transition.toBookId };
    const entries = index.get(transition.fromBookId);
    if (entries) entries.push(entry);
    else index.set(transition.fromBookId, [entry]);
  }
  return index;
}

/** Same replay as `translateLocation`, with O(1) lookups per chain step. */
function translateBookId(
  index: Map<string, IndexedTransition[]>,
  bookId: string | null,
): string | null {
  if (bookId === null || !index.size) return bookId;
  let current: string | null = bookId;
  let cursor = -1;
  while (current !== null) {
    const entries = index.get(current);
    if (!entries) break;
    let next: IndexedTransition | undefined;
    for (const entry of entries) {
      if (entry.position > cursor) {
        next = entry;
        break;
      }
    }
    if (!next) break;
    current = next.toBookId;
    cursor = next.position;
  }
  return current;
}

/** Same contract as `translateLocation`, reusing a prebuilt transition index. */
function translateLocationWithIndex<T extends { bookId: string | null }>(
  index: Map<string, IndexedTransition[]>,
  location: T,
): T {
  const bookId = translateBookId(index, location.bookId);
  return bookId === location.bookId ? location : { ...location, bookId };
}

export function translateTabs(
  tabs: readonly Tab[],
  transitions: readonly ReferenceTransition[],
  afterRevision: number,
): Tab[] {
  const index = transitionIndex(transitions, afterRevision);
  return tabs.map((tab) => {
    const current = translateLocationWithIndex(index, tab);
    const back = tab.back.map((location) => translateLocationWithIndex(index, location));
    const forward = tab.forward.map((location) => translateLocationWithIndex(index, location));
    return current === tab && back.every((entry, index) => entry === tab.back[index]) &&
      forward.every((entry, index) => entry === tab.forward[index])
      ? tab : { ...current, back, forward };
  });
}

/** Bookmark draft keys contain reading references, never installation authority. */
export function translateBookmarkDrafts<T>(
  drafts: Partial<Record<string, T>>,
  transitions: readonly ReferenceTransition[],
  afterRevision: number,
): Partial<Record<string, T>> {
  const index = transitionIndex(transitions, afterRevision);
  return Object.fromEntries(Object.entries(drafts).map(([key, draft]) => {
    const [bookId, documentPath] = JSON.parse(key) as [string | null, string];
    const translated = translateBookId(index, bookId);
    return [translated === bookId ? key : JSON.stringify([translated, documentPath]), draft];
  }));
}
