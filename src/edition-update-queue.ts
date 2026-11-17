import type { Book, ContentUpdates, DownloadTarget } from "./types";

/** An unambiguous newer edition that may be queued without per-archive review. */
export interface EditionUpdateCandidate {
  bookId: string;
  installationId: string;
  title: string;
  size: number;
  selection: ContentUpdates["updates"][number]["selection"];
}

export interface EditionUpdateQueueProgress {
  total: number;
  processed: number;
  started: number;
  failed: number;
  skipped: number;
  stopped: boolean;
  failures: { bookId: string; title: string; error: unknown }[];
}

/**
 * Native `find_updates` marks only bound series with exactly one newer
 * candidate as `bulkEligible`. Keep that policy native: this filter only
 * verifies the installed archive still exists and refuses to queue the same
 * installation twice.
 */
export function selectBulkEditionUpdates(
  updates: ContentUpdates["updates"],
  installed: Pick<Book, "id" | "installationId">[],
): EditionUpdateCandidate[] {
  const installedKeys = new Set(
    installed.map((book) => `${book.id}\u0000${book.installationId}`),
  );
  const seenInstallations = new Set<string>();
  const candidates: EditionUpdateCandidate[] = [];
  for (const update of updates) {
    if (!update.bulkEligible) continue;
    if (!installedKeys.has(`${update.bookId}\u0000${update.installationId}`)) continue;
    if (seenInstallations.has(update.installationId)) continue;
    seenInstallations.add(update.installationId);
    candidates.push({
      bookId: update.bookId,
      installationId: update.installationId,
      title: update.selection.entry.title,
      size: update.selection.entry.size,
      selection: update.selection,
    });
  }
  return candidates;
}

/**
 * Queue prepared edition downloads through the existing native edition
 * commands with bounded concurrency. A failed or cancelled item never blocks
 * later items from starting, and every accepted download is reported so the
 * existing Downloads replacement flow owns it.
 */
export async function queueEditionUpdates(
  candidates: readonly EditionUpdateCandidate[],
  options: {
    isCurrent: () => boolean;
    concurrency?: number;
    prepare: (candidate: EditionUpdateCandidate) => Promise<{ token: string }>;
    start: (token: string, candidate: EditionUpdateCandidate) => Promise<DownloadTarget>;
    onStarted?: (candidate: EditionUpdateCandidate, target: DownloadTarget) => void;
    onProgress: (progress: EditionUpdateQueueProgress) => void;
  },
): Promise<EditionUpdateQueueProgress> {
  const list = candidates.map((candidate) => ({
    ...candidate,
    selection: { ...candidate.selection },
  }));
  const progress: EditionUpdateQueueProgress = {
    total: list.length,
    processed: 0,
    started: 0,
    failed: 0,
    skipped: 0,
    stopped: false,
    failures: [],
  };
  const publish = (): EditionUpdateQueueProgress => {
    const snapshot: EditionUpdateQueueProgress = {
      ...progress,
      failures: progress.failures.map((failure) => ({ ...failure })),
    };
    options.onProgress(snapshot);
    return snapshot;
  };
  publish();
  if (!list.length) return publish();

  const recordFailure = (candidate: EditionUpdateCandidate, error: unknown) => {
    progress.failed++;
    progress.processed++;
    progress.failures.push({
      bookId: candidate.bookId,
      title: candidate.title,
      error,
    });
    publish();
  };

  let next = 0;
  // Native edition preparation retains one decision per window and download
  // admission shares a single pending disk-space confirmation. Serialize the
  // prepare -> start handoff; transfers still run in parallel once started.
  let handoff: Promise<unknown> = Promise.resolve();
  const withHandoff = <T>(action: () => Promise<T>): Promise<T> => {
    const run = handoff.then(action, action);
    handoff = run.then(() => undefined, () => undefined);
    return run;
  };
  const worker = async () => {
    while (true) {
      if (progress.stopped) return;
      if (!options.isCurrent()) {
        progress.stopped = true;
        publish();
        return;
      }
      const index = next++;
      if (index >= list.length) return;
      const candidate = list[index];
      const outcome = await withHandoff(async () => {
        if (progress.stopped || !options.isCurrent()) return { skipped: true as const };
        let token: string;
        try {
          ({ token } = await options.prepare(candidate));
        } catch (error) {
          return { error };
        }
        // Retirement or cancellation after preparation must not start a transfer.
        if (progress.stopped || !options.isCurrent()) return { skipped: true as const };
        try {
          return { target: await options.start(token, candidate) };
        } catch (error) {
          return { error };
        }
      });
      if ("skipped" in outcome) {
        progress.stopped = true;
        progress.skipped++;
        progress.processed++;
        publish();
        return;
      }
      if ("error" in outcome) {
        recordFailure(candidate, outcome.error);
        continue;
      }
      progress.started++;
      progress.processed++;
      publish();
      // An accepted job belongs to Downloads even if the initiating view was
      // retired while admission was in flight.
      options.onStarted?.(candidate, outcome.target);
    }
  };

  const concurrency = Math.max(
    1,
    Math.min(Math.floor(options.concurrency ?? 2), list.length),
  );
  await Promise.all(Array.from({ length: concurrency }, worker));

  const remaining = list.length - progress.processed;
  if (remaining > 0) {
    progress.skipped += remaining;
    progress.processed = list.length;
    publish();
  }
  return publish();
}
