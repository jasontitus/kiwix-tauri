import type { PrivacyContext } from "./api";

/** Bounded, self-contained capture of the authenticated current article. */
export interface LiveArticleSnapshot {
  html: string;
  width: number;
  height: number;
  documentUrl: string;
}

export interface ArchiveMetadata {
  id: string;
  title: string;
  description: string;
  language: string;
  creator: string;
  publisher: string;
  date: string;
  name: string;
  tags: string[];
  size: number;
  articleCount: number;
  mediaCount: number;
  hasFulltextIndex: boolean;
  hasTitleIndex: boolean;
  /** Archive-relative location (percent-encoded libzim path), never a raw URL. */
  mainPath: string;
}
export interface Book extends ArchiveMetadata {
  installationId: string;
  path: string;
  sourceReference?: { reference: string; displayName: string; persistent: boolean } | null;
  addedAt: string;
  available: boolean;
  managed: boolean;
  integrity?: {
    status: "valid" | "invalid";
    checkedAt: string;
    installationId: string;
    identity: unknown;
  } | null;
}
export interface Highlight {
  id: string;
  documentPath: string;
  exact: string;
  prefix: string;
  suffix: string;
  start: number;
  end: number;
  comment: string;
}
export interface Bookmark {
  id: string;
  bookId: string;
  path: string;
  title: string;
  note: string;
  tags: string[];
  highlights: Highlight[];
  createdAt: string;
}
export interface HistoryEntry {
  id: string;
  bookId: string;
  path: string;
  title: string;
  visitedAt: string;
  scroll: number;
}
export interface TabLocation {
  bookId: string;
  path: string;
  title: string;
  scroll: number;
  /** Request a restricted reader session for this first open. */
  restricted?: boolean;
}
export interface Tab {
  id: string;
  bookId: string | null;
  path: string;
  title: string;
  scroll: number;
  restricted?: boolean;
  back: TabLocation[];
  forward: TabLocation[];
}

/** Ordered native reference changes. These move reading locations, not authority. */
export interface ReferenceTransition {
  revision: number;
  transactionId: string;
  fromBookId: string;
  toBookId: string;
  fromInstallationId: string;
  toInstallationId: string;
}
/** A ZIM file discovered by the Android device scan. */
export interface DeviceArchive {
  /** Physical identity: volume + size + modification time. */
  id: string;
  name: string;
  names: string[];
  size: number;
  modified: number;
  volume: string;
  location: string;
  sources: string[];
  readable: boolean;
  split: boolean;
  /** Opaque native open token; never a path or URI. */
  token: string;
  problem?: string | null;
  inLibrary: boolean;
}
export interface DeviceScan {
  entries: DeviceArchive[];
  limitations: string[];
}
export interface LibraryFilters {
  languages: string[];
  categories: string[];
  content: string[];
}
export interface SpellingIndexStatus {
  bookId: string;
  installationId: string;
  sourceKey: string;
  state: "missing" | "rebuild" | "building" | "ready" | "cancelled" | "error";
  processed: number;
  total: number;
  words: number;
  error: string | null;
  cancellable?: boolean;
}
export interface SpellingCorrection {
  originalQuery: string;
  correctedQuery: string;
}
export interface SpeechPreference {
  voice: string;
  rate: number;
}
export interface Settings {
  theme: "system" | "light" | "dark";
  locale: string;
  downloadDirectory: string;
  monitorDirectories: string[];
  /** Runtime launch policy; never a saved or client-selectable preference. */
  portableMode: boolean;
  lastImportDirectory: string | null;
  lastSaveDirectory: string | null;
  restoreSession: boolean;
  /** Open reader "new tab" link actions without switching away from the current article. */
  openNewTabInBackground: boolean;
  zoom: number;
  bookZoom: Record<string, number>;
  libraryFilters: LibraryFilters;
  catalogFilters: LibraryFilters;
  catalogFiltersCustomized: boolean;
  trashDeletedFiles: boolean;
  spellingEnabled: boolean;
  showSearchSnippets: boolean;
  /** Render recoverable failures as bands; blocking failures always render. */
  showTechnicalErrors: boolean;
  fontSize: number;
  fontFamily: "original" | "serif" | "sans";
  /** Article column width: the archive's own layout, a readable measure, or the full window. */
  readerWidth: "original" | "comfortable" | "full";
  readerTheme: "original" | "light" | "dark" | "sepia";
  expandSections: boolean;
  linkPreviews: boolean;
  catalogUrl: string;
  mapCatalogUrl: string;
  mapProxyUrl: string;
  externalLinks: "ask" | "deny";
  historyEnabled: boolean;
  /** Canonical BCP 47 language keys; "*" preserves migrated global defaults. */
  speechPreferences: Record<string, SpeechPreference>;
  privateMode: boolean;
  readOnly: boolean;
  downloadLimit: number;
  uploadLimit: number;
  /** Apple-only Nearby preference: prefer AWDL/peer-to-peer before infrastructure. */
  directConnection: boolean;
  autoResumeDownloads: boolean;
  allowMeteredDownloads: boolean;
  completionNotifications: boolean;
  backupArchives: boolean;
  opdsSources: string[];
  sharingBookIds: string[];
  /** Library archive IDs pinned first, in pin order; at most three. */
  pinnedBookIds: string[];
  sharingName: string;
  diagnosticsEnabled: boolean;
  /** Desktop: let local AI agents (MCP) search and read the library. */
  agentConnector: boolean;
  logLevel: string;
}
export type SharingBinding =
  | { mode: "loopback" | "ipv4All" | "ipv6All" | "dualStackAll" | "hotspot" }
  | { mode: "interface"; address: string };
