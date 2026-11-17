import type { Book, DownloadJob, DownloadNotificationAction, MapJob, Settings } from "./types";

/** Anything the Downloads list orders: a download job or the map job. */
export interface Ranked {
  id: string;
  createdAt?: string;
}

/**
 * When the download was last started by the user: its creation (RFC3339), or a
 * later Resume/Retry in this view. `null` when neither is known.
 */
function jobStartTime(job: Ranked, restartedAt?: ReadonlyMap<string, number>): number | null {
  const created = job.createdAt ? Date.parse(job.createdAt) : Number.NaN;
  const restarted = restartedAt?.get(job.id);
  if (restarted !== undefined) return Number.isNaN(created) ? restarted : Math.max(created, restarted);
  return Number.isNaN(created) ? null : created;
}

/**
 * Downloads in reverse chronological order: the most recently started at the
 * top, whatever its state. A download keeps its place while it runs and after
 * it finishes, so it never drops below the fold when it completes; resuming or
 * retrying one moves it back to the top. Entries without a start stamp follow,
 * newest first-seen first.
 */
export function rankJobs<T extends Ranked>(
  jobs: T[],
  seenAt: ReadonlyMap<string, number>,
  restartedAt?: ReadonlyMap<string, number>,
): T[] {
  return [...jobs].sort((left, right) => {
    const leftTime = jobStartTime(left, restartedAt);
    const rightTime = jobStartTime(right, restartedAt);
    if (leftTime !== rightTime) {
      if (leftTime === null) return 1;
      if (rightTime === null) return -1;
      return rightTime - leftTime;
    }
    return (seenAt.get(right.id) ?? 0) - (seenAt.get(left.id) ?? 0);
  });
}

/** DownloadManager owns these acquisition IDs; this classification grants no route authority. */
function isDownloadManagerJob(job: DownloadJob): boolean {
  return job.transport === undefined || job.transport === "http" ||
    job.transport === "quic-library" || job.transport === "unknown";
}

const NOTIFICATION_ACTIVE = [
  "queued", "connecting", "resolving", "initializing", "metadata",
  "checking", "downloading", "verifying", "publishing",
];

/** Match the native DownloadManager notification phases, not transport-derived permissions. */
export function downloadNotificationPhase(job: DownloadJob): DownloadNotificationAction["phase"] | null {
  if (job.transport !== "torrent" && !isDownloadManagerJob(job)) return null;
  if (NOTIFICATION_ACTIVE.includes(job.status)) return "active";
  if (job.status === "paused") return "paused";
  if (job.status === "failed" || job.status === "error") return "failed";
  return null;
}

/**
 * What the active download card says about the transfer itself. Bytes come
 * from the job directly; this only decides which rate/note belongs beside them.
 */
export type DownloadLiveSummary =
  | { kind: "active"; bytesPerSecond: number; etaSeconds: number | null }
  | { kind: "paused" }
  | { kind: "verifying" };

export function downloadLiveSummary(job: DownloadJob): DownloadLiveSummary | null {
  if (job.status === "paused") return { kind: "paused" };
  if (job.status === "verifying") return { kind: "verifying" };
  if (
    job.error ||
    ["completed", "complete", "seeding", "failed", "error", "cancelled", "canceled"].includes(job.status)
  ) return null;
  return {
    kind: "active",
    bytesPerSecond: Number.isFinite(job.speed) ? Math.max(0, job.speed) : 0,
    etaSeconds: job.etaSeconds != null && Number.isFinite(job.etaSeconds) && job.etaSeconds >= 0
      ? job.etaSeconds
      : null,
  };
}

export type RemainingTimeUnit = "seconds" | "minutes" | "hours";

/** Time remaining only needs the coarsest unit that a person can act on. */
export function remainingTime(seconds: number): { count: number; unit: RemainingTimeUnit } {
  if (seconds < 60) return { count: Math.max(1, Math.ceil(seconds)), unit: "seconds" };
  if (seconds < 3600) return { count: Math.ceil(seconds / 60), unit: "minutes" };
  return { count: Math.ceil(seconds / 3600), unit: "hours" };
}

/**
 * A crash-interrupted pause outranks the informational error saved with it.
 * Capacity/space confirmation stays first; only then may an error paint Failed.
 */
export type DownloadStatusDecision =
  | { kind: "confirmation" }
  | { kind: "cancelled" }
  | { kind: "paused"; interrupted: boolean }
  | { kind: "verifying" }
  | { kind: "failed" }
  | { kind: "seeding" }
  | { kind: "completed" }
  | { kind: "unavailable" }
  | { kind: "preparing" }
  | { kind: "status"; status: string };

export function downloadStatusDecision(
  job: DownloadJob | MapJob,
  ready: boolean,
  diskWarning: boolean,
): DownloadStatusDecision {
  if (diskWarning) return { kind: "confirmation" };
  if (job.status === "cancelled") return { kind: "cancelled" };
  if (job.status === "paused") {
    return { kind: "paused", interrupted: "interrupted" in job && job.interrupted === true };
  }
  // Local checksum/publication work needs no network and must never read as a
  // stalled download or be repainted by a stale informational error.
  if (job.status === "verifying") return { kind: "verifying" };
  if (job.error || job.importError || job.imports?.some((entry) => ["failed", "cancelled"].includes(entry.state))) {
    return { kind: "failed" };
  }
  if (ready) return "seeding" in job && job.seeding ? { kind: "seeding" } : { kind: "completed" };
  if (job.importState === "imported") return { kind: "unavailable" };
  if (["complete", "completed", "seeding", "composing", "validating"].includes(job.status)) {
    return { kind: "preparing" };
  }
  return { kind: "status", status: job.status };
}

/** Only committed, verified acquisition receipts authorize opening an installed book. */
export function readyDownloadBooks(job: DownloadJob | MapJob, books: Book[]): Book[] {
  if (!["complete", "completed", "seeding"].includes(job.status) ||
    job.error || job.importError || job.importState !== "imported" ||
    !job.imports?.length || job.imports.some((entry) => entry.state !== "imported")) return [];
  const paths = new Set(job.imports.flatMap((entry) => entry.paths));
  return books.filter((book) => book.available && paths.has(book.path));
}

/** Candidate hints only; native publication proof authorizes every deletion. */
export function cleanupDownloadBooks(job: DownloadJob | MapJob, books: Book[]): Book[] {
  const paths = new Set(job.imports?.filter((entry) => entry.state === "imported").flatMap((entry) => entry.paths));
  if ("path" in job) {
    if (job.path) paths.add(job.path);
  } else {
    for (const path of job.completedPaths ?? []) paths.add(path);
    if (isDownloadManagerJob(job) && ["complete", "completed"].includes(job.status)) {
      paths.add(job.destination);
    }
  }
  return books.filter((book) => paths.has(book.path));
}

/** A retained map warning may only continue the configuration that created it. */
export function mapDownloadConfiguration(settings: Settings, books: Book[]): string {
  return JSON.stringify([
    settings.mapCatalogUrl,
    settings.mapProxyUrl,
    settings.downloadDirectory,
    books.map((book) => [book.id, book.installationId, book.path, book.available]),
  ]);
}
