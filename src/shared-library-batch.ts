import type { Book, DownloadJob, SharedLibraryInventory } from "./types";

export interface SharedLibraryBatchProgress {
  total: number;
  processed: number;
  queued: number;
  installed: number;
  active: number;
  unavailable: number;
  duplicates: number;
  stopped: boolean;
  failure?: { bookId: string; title: string; error: unknown };
}


export async function queueSharedLibrary(
  inventory: SharedLibraryInventory,
  installed: Pick<Book, "id" | "available">[],
  activeJobs: DownloadJob[],
  options: {
    isCurrent: () => boolean;
    start: (bookId: string) => Promise<DownloadJob>;
    onQueued: (job: DownloadJob) => void;
    onProgress: (progress: SharedLibraryBatchProgress) => void;
  },
): Promise<SharedLibraryBatchProgress> {
  const books = inventory.books.map((book) => ({ ...book }));
  const installedById = new Map(installed.map((book) => [book.id, book.available]));
  const activeByUrl = new Map<string, DownloadJob>();
  for (const job of activeJobs) {
    if (job.status === "queued" || job.status === "downloading" || job.status === "verifying") {
      if (!activeByUrl.has(job.url)) activeByUrl.set(job.url, job);
    }
  }
  const seenArchives = new Set<string>();
  const progress: SharedLibraryBatchProgress = {
    total: books.length,
    processed: 0,
    queued: 0,
    installed: 0,
    active: 0,
    unavailable: 0,
    duplicates: 0,
    stopped: false,
  };
  const snapshot = (): SharedLibraryBatchProgress => Object.freeze({
    ...progress,
    ...(progress.failure ? { failure: Object.freeze({ ...progress.failure }) } : {}),
  });
  const publish = () => options.onProgress(snapshot());
  publish();

  for (const book of books) {
    if (!options.isCurrent()) {
      progress.stopped = true;
      publish();
      break;
    }
    let currentAfterAdmission = true;
    const active = activeByUrl.get(book.downloadIdentity);
    if (book.archiveId && seenArchives.has(book.archiveId)) {
      progress.duplicates++;
    } else if (book.archiveId && installedById.has(book.archiveId)) {
      if (installedById.get(book.archiveId)) progress.installed++;
      else progress.unavailable++;
    } else if (active) {
      progress.active++;
      options.onQueued(active);
    } else {
      // Classification and progress callbacks may retire the view; admission
      // requires its own immediately preceding ownership check.
      if (!options.isCurrent()) {
        progress.stopped = true;
        publish();
        break;
      }
      let job: DownloadJob;
      try {
        job = await options.start(book.id);
      } catch (error) {
        progress.failure = { bookId: book.id, title: book.title || book.fileName, error };
        progress.stopped = true;
        publish();
        break;
      }
      currentAfterAdmission = options.isCurrent();
      progress.queued++;
      // An accepted job belongs to Downloads even if the initiating view was
      // retired during admission. Never gate root tracking on view ownership.
      options.onQueued(job);
    }
    if (book.archiveId) seenArchives.add(book.archiveId);
    progress.processed++;
    if (!currentAfterAdmission && progress.processed < progress.total) progress.stopped = true;
    publish();
    if (progress.stopped) break;
  }
  return snapshot();
}