export type SharingScope = "selected" | "library";
export interface SharingInterface {
  name: string;
  address: string;
  family: "ipv4" | "ipv6";
}
export interface SharingStatus {
  running: boolean;
  scope: SharingScope;
  url: string | null;
  port: number | null;
  binding: SharingBinding | null;
  bindings: string[];
  urls: string[];
  books: { id: string; port: number; urls: string[] }[];
  token: string | null;
  bookIds: string[];
  catalogUrl: string | null;
  catalogUrls: string[];
  libraryCount: number;
  sessionId?: string | null;
  locked?: boolean;
  discoveryError?: string | null;
  /**
   * A share bound to one interface address whose address is no longer on this
   * device: the links cannot work until it returns or sharing is restarted.
   * Absent for other bindings, whose links follow the live addresses.
   */
  addressLost?: boolean;
  /**
   * A share on all interfaces while this device has no network address: it
   * is still running, but only this device can open its links. It serves the
   * next network the device joins without being restarted.
   */
  localOnly?: boolean;
}
export interface Snapshot {
  books: Book[];
  bookmarks: Bookmark[];
  history: HistoryEntry[];
  recentSearches: string[];
  tabs: Tab[];
  activeTab: string | null;
  activeSurface: "library" | "catalog" | "downloads" | "nearby" | "maps" | "bookmarks" | "history" | "settings" | "reader";
  /** Runtime-only: this window was opened by the explicit restore command. */
  sessionRestored?: boolean;
  settings: Settings;
  sharing: SharingStatus;
  notifications: { available: boolean; reason?: string };
  /** Desktop "Look up in Kiwix": the latest external search for this window. */
  launchSearch?: { launchId: string; targetWindow: string; query: string } | null;
  launchLocations?: (TabLocation & {
    launchId: string;
    targetWindow: string;
  })[];
  privacyGeneration: number;
  privacyPolicyGeneration: number;
  referenceRevision: number;
  referenceTransitions: ReferenceTransition[];
  startupWarnings?: string[];
}
export interface SearchHit {
  bookId: string;
  path: string;
  title: string;
  snippet: string;
  score: number;
}
export interface SearchResults {
  hits: SearchHit[];
  total: number;
  hasMore: boolean;
  errors: { bookId: string; error: string }[];
  corrections?: SpellingCorrection[];
  spellingStatus?: SpellingIndexStatus[];
}
/** One title-suggestion row; never carries article text or snippets. */
export interface TitleSuggestion {
  bookId: string;
  path: string;
  title: string;
}
export interface SuggestionPage {
  suggestions: TitleSuggestion[];
  hasMore: boolean;
  /** Opaque cursor for the next page; bound to prefix and archive scope. */
  nextToken: string | null;
  errors: { bookId: string; error: string }[];
}
export interface ReaderLocation {
  bookId: string;
  path: string;
  title: string;
  url: string;
  origin: string;
  baseUrl: string;
  retirementUrl: string;
  retirementProof: string;
  mime: string;
  /** The requested entry is absent; `path` is the archive's main document. */
  missing?: boolean;
  /** The archive has no main page; `path` is a random article. */
  noMainPage?: boolean;
  /** Session-scoped: archive scripts/forms/network are blocked by CSP. */
  restricted?: boolean;
}
export interface CatalogEntry {
  id: string;
  title: string;
  description: string;
  language: string;
  creator: string;
  publisher: string;
  date: string;
  name: string;
  tags: string[];
  size: number;
  articleCount: number;
  mediaCount: number;
  url: string;
  thumbnailUrl: string | null;
  torrentUrl?: string | null;
  torrentResolverUrl?: string | null;
}
export interface CatalogCacheStatus {
  status: "network" | "revalidated" | "offline";
  stale: boolean;
  stored: boolean;
  error: string | null;
}
export interface CatalogPage {
  entries: CatalogEntry[];
  next: string | null;
  total: number | null;
  cache?: CatalogCacheStatus;
}
export interface MapVersion {
  lang: string;
  label: string;
  name: string;
  recipe: string;
  /** Complete archive bytes from the index, or null until recipe metadata provides it. */
  size: number | null;
  /** Catalog estimate for incremental language data, not the archive or a fresh download total. */
  downloadBytes: number;
  md5: string;
}
export interface MapRecipeMetadata {
  regionId: string;
  language: string;
  name: string;
  md5: string;
  assembledBytes: number;
  /** Ordered source-chunk transfer with no retained payload; excludes JSON and retries. */
  coldDownloadBytes: number;
  /** Same estimate with every base chunk verified locally; excludes JSON and retries. */
  reusableBaseDownloadBytes: number;
}
export interface MapRegion {
  id: string;
  title: string;
  base: { file: string; size: number; download: string; url?: string };
  menu: MapVersion[];
}
export interface MapPreparation {
  token: string;
  regionId: string;
  language: string;
  name: string;
  md5: string;
  version: string;
  installed: Book | null;
  sameVersion: boolean;
}
export interface MapStartRequest {
  regionId: string;
  language: string;
  configuration: string;
  /** Catalog selection captured before preparation; never renewed by a retry. */
  selection: string;
  decisionToken: string;
  replaceExisting: boolean;
  privacy: PrivacyContext;
}
export type DownloadTarget =
  | { kind: "download"; id: string }
  | { kind: "map"; id: string; request?: MapStartRequest };
