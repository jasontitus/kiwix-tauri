export interface MediaPlaybackPosition {
  frames: number[];
  document: string;
  source: string;
  index: number;
  time: number;
  duration: number;
}
export interface MediaCheckpointAuthority {
  bookId: string;
  installationId: string;
  article: string;
  privacyGeneration: number;
  privacyPolicyGeneration: number;
}
export interface MediaCheckpointOwner {
  tabId: string;
  bookId: string;
  installationId: string;
  article: string;
  sourceIdentity: string;
  privacyPolicyGeneration: number;
}
const MAX_POSITIONS = 16;
function sameOwner(left: MediaCheckpointOwner, right: MediaCheckpointOwner): boolean {
  return left.tabId === right.tabId && left.bookId === right.bookId &&
    left.installationId === right.installationId && left.article === right.article &&
    left.sourceIdentity === right.sourceIdentity && left.privacyPolicyGeneration === right.privacyPolicyGeneration;
}

export function validMediaPlaybackPositions(value: unknown): value is MediaPlaybackPosition[] {
  return Array.isArray(value) && value.length <= MAX_POSITIONS && value.every((position) =>
    position && typeof position === "object" &&
    typeof position.document === "string" && position.document.startsWith("/") && position.document.length <= 8192 &&
    typeof position.source === "string" && position.source.startsWith("/") && position.source.length <= 8192 &&
    Array.isArray(position.frames) && position.frames.length <= 32 &&
    position.frames.every((index: unknown) => typeof index === "number" && Number.isSafeInteger(index) && index >= 0 && index < 64) &&
    !position.document.startsWith("/.kiwix/") && !position.source.startsWith("/.kiwix/") &&
    Number.isSafeInteger(position.index) && position.index >= 0 && position.index < MAX_POSITIONS &&
    typeof position.time === "number" && Number.isFinite(position.time) && position.time >= 0 &&
    typeof position.duration === "number" && Number.isFinite(position.duration) && position.duration > 0 &&
    position.duration <= 100000000 && position.time <= position.duration);
}

/** Root-window memory only. Reader generation is deliberately not authority:
 * background retirement can carry data into a new authenticated reader, while
 * every policy, tab, article or installation/source replacement fails closed.
 */
export class ReaderMediaCheckpoint {
  private entry: { owner: MediaCheckpointOwner; positions: MediaPlaybackPosition[] } | null = null;

  record(owner: MediaCheckpointOwner, positions: MediaPlaybackPosition[]): void {
    if (!validMediaPlaybackPositions(positions) || positions.length === 0) return;
    const previous = this.entry;
    const same = previous && sameOwner(owner, previous.owner);
    const merged = new Map<string, MediaPlaybackPosition>();
    if (same) for (const position of previous.positions)
      merged.set(JSON.stringify([position.frames, position.document, position.index]), position);
    for (const position of positions) {
      const key = JSON.stringify([position.frames, position.document, position.index]);
      merged.delete(key);
      merged.set(key, { frames: [...position.frames], document: position.document, source: position.source,
        index: position.index, time: position.time, duration: position.duration });
    }
    this.entry = { owner: { ...owner }, positions: [...merged.values()].slice(-MAX_POSITIONS) };
  }

  read(owner: MediaCheckpointOwner): MediaPlaybackPosition[] {
    const entry = this.entry;
    if (!entry) return [];
    if (!sameOwner(owner, entry.owner)) {
      this.entry = null;
      return [];
    }
    return entry.positions.map((position) => ({ ...position, frames: [...position.frames] }));
  }

  clear(): void {
    this.entry = null;
  }
}
