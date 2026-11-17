/**
 * What a screen reader should hear from the library search panel. Keeping this
 * pure lets the panel announce busy, empty, validation, failed and paginated
 * states without duplicating the rules in JSX. Every variant carries a literal
 * catalog key; the caller translates it with static t() calls so the
 * localization audit can inventory every string.
 */
export interface ReaderSearchStatusInput {
  busy: boolean;
  /** Rows rendered for the current page, or null before a search completes. */
  resultCount: number | null;
  /** Total matches reported by the search, or null before a search completes. */
  total: number | null;
  /** Rows before the current page, used to describe the visible range. */
  offset?: number;
  error?: string | null;
  /** True when `error` is input validation rather than a request failure. */
  validation?: boolean;
  hasMore?: boolean;
}

export type ReaderSearchStatus =
  | { kind: "searching"; key: "Searching…" }
  | {
      kind: "validation";
      key: "Enter a word or phrase to search your books.";
    }
  | { kind: "failed"; key: "Search failed" }
  | { kind: "no-results"; key: "No results" }
  | {
      kind: "page";
      key: "{start}–{end} of {total}";
      values: { start: number; end: number; total: number };
      hasMore: boolean;
    }
  | {
      kind: "results";
      key: "About {count} results";
      values: { count: number };
      hasMore: boolean;
    };

export function searchStatusMessage(
  input: ReaderSearchStatusInput,
): ReaderSearchStatus | null {
  if (input.busy) return { kind: "searching", key: "Searching…" };
  if (input.validation)
    return {
      kind: "validation",
      key: "Enter a word or phrase to search your books.",
    };
  if (input.error) return { kind: "failed", key: "Search failed" };
  const total = input.total ?? input.resultCount;
  if (total === null) return null;
  if (total === 0) return { kind: "no-results", key: "No results" };
  const resultCount = input.resultCount;
  // The current page is smaller than the whole result set: say which rows are
  // visible so moving to the next page changes the announcement.
  if (resultCount !== null && resultCount > 0 && resultCount < total) {
    const start = (input.offset ?? 0) + 1;
    return {
      kind: "page",
      key: "{start}–{end} of {total}",
      values: { start, end: start + resultCount - 1, total },
      hasMore: input.hasMore ?? false,
    };
  }
  return {
    kind: "results",
    key: "About {count} results",
    values: { count: total },
    hasMore: input.hasMore ?? false,
  };
}