export interface DownloadAdmission {
  privacy: PrivacyContext;
  focus: boolean;
}
export interface AcquisitionImport {
  receipt: string;
  paths: string[];
  state: "queued" | "verifying" | "ready" | "imported" | "failed" | "cancelled";
  error?: string;
  verifiedBytes?: number;
  verifiedTotal?: number;
}
export interface AcquisitionVerification {
  importState?: AcquisitionImport["state"] | "imported";
  imports?: AcquisitionImport[];
  importError?: string | null;
}
export interface DownloadNotificationAction {
  requestId: string;
  id: string;
  action: "pause" | "resume" | "cancel" | "retry";
  phase: "active" | "paused" | "failed";
  privacyEpoch: number;
  privacyGeneration: number;
}
export interface DownloadJob extends AcquisitionVerification {
  historyWarning?: string;
  id: string;
  url: string;
  fileName: string;
  destination: string;
  status: string;
  /** RFC3339 creation/activity stamps; torrent jobs serialize without them. */
  createdAt?: string;
  updatedAt?: string;
  /** True only for a pause left by shutdown or a crash; a manual pause is false. */
  interrupted?: boolean;
  downloaded: number;
  total: number | null;
  error: string | null;
  speed: number;
  etaSeconds?: number | null;
  /** Recorded wire provenance; unknown means a legacy acquisition has no proof. */
  transport?: "http" | "quic-library" | "unknown" | "torrent" | "localswarm" | "map";
  seeding?: boolean;
  uploaded?: number;
  uploadSpeed?: number;
  completedPaths?: string[];
  requiresPin?: boolean;
  admissionGeneration?: number;
  /** Present when this job seeds an existing library archive. */
  shared?: boolean;
  magnet?: string | null;
  torrentPath?: string | null;
  /** Bounded route receipt for a Nearby transfer (interface/local address). */
  route?: string;
}
interface NearbyPeerPresentation {
  id: string;
  /** Discovery selection identity; not authenticated personal identity. */
  peerId: string;
  publisherName?: string;
}
export interface LocalSwarmArchivePeer extends NearbyPeerPresentation {
  transport: "quic";
  swarmId: string;
  name: string;
  totalBytes: number;
  chunkCount: number;
  host: string;
  port: number;
  locked: boolean;
  addresses?: string[];
}
export interface LocalSwarmLibraryPeer extends NearbyPeerPresentation {
  transport: "quic-library";
  discovery?: "network" | "bluetooth";
  name?: string;
  librarySessionId?: string;
  libraryGeneration?: number;
  libraryCount?: number;
  host?: string;
  port?: number;
  addresses?: string[];
}
export type LocalSwarmPeer = LocalSwarmArchivePeer | LocalSwarmLibraryPeer;
export interface SharedLibraryBook {
  id: string;
  title: string;
  fileName: string;
  size: number;
  /** Backend-computed channel/session identity; never reconstructed from metadata. */
  downloadIdentity: string;
  archiveId?: string;
}
export interface SharedLibraryInventory {
  receiptId: string;
  sessionId: string;
  name: string;
  books: SharedLibraryBook[];
}
export type LocalSwarmConnection = {
  host: string;
  port: number;
  swarmId: string;
  pin?: string;
  /** Selected native discovery record; endpoint addresses are resolved natively. */
  discoveryId?: string;
};
export interface LocalSwarmSeed {
  id: string;
  bookId: string;
  name: string;
  swarmId: string;
  totalBytes: number;
  chunkCount: number;
  port: number;
  locked: boolean;
  uploaded: number;
  uploadSpeed: number;
  peerCount: number;
  addresses?: string[];
}
export type LocalSwarmHotspot =
  | {
      phase: "active";
      ssid: string;
      passphrase: string;
      qr: string;
      /** The access-point address a joined peer reaches this phone on. */
      address?: string;
    }
  | { phase: "starting" | "stopping" };
export interface NearbyLibrarySharing {
  running: boolean;
  sessionId?: string | null;
  publisherName?: string | null;
  libraryCount: number;
  uploaded: number;
  peerCount: number;
  error?: string | null;
}
export interface PublicNearbyStatus {
  supported: boolean;
  nativeReady: boolean;
  discovery: boolean;
  consumer: {
    owner: string;
    peerId: string | null;
    phase: string;
    error?: string | null;
  } | null;
  /** The Bluetooth side of "Share all"; phase "failed" means it is not advertising. */
  publisher?: {
    owner: string;
    peerId: string | null;
    phase: string;
    error?: string | null;
  } | null;
  /** Why Bluetooth finding stopped by itself; a failed connection is on `consumer`. */
  error?: string | null;
}
export interface LocalSwarmStatus {
  discovery: boolean;
  peers: LocalSwarmPeer[];
  seeds: LocalSwarmSeed[];
  error: string | null;
  /** Android-only local-only hotspot credentials; null or absent elsewhere. */
  hotspot?: LocalSwarmHotspot | null;
  librarySharing?: NearbyLibrarySharing;
  nearby?: PublicNearbyStatus;
}
export interface LocalSwarmInspection {
  admissionGeneration: number;
  swarmId: string;
  /** Verified ZIM-header identity, inspected before creating a transfer. */
  archiveId?: string;
  name: string;
  totalBytes: number;
  chunkCount: number;
  files: { path: string; sizeBytes: number }[];
  supported: boolean;
  connectedHost?: string;
  route?: string;
  reason?: string;
}
export interface MapCatalog {
  format: string;
  regions: MapRegion[];
}
export interface MapJob extends AcquisitionVerification {
  id: string;
  status: string;
  /** RFC3339 start of this map download; absent in older job journals. */
  createdAt?: string;
  regionId: string;
  language: string;
  written: number;
  total: number;
  incoming: number;
  incomingTotal: number;
  /**
   * Bytes per second of the block being fetched from the network (or of the
   * last one while the next is prepared); 0 or absent when the latest block
   * came from the cache or a local map, and when idle. There is no ETA.
   */
  speed?: number;
  /** Unique required recipe blocks; absent in older job journals. */
  blocksTotal?: number | null;
  blocksReady?: number | null;
  blocksDownloaded?: number | null;
  /** Native-verified durable checkpoint; absence never implies resumability. */
  resumable?: boolean;
  /** Native permission to hide this job without orphaning retained files. */
  canRemoveFromList?: boolean;
  /**
   * Files Kiwix kept because it could not prove they were its own to delete.
   * Pass one unchanged as `path` to `map_forget_recovery` to dismiss its
   * warning (the file is never touched). Absent when there are none, and in
   * private or read-only mode.
   */
  recoveryPaths?: string[];
  error: string | null;
  path: string | null;
}
export interface CatalogSourceSnapshot {
  sourceId: string;
  sourceUrl: string;
  locale: string;
  generation: number;
  completeness: "partial" | "traversed";
  stale: boolean;
  stored: boolean;
  pages: {
    requestUrl: string;
    responseUrl: string;
    representationDigest: string;
    next: string | null;
    entryIds: string[];
    generation: number;
    stale: boolean;
    status: string;
  }[];
  entries: { sourceId: string; entryId: string; entry: CatalogEntry }[];
  error: string | null;
}
export interface EditionSelection {
  sourceId: string;
  sourceUrl: string;
  entryId: string;
  catalogGeneration: number;
  entry: CatalogEntry;
}
export interface ContentUpdates {
  updates: {
    bookId: string;
    installationId: string;
    selection: EditionSelection;
    associationRequired: boolean;
    /** Native policy: bound series with exactly one newer candidate. */
    bulkEligible: boolean;
    reason?: string;
  }[];
  sources: CatalogSourceSnapshot[];
  errors: string[];
}
export interface EditionPreparation {
  token: string;
  installed: Book;
  selection: EditionSelection;
  intentId: string | null;
}
export interface EditionIntent {
  id: string;
  selection: EditionSelection;
  target: Book;
  destination: string;
  downloadDirectory: string;
  downloadId: string | null;
  receipt: string | null;
  status: "reserved" | "downloading" | "ready" | "committed" | "cancelled" | "needsAuthorization";
  verified: Book | null;
  transactionId: string | null;
  needsAuthorization: boolean;
  error?: string | null;
}
export interface EditionTransaction {
  id: string;
  intentId: string;
  original: Book;
  replacement: Book;
  revision: number;
  rolledBackInstallationId: string | null;
  rollbackBook: Book | null;
  /** The retained original was deleted or released; it cannot be rolled back to or deleted again. */
  originalDiscarded: boolean;
}
/** Reply of `edition_discard_original`. */
export interface EditionOriginalDiscarded {
  transaction: EditionTransaction;
  /** What happened to the original's file. */
  originalFile: "deleted" | "trashed" | "missing" | "external";
}
export interface EditionStatus {
  intents: EditionIntent[];
  transactions: EditionTransaction[];
}
/** A signed desktop update announced by the update channel. */
export interface AvailableUpdate {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
}
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
  phase: "downloading" | "verifying" | "installing" | string;
}
export interface UpdateStatus {
  platform: "macos" | "windows" | "linux" | "ios" | "android" | "other" | string;
  supported: boolean;
  management: "self" | "app-store" | "play-store" | "manual" | string;
  storeName: string | null;
  explanation: string;
  channel: string;
  currentVersion: string;
  configured: boolean;
  configurationNote: string | null;
  blockedReason: string | null;
  lastChecked: string | null;
  automaticChecks: boolean;
  nextCheckAt: string | null;
  lastAvailableVersion: string | null;
  lastError: string | null;
  busy: boolean;
  available: AvailableUpdate | null;
  installBlockedReason: string | null;
  progress: UpdateProgress | null;
}
export interface ImportResult {
  books: Book[];
  errors: { path: string; error: string }[];
}
