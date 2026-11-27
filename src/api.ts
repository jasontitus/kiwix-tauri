import { invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { isSuspendedError, parseProviderDocumentError, parseSuspendedError, SuspendedError, UserCancelledError } from "./error-visibility";
import {
  ForegroundQueue,
  retrySuspended,
  type ForegroundGate,
} from "./suspended-retry";
import type { DownloadJob, DownloadTarget, LiveArticleSnapshot, SearchResults, Settings, Snapshot, SpeechPreference, SuggestionPage } from "./types";

export const isNative = isTauri();
// TAURI_ENV_PLATFORM is only set when the bundle is produced by the Tauri CLI.
// A frontend built straight into dist leaves it undefined, which previously made
// an iPhone identify as desktop and invoke desktop-only commands. Fall back to
// the webview's own user agent so that mistake cannot reach a phone again.
const agent = typeof navigator === "undefined" ? "" : navigator.userAgent;
const buildPlatform = import.meta.env.TAURI_ENV_PLATFORM ??
  (/android/i.test(agent) ? "android"
    : /iphone|ipad|ipod/i.test(agent) || (/macintosh/i.test(agent) && (navigator.maxTouchPoints ?? 0) > 1)
      ? "ios" : undefined);
export const isMobile = isNative && (buildPlatform === "ios" || buildPlatform === "android");
export const isDesktop = isNative && !isMobile;
export const isIOS = isNative && buildPlatform === "ios";
export const isAndroid = isNative && buildPlatform === "android";
export const isWindows = isDesktop &&
  (buildPlatform === "windows" || (!buildPlatform && /windows/i.test(agent)));
export const isMacos =
  isDesktop &&
  (buildPlatform === "macos" ||
    buildPlatform === "darwin" ||
    (!buildPlatform && /macintosh|mac os x/i.test(agent)));
/** Platforms with a system share sheet wired to `share_book`. */
export const canShareSystem = isNative && (isIOS || isAndroid || isMacos);
/** Native renderers return a shareable PDF on Apple and Android. */
export const canShareArticlePdf = isNative && (isMobile || isMacos);
/** Desktop platforms present their native physical-printer dialog. */
export const canPrintArticlePdf = isDesktop;

// Capture this document's source once: a queued call must never acquire the
// authority of a replacement Activity or label-routed WebviewWindow.
const androidDocumentSource =
  isAndroid
    ? (() => {
        const source = window as Window & {
          readonly __KIWIX_ANDROID_WINDOW_INCARNATION__?: string;
          readonly __KIWIX_ANDROID_ACTIVITY_SOURCE__?: {
            generation(): string;
          };
        };
        return {
          androidWindowIncarnation:
            source.__KIWIX_ANDROID_WINDOW_INCARNATION__ ?? null,
          androidActivityGeneration:
            source.__KIWIX_ANDROID_ACTIVITY_SOURCE__?.generation() ?? null,
        };
      })()
    : null;
export type ReaderAppearance = Pick<
  Settings,
  | "zoom"
  | "fontSize"
  | "fontFamily"
  | "readerWidth"
  | "readerTheme"
  | "speechPreferences"
>;
const appearanceKeys = [
  "zoom",
  "fontSize",
  "fontFamily",
  "readerWidth",
  "readerTheme",
] as const;
type PreferenceHandlers = {
  onSaved: () => void;
  onError: (error: unknown) => void;
};
type PendingPreference = {
  value: ReaderAppearance[keyof ReaderAppearance];
  revision: number;
  handlers: PreferenceHandlers;
};
type PreferenceEdits = Partial<
  Record<keyof ReaderAppearance, PendingPreference>
>;
type ReaderPreferenceState = {
  confirmed: ReaderAppearance | null;
  pending: Partial<ReaderAppearance>;
  privateMode: boolean;
  readOnly: boolean;
  bookZoom: Record<string, number>;
  pendingBookZoom: Record<string, number | null>;
};
let privacyGeneration: number | null = null;
// Authority retires reader IPC on background; only policy changes cancel
// unsaved appearance/filter edits and advance their optimistic queue epoch.
let privacyPolicyGeneration: number | null = null;
/** The native generation check and the shell guards share one boundary message. */
const PRIVACY_TRANSITION = "Browsing session expired after a privacy transition";

/** A refusal that only a fresh privacy generation can clear, never a user error. */
export function isPrivacyTransitionError(value: unknown): boolean {
  return (value instanceof Error ? value.message : String(value)) === PRIVACY_TRANSITION;
}

const checkpointDurabilityCommands: Record<string, true> = {
  save_session: true, save_settings: true, set_speech_preference: true,
  set_book_zoom: true, reader_storage_write: true, record_visit: true,
  sensor_revoke: true, sensor_reset: true,
};
let mobileBackgroundCheckpoint: { id: string } | undefined;

/** Only the trusted root scopes a finite native checkpoint's durability work. */
export async function withMobileBackgroundCheckpoint<T>(
  id: string,
  operation: () => Promise<T>,
): Promise<T> {
  if (!isMobile) return operation();
  if (mobileBackgroundCheckpoint)
    throw new SuspendedError("The previous background checkpoint has not finished", false);
  const checkpoint = { id };
  mobileBackgroundCheckpoint = checkpoint;
  try {
    return await operation();
  } finally {
    if (mobileBackgroundCheckpoint === checkpoint) mobileBackgroundCheckpoint = undefined;
  }
}

// A suspended command is a lifecycle gate. Every waiter is retried once when
// the native lifecycle reports an interactive foreground; duplicate signals
// (focus, pageshow, visibilitychange, the native event) are no-ops because the
// queue is drained by the first one.
const foreground = new ForegroundQueue();
let foregroundListening = false;
let foregroundFallback: number | undefined;

function clearForegroundFallback() {
  if (foregroundFallback === undefined) return;
  window.clearTimeout(foregroundFallback);
  foregroundFallback = undefined;
}

function nativeForeground() {
  clearForegroundFallback();
  foreground.resume("native");
}

function scheduleForegroundFallback(delay: number) {
  if (foregroundFallback !== undefined) return;
  foregroundFallback = window.setTimeout(() => {
    foregroundFallback = undefined;
    foreground.resume("fallback");
  }, delay);
}

function installForegroundListeners() {
  if (foregroundListening) return;
  foregroundListening = true;
  const visible = () => {
    // Give the native lifecycle time to finish resuming transfers before a
    // fallback retry; a delivered native event cancels this timer.
    if (document.visibilityState !== "hidden") scheduleForegroundFallback(4000);
  };
  window.addEventListener("focus", visible);
  window.addEventListener("pageshow", visible);
  document.addEventListener("visibilitychange", visible);
  void import("@tauri-apps/api/event")
    .then(({ listen }) => listen("kiwix-mobile-active", nativeForeground))
    .catch(() => {
      // The visibility fallback still retries if the event bridge is missing.
    });
}

const foregroundGate: ForegroundGate = {
  wait() {
    installForegroundListeners();
    const waiting = foreground.wait();
    // The resume event can arrive before the first suspended command installs
    // its listener. If the webview is already visible, the gate is opening
    // now; retry shortly even if the native event was missed.
    if (document.visibilityState === "visible")
      scheduleForegroundFallback(1500);
    return waiting;
  },
};
let preferenceEpoch = 0;
let preferenceRevision = 0;
let settingsRevision = 0;
let privacyChangesPending = 0;
let awaitingPrivacySnapshot = false;
let flushScheduled = false;
let settingsQueue: Promise<void> = Promise.resolve();
// Failures outlive the recovered scheduling queue until the same fields save.
const failedPreferenceWrites = new Map<string, unknown>();
const pendingPreferenceDrains = new Set<() => Promise<void>>();
const pendingPreferenceCutoverDrains = new Set<
  () => Promise<(committed: boolean) => void>
>();
let queuedPreferences: PreferenceEdits = {};
let optimisticPreferences: PreferenceEdits = {};
type BookZoomEdit = {
  value: number | null;
  revision: number;
  handlers: PreferenceHandlers;
};
let queuedBookZoom = new Map<string, BookZoomEdit>();
let optimisticBookZoom = new Map<string, BookZoomEdit>();
let bookZoomFlushScheduled = false;
type SpeechEdit = {
  patch: Partial<SpeechPreference>;
  revision: number;
  handlers: PreferenceHandlers;
};
let queuedSpeech = new Map<string, SpeechEdit>();
let optimisticSpeech = new Map<string, SpeechEdit>();
let speechFlushScheduled = false;
type FilterKey = "libraryFilters" | "catalogFilters";
type FilterValue = Settings["libraryFilters"];
type FilterEdit = {
  value: FilterValue;
  revision: number;
  handlers: PreferenceHandlers;
  extra?: Record<string, unknown>;
};
const filterKeys: FilterKey[] = ["libraryFilters", "catalogFilters"];
let confirmedFilters: Partial<Record<FilterKey, FilterValue>> = {};
let queuedFilters = new Map<FilterKey, FilterEdit>();
let optimisticFilters = new Map<FilterKey, FilterEdit>();
let filterFlushScheduled = false;
let filterFlushTimer: ReturnType<typeof setTimeout> | undefined;
let readerPreferenceState: ReaderPreferenceState = {
  confirmed: null,
  pending: {},
  privateMode: false,
  readOnly: false,
  bookZoom: {},
  pendingBookZoom: {},
};
const preferenceListeners = new Set<() => void>();
let confirmedSettings: Settings | null = null;
const settingsDraftFlushers = new Set<() => void>();

export function getConfirmedSettings(): Settings | null {
  return confirmedSettings;
}

/** Dispatch valid debounced settings before close or a privacy cutover queues. */
export function registerSettingsDraftFlusher(flush: () => void): () => void {
  settingsDraftFlushers.add(flush);
  return () => { settingsDraftFlushers.delete(flush); };
}

function flushSettingsDrafts(): void {
  for (const flush of settingsDraftFlushers) flush();
}

function appearance(settings: Settings): ReaderAppearance {
  return {
    zoom: settings.zoom,
    fontSize: settings.fontSize,
    fontFamily: settings.fontFamily,
    readerWidth: settings.readerWidth,
    readerTheme: settings.readerTheme,
    speechPreferences: settings.speechPreferences,
  };
}
function publishPreferences(): void {
  const pending = Object.fromEntries(
    appearanceKeys
      .filter((key) => optimisticPreferences[key])
      .map((key) => [key, optimisticPreferences[key]!.value]),
  ) as Partial<ReaderAppearance>;
  if (optimisticSpeech.size) {
    const preferences = { ...readerPreferenceState.confirmed?.speechPreferences };
    for (const [language, edit] of optimisticSpeech)
      preferences[language] = { ...(preferences[language] ?? preferences["*"] ?? { voice: "", rate: 1 }), ...edit.patch };
    pending.speechPreferences = preferences;
  }
  readerPreferenceState = {
    ...readerPreferenceState,
    pending,
    pendingBookZoom: Object.fromEntries(
      [...optimisticBookZoom].map(([id, edit]) => [id, edit.value]),
    ),
  };
  for (const listener of preferenceListeners) listener();
}
function confirmPreferences(settings: Settings): void {
  confirmedSettings = settings;
  confirmedFilters = {
    libraryFilters: settings.libraryFilters,
    catalogFilters: settings.catalogFilters,
  };
  readerPreferenceState = {
    ...readerPreferenceState,
    confirmed: appearance(settings),
    privateMode: settings.privateMode,
    readOnly: settings.readOnly,
    bookZoom: settings.bookZoom,
  };
}
function invalidatePreferences(clearConfirmed = false): void {
  preferenceEpoch++;
  queuedPreferences = {};
  optimisticPreferences = {};
  queuedBookZoom.clear();
  optimisticBookZoom.clear();
  queuedSpeech.clear();
  optimisticSpeech.clear();
  queuedFilters.clear();
  optimisticFilters.clear();
  confirmedFilters = {};
  clearTimeout(filterFlushTimer);
  filterFlushTimer = undefined;
  if (clearConfirmed)
    readerPreferenceState = {
      ...readerPreferenceState,
      confirmed: null,
      bookZoom: {},
    };
  publishPreferences();
}
function enqueueSettings<T>(operation: () => Promise<T>): Promise<T> {
  const result = settingsQueue.then(operation);
  // Recover the queue, not the caller's result: every dispatched failure is reported.
  settingsQueue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}
async function persistPreferences<T>(
  keys: string[],
  operation: () => Promise<T>,
): Promise<T> {
  try {
    const result = await operation();
    for (const key of keys) failedPreferenceWrites.delete(key);
    return result;
  } catch (error) {
    for (const key of keys) failedPreferenceWrites.set(key, error);
    throw error;
  }
}

/** Include independently queued persistence in trusted close/quit and mode drains. */
export function registerPendingPreferenceDrain(
  drain: () => Promise<void>,
): () => void {
  pendingPreferenceDrains.add(drain);
  return () => {
    pendingPreferenceDrains.delete(drain);
  };
}

/** Freeze external writers; committed cutovers must keep retired frames frozen. */
export function registerPendingPreferenceCutoverDrain(
  drain: () => Promise<(committed: boolean) => void>,
): () => void {
  pendingPreferenceCutoverDrains.add(drain);
  return () => {
    pendingPreferenceCutoverDrains.delete(drain);
  };
}

async function collectPendingDrainErrors(): Promise<unknown[]> {
  const results = await Promise.allSettled(
    [...pendingPreferenceDrains].map((drain) => Promise.resolve().then(drain)),
  );
  return results.flatMap((result) =>
    result.status === "rejected" ? [result.reason] : [],
  );
}

export type PrivacyContext = Readonly<{
  generation: number;
  epoch: number;
  privateMode: boolean;
  readOnly: boolean;
}>;

/** Capture at request start, never when a late browsing callback completes. */
export function capturePrivacyContext(): PrivacyContext | null {
  if (
    privacyGeneration === null ||
    privacyChangesPending ||
    awaitingPrivacySnapshot
  )
    return null;
  return {
    generation: privacyGeneration,
    epoch: preferenceEpoch,
    privateMode: readerPreferenceState.privateMode,
    readOnly: readerPreferenceState.readOnly,
  };
}

export function isPrivacyContextCurrent(context: PrivacyContext): boolean {
  return (
    !privacyChangesPending &&
    !awaitingPrivacySnapshot &&
    context.generation === privacyGeneration &&
    context.epoch === preferenceEpoch &&
    context.privateMode === readerPreferenceState.privateMode &&
    context.readOnly === readerPreferenceState.readOnly
  );
}
/** Accepted native picker intent may survive background-only reader retirement.
 * This never authorizes reader IPC: the root must first reconcile native
 * interactivity, refresh the snapshot, and match the returned installation.
 */
export function isPrivacyPolicyContextCurrent(context: PrivacyContext): boolean {
  return (
    !privacyChangesPending &&
    !awaitingPrivacySnapshot &&
    context.epoch === preferenceEpoch &&
    context.privateMode === readerPreferenceState.privateMode &&
    context.readOnly === readerPreferenceState.readOnly
  );
}
type SearchIdentity = { sessionId: string; ownerId: string };
type SearchParameters = {
  bookIds: string[];
  query: string;
  mode: "combined" | "title" | "fulltext";
  offset: number;
  limit: number;
};
type SuggestionParameters = {
  bookIds: string[];
  prefix: string;
  limit: number;
  /** Continuation from the previous page; bound to prefix and scope natively. */
  token?: string | null;
};
export type SearchRequest = {
  run: (parameters: SearchParameters, privacy: PrivacyContext) => Promise<SearchResults>;
  suggest: (
    parameters: SuggestionParameters,
    privacy: PrivacyContext,
  ) => Promise<SuggestionPage>;
  cancel: () => Promise<void>;
  dispose: () => Promise<void>;
};
let searchDocument: Promise<string> | null = null;

function searchDocumentSession(): Promise<string> {
  if (window.parent !== window)
    throw new Error(t("Search is only available inside the Kiwix window."));
  if (!searchDocument) {
    // Root snapshot bootstrap registers this before any query, retiring the old
    // document's work even when the reloaded shell never searches again.
    searchDocument = request<{ sessionId: string }>("search_session", {})
      .then((result) => result.sessionId)
      .catch((error) => {
        searchDocument = null;
        throw error;
      });
  }
  return searchDocument;
}

/** One mounted consumer owns one native cancellation scope, never a global query. */
export function createSearchRequest(): SearchRequest {
  let owner: Promise<SearchIdentity> | null = null;
  let sequence = 0;
  let disposed = false;
  let active: {
    requestId: number;
    identity?: SearchIdentity;
  } | null = null;
  const identity = () => {
    if (!owner) {
      owner = searchDocumentSession().then(async (sessionId) => {
        const result = await request<{ ownerId: string }>("search_owner", { sessionId });
        return { sessionId, ownerId: result.ownerId };
      }).catch((error) => {
        owner = null;
        throw error;
      });
    }
    return owner;
  };
  const cancel = async () => {
    const previous = active;
    active = null;
    if (previous?.identity) {
      await request("cancel_search", {
        ...previous.identity,
        requestId: previous.requestId,
      });
    }
  };
  const send = async <T>(
    command: "search" | "suggest",
    parameters: SearchParameters | SuggestionParameters,
    privacy: PrivacyContext,
  ): Promise<T> => {
    if (disposed || !isPrivacyContextCurrent(privacy)) throw new Error("Search cancelled");
    if (!Number.isSafeInteger(++sequence)) throw new Error("Search request identity exhausted");
    const cancellation = cancel();
    const current = { requestId: sequence, identity: undefined as SearchIdentity | undefined };
    active = current;
    try {
      await cancellation;
      if (active !== current || disposed || !isPrivacyContextCurrent(privacy))
        throw new Error("Search cancelled");
      const registered = await identity();
      if (active !== current || disposed || !isPrivacyContextCurrent(privacy))
        throw new Error("Search cancelled");
      current.identity = registered;
      return await api<T>(command, {
        ...parameters,
        ...registered,
        requestId: current.requestId,
        privacyGeneration: privacy.generation,
      });
    } finally {
      if (active === current) active = null;
    }
  };
  return {
    run: (parameters, privacy) => send<SearchResults>("search", parameters, privacy),
    suggest: (parameters, privacy) => send<SuggestionPage>("suggest", parameters, privacy),
    cancel,
    async dispose() {
      disposed = true;
      try {
        await cancel();
      } finally {
        const registered = owner && await owner;
        if (registered) await request("search_release", registered);
      }
    },
  };
}
export function subscribeReaderPreferences(listener: () => void): () => void {
  preferenceListeners.add(listener);
  return () => {
    preferenceListeners.delete(listener);
  };
}
export function getReaderPreferences(): ReaderPreferenceState {
  return readerPreferenceState;
}

/** Drain coalesced reading preferences before a trusted native close/quit flush. */
export async function flushPendingPreferences(): Promise<void> {
  flushSettingsDrafts();
  const externalErrors = await collectPendingDrainErrors();
  let drainedQueue: Promise<void>;
  do {
    schedulePreferenceFlush();
    scheduleBookZoomFlush();
    scheduleSpeechFlush();
    scheduleLibraryFilterFlush(true);
    await Promise.resolve(); // Let a just-scheduled batch join the shared queue.
    drainedQueue = settingsQueue;
    await drainedQueue;
    await Promise.resolve(); // Let the batch's completion schedule its successor.
  } while (
    drainedQueue !== settingsQueue ||
    flushScheduled ||
    bookZoomFlushScheduled ||
    speechFlushScheduled ||
    queuedSpeech.size > 0 ||
    filterFlushScheduled ||
    queuedFilters.size > 0 ||
    queuedBookZoom.size > 0 ||
    appearanceKeys.some((key) => queuedPreferences[key])
  );
  const errors = [...failedPreferenceWrites.values(), ...externalErrors];
  if (errors.length)
    throw new AggregateError(
      [...new Set(errors)],
      "Pending preferences could not be saved",
    );
}

function schedulePreferenceFlush(): void {
  if (flushScheduled || !appearanceKeys.some((key) => queuedPreferences[key]))
    return;
  flushScheduled = true;
  queueMicrotask(() => {
    // Freeze the batch when it joins the queue. Reading it only when this
    // operation runs could pull a newer Reader edit ahead of an intervening
    // Settings save and let that older Settings value win.
    const edits = queuedPreferences;
    queuedPreferences = {};
    void enqueueSettings(async () => {
      const keys = appearanceKeys.filter((key) => edits[key]);
      if (!keys.length) return;
      const generation = privacyGeneration;
      const handlers = new Set(keys.map((key) => edits[key]!.handlers));
      const patch = Object.fromEntries(
        keys.map((key) => [key, edits[key]!.value]),
      );
      settingsRevision++;
      try {
        const saved = await persistPreferences(keys, () =>
          request<Settings>("save_settings", {
            settings: patch,
            privacyGeneration: generation,
          }),
        );
        // A queued cutover can cancel drafts, but must not discard a write that
        // actually succeeded in the still-current privacy generation.
        if (generation === privacyGeneration && !awaitingPrivacySnapshot)
          confirmPreferences(saved);
        for (const handler of handlers) handler.onSaved();
      } catch (error) {
        // A retired Reader still owns the error from its dispatched write.
        for (const handler of handlers) handler.onError(error);
      } finally {
        settingsRevision++;
        for (const key of keys) {
          if (optimisticPreferences[key]?.revision === edits[key]!.revision)
            delete optimisticPreferences[key];
        }
        publishPreferences();
      }
    }).finally(() => {
      flushScheduled = false;
      schedulePreferenceFlush();
    });
  });
}

// Appearance fields and language-level speech edits share one authority-scoped queue.
export function saveReaderPreferences(
  patch: Partial<Omit<ReaderAppearance, "speechPreferences">>,
  handlers: PreferenceHandlers,
): boolean {
  if (
    privacyGeneration === null ||
    privacyChangesPending ||
    awaitingPrivacySnapshot ||
    readerPreferenceState.readOnly
  ) {
    handlers.onError(
      new Error(PRIVACY_TRANSITION),
    );
    return false;
  }
  // Preserve intent order: an older Settings debounce must enter the shared
  // queue before this newer Reader edit, rather than overwrite it later.
  flushSettingsDrafts();
  const revision = ++preferenceRevision;
  for (const key of appearanceKeys) {
    const value = patch[key];
    if (value !== undefined)
      queuedPreferences[key] = optimisticPreferences[key] = {
        value,
        revision,
        handlers,
      };
  }
  publishPreferences();
  schedulePreferenceFlush();
  return true;
}

function scheduleSpeechFlush(): void {
  if (speechFlushScheduled || !queuedSpeech.size) return;
  speechFlushScheduled = true;
  queueMicrotask(() => {
    void enqueueSettings(async () => {
      const edits = queuedSpeech;
      queuedSpeech = new Map();
      const epoch = preferenceEpoch;
      for (const [language, edit] of edits) {
        if (epoch !== preferenceEpoch) break;
        const generation = privacyGeneration;
        settingsRevision++;
        try {
          const saved = await persistPreferences(Object.keys(edit.patch).map((field) => `speech:${language}:${field}`), () =>
            request<Settings>("set_speech_preference", {
              language, preference: edit.patch, privacyGeneration: generation,
            }),
          );
          if (generation === privacyGeneration && !awaitingPrivacySnapshot) confirmPreferences(saved);
          edit.handlers.onSaved();
        } catch (error) {
          edit.handlers.onError(error);
        } finally {
          settingsRevision++;
          if (optimisticSpeech.get(language)?.revision === edit.revision) optimisticSpeech.delete(language);
          publishPreferences();
        }
      }
    }).finally(() => {
      speechFlushScheduled = false;
      scheduleSpeechFlush();
    });
  });
}

/** Send only the user's edited properties, never a stale language map. */
export function saveSpeechPreference(
  language: string, patch: Partial<SpeechPreference>, handlers: PreferenceHandlers,
): boolean {
  if (privacyGeneration === null || privacyChangesPending || awaitingPrivacySnapshot || readerPreferenceState.readOnly) {
    handlers.onError(new Error(PRIVACY_TRANSITION));
    return false;
  }
  const revision = ++preferenceRevision;
  const queuedPatch = { ...queuedSpeech.get(language)?.patch, ...patch };
  queuedSpeech.set(language, { patch: queuedPatch, revision, handlers });
  optimisticSpeech.set(language, {
    patch: { ...optimisticSpeech.get(language)?.patch, ...patch }, revision, handlers,
  });
  publishPreferences();
  scheduleSpeechFlush();
  return true;
}

function scheduleBookZoomFlush(): void {
  if (bookZoomFlushScheduled || !queuedBookZoom.size) return;
  bookZoomFlushScheduled = true;
  queueMicrotask(() => {
    void enqueueSettings(async () => {
      const edits = queuedBookZoom;
      queuedBookZoom = new Map();
      const epoch = preferenceEpoch;
      for (const [bookId, edit] of edits) {
        if (epoch !== preferenceEpoch) break;
        const generation = privacyGeneration;
        settingsRevision++;
        try {
          const saved = await persistPreferences([`bookZoom:${bookId}`], () =>
            request<Settings>("set_book_zoom", {
              bookId,
              zoom: edit.value,
              privacyGeneration: generation,
            }),
          );
          if (generation === privacyGeneration && !awaitingPrivacySnapshot)
            confirmPreferences(saved);
          edit.handlers.onSaved();
        } catch (error) {
          edit.handlers.onError(error);
        } finally {
          settingsRevision++;
          if (optimisticBookZoom.get(bookId)?.revision === edit.revision)
            optimisticBookZoom.delete(bookId);
          publishPreferences();
        }
      }
    }).finally(() => {
      bookZoomFlushScheduled = false;
      scheduleBookZoomFlush();
    });
  });
}

/** Per-book edits share the global settings queue; no stale whole-map saves. */
export function saveBookZoom(
  bookId: string,
  zoom: number | null,
  handlers: PreferenceHandlers,
): boolean {
  if (
    privacyGeneration === null ||
    privacyChangesPending ||
    awaitingPrivacySnapshot ||
    readerPreferenceState.readOnly
  ) {
    handlers.onError(
      new Error(PRIVACY_TRANSITION),
    );
    return false;
  }
  if (!bookId || (zoom !== null && !Number.isFinite(zoom))) return false;
  const edit = {
    value: zoom === null ? null : Math.max(0.25, Math.min(5, zoom)),
    revision: ++preferenceRevision,
    handlers,
  };
  queuedBookZoom.set(bookId, edit);
  optimisticBookZoom.set(bookId, edit);
  publishPreferences();
  scheduleBookZoomFlush();
  return true;
}

export function getLibraryFilters(key: FilterKey): FilterValue | undefined {
  return optimisticFilters.get(key)?.value ?? confirmedFilters[key];
}

export function saveLibraryFilters(
  key: FilterKey,
  filters: FilterValue,
  handlers: PreferenceHandlers,
  extra?: Record<string, unknown>,
): boolean {
  if (
    privacyGeneration === null ||
    privacyChangesPending ||
    awaitingPrivacySnapshot ||
    readerPreferenceState.readOnly
  ) {
    handlers.onError(
      new Error(PRIVACY_TRANSITION),
    );
    return false;
  }
  const edit = {
    value: {
      languages: [...filters.languages],
      categories: [...filters.categories],
      content: [...filters.content],
    },
    revision: ++preferenceRevision,
    handlers,
    extra,
  };
  queuedFilters.set(key, edit);
  optimisticFilters.set(key, edit);
  scheduleLibraryFilterFlush();
  return true;
}

function scheduleLibraryFilterFlush(immediate = false): void {
  clearTimeout(filterFlushTimer);
  filterFlushTimer = undefined;
  if (filterFlushScheduled || !queuedFilters.size) return;
  if (!immediate) {
    filterFlushTimer = setTimeout(() => scheduleLibraryFilterFlush(true), 300);
    return;
  }
  filterFlushScheduled = true;
  queueMicrotask(() => {
    void enqueueSettings(async () => {
      const edits = queuedFilters;
      queuedFilters = new Map();
      if (!edits.size) return;
      const epoch = preferenceEpoch;
      const generation = privacyGeneration;
      settingsRevision++;
      try {
        const patch: Record<string, unknown> = Object.fromEntries(
          [...edits].map(([key, edit]) => [key, edit.value]),
        );
        for (const edit of edits.values()) Object.assign(patch, edit.extra);
        const saved = await persistPreferences([...edits.keys()], () =>
          request<Settings>("save_settings", {
            settings: patch,
            privacyGeneration: generation,
          }),
        );
        if (
          epoch === preferenceEpoch &&
          generation === privacyGeneration &&
          !awaitingPrivacySnapshot
        )
          confirmPreferences(saved);
        for (const edit of edits.values()) edit.handlers.onSaved();
      } catch (error) {
        for (const edit of edits.values()) edit.handlers.onError(error);
      } finally {
        settingsRevision++;
        for (const [key, edit] of edits)
          if (optimisticFilters.get(key)?.revision === edit.revision)
            optimisticFilters.delete(key);
        publishPreferences();
      }
    }).finally(() => {
      filterFlushScheduled = false;
      scheduleLibraryFilterFlush();
    });
  });
}

function rememberSnapshot<T>(command: string, result: T, revision: number): T {
  if (command !== "snapshot") return result;
  const snapshot = result as Snapshot;
  if (typeof snapshot?.privacyGeneration !== "number" || !snapshot.settings)
    return result;
  // Older requests may finish after a settings save or a newer privacy epoch.
  if (
    privacyGeneration !== null &&
    snapshot.privacyGeneration < privacyGeneration
  )
    return result;
  if (
    privacyPolicyGeneration !== null &&
    privacyPolicyGeneration !== snapshot.privacyPolicyGeneration
  ) {
    invalidatePreferences(true);
    readerPreferenceState = {
      ...readerPreferenceState,
      privateMode: snapshot.settings.privateMode,
      readOnly: snapshot.settings.readOnly,
    };
    awaitingPrivacySnapshot = privacyChangesPending > 0;
    publishPreferences();
  }
  privacyGeneration = snapshot.privacyGeneration;
  privacyPolicyGeneration = snapshot.privacyPolicyGeneration;
  if (revision !== settingsRevision || privacyChangesPending) return result;
  awaitingPrivacySnapshot = false;
  confirmPreferences(snapshot.settings);
  publishPreferences();
  return result;
}
const fragment = new URLSearchParams(location.hash.slice(1));
const suppliedToken = fragment.get("token");
const previewToken = suppliedToken;
if (suppliedToken) {
  history.replaceState(null, "", location.pathname + location.search);
}

// Stop invalidates deferred nearby admissions even after their producer unmounts.
// Native inspection/job generations additionally fence other root windows.
let localSwarmStopGeneration = 0;
function checkLocalSwarmAdmission(generation: number | undefined) {
  if (generation !== undefined && generation !== localSwarmStopGeneration)
    throw new Error(t("Download cancelled."));
}

export async function api<T = unknown>(
  command: string,
  args: Record<string, unknown> = {},
  admissionCurrent?: () => boolean,
): Promise<T> {
  if (command === "localswarm_stop" || (command === "nearby_stop" && !args.owner))
    localSwarmStopGeneration++;
  const nearbyAdmission = command === "nearby_connect" || command === "shared_library_browse" ||
    (command === "nearby_discover" && args.enabled === true);
  const localSwarmAdmission = command === "localswarm_download" || command === "shared_library_download" ||
    command === "localswarm_seed" || command === "localswarm_share_all" || nearbyAdmission ||
    (command === "resume_download" && typeof args.id === "string" && args.id.startsWith("localswarm:"))
      ? localSwarmStopGeneration : undefined;
  // Snapshot is the trusted root's startup request. Do not let an archive
  // iframe importing shell code initialize or replace root search authority.
  if (command === "snapshot" && window.parent === window)
    await searchDocumentSession();
  if (command === "save_settings") {
    const patch = args.settings as Partial<Settings> | undefined;
    const privacyChange =
      !!patch && ("privateMode" in patch || "readOnly" in patch);
    if (privacyChange) {
      flushSettingsDrafts();
      privacyChangesPending++;
      invalidatePreferences();
    } else if (patch) {
      // A later preferences-form edit supersedes undispatched Reader edits too.
      for (const key of appearanceKeys) {
        if (key in patch) {
          delete queuedPreferences[key];
          delete optimisticPreferences[key];
        }
      }
      for (const key of filterKeys) {
        if (key in patch) {
          queuedFilters.delete(key);
          optimisticFilters.delete(key);
        }
      }
      publishPreferences();
    }
    const epoch = preferenceEpoch;
    settingsRevision++;
    return enqueueSettings(async () => {
      const releases: ((committed: boolean) => void)[] = [];
      let committed = false;
      let operationError: unknown;
      try {
        if (privacyChange) {
          // The native cutover must follow old-generation iframe writes.
          const freezes = await Promise.allSettled(
            [...pendingPreferenceCutoverDrains].map((drain) =>
              Promise.resolve().then(drain),
            ),
          );
          const freezeErrors: unknown[] = [];
          for (const result of freezes) {
            if (result.status === "fulfilled") releases.push(result.value);
            else freezeErrors.push(result.reason);
          }
          if (freezeErrors.length)
            throw new AggregateError(
              freezeErrors,
              "Reader storage could not be frozen",
            );
          const errors = await collectPendingDrainErrors();
          if (errors.length)
            throw new AggregateError(
              errors,
              "Pending reader storage could not be saved",
            );
        }
        const previousPrivateMode = readerPreferenceState.privateMode;
        const previousReadOnly = readerPreferenceState.readOnly;
        const saved = await persistPreferences(Object.keys(patch ?? {}), () =>
          request<Settings>(command, args),
        );
        confirmedSettings = saved;
        // A successful no-op mode save does not retire the mounted iframe.
        committed =
          privacyChange &&
          (saved.privateMode !== previousPrivateMode ||
            saved.readOnly !== previousReadOnly);
        if (epoch === preferenceEpoch) confirmPreferences(saved);
        if (privacyChange) awaitingPrivacySnapshot = true;
        return saved as T;
      } catch (error) {
        operationError = error;
        throw error;
      } finally {
        const releaseErrors: unknown[] = [];
        for (const release of releases) {
          try {
            release(committed);
          } catch (error) {
            releaseErrors.push(error);
          }
        }
        if (privacyChange) privacyChangesPending--;
        settingsRevision++;
        publishPreferences();
        if (releaseErrors.length)
          throw new AggregateError(
            operationError === undefined
              ? releaseErrors
              : [operationError, ...releaseErrors],
            "Reader storage could not be resumed",
          );
      }
    });
  }
  if (nearbyAdmission || ["start_download", "resume_download", "edition_prepare", "edition_download", "edition_commit", "edition_rollback", "localswarm_download", "shared_library_download", "map_prepare", "map_start", "map_resume", "localswarm_seed", "localswarm_share_all", "sharing_start"].includes(command)) {
    // A tap immediately after changing network policy must observe its commit,
    // not race the serialized settings write with transfer admission.
    let pending: Promise<void>;
    do {
      pending = settingsQueue;
      await pending;
    } while (pending !== settingsQueue);
    checkLocalSwarmAdmission(localSwarmAdmission);
    for (const key of ["allowMeteredDownloads", "privateMode", "readOnly"])
      if (failedPreferenceWrites.has(key)) throw failedPreferenceWrites.get(key);
    if (localSwarmAdmission !== undefined && failedPreferenceWrites.has("directConnection"))
      throw failedPreferenceWrites.get("directConnection");
    if (["localswarm_seed", "localswarm_share_all", "sharing_start"].includes(command) && failedPreferenceWrites.has("sharingName"))
      throw failedPreferenceWrites.get("sharingName");
  }
  const revision = settingsRevision;
  return rememberSnapshot(command, await request<T>(command, args, localSwarmAdmission, admissionCurrent), revision);
}

/** Full edition transfers use the same settings/disk-space admission as Downloads. */
export async function editionDownload(decisionToken: string): Promise<DownloadTarget> {
  const privacy = capturePrivacyContext();
  if (!privacy || privacy.privateMode || privacy.readOnly)
    throw new Error(t("Leave private or read-only mode before downloading an edition."));
  const job = await api<DownloadJob>("edition_download", {
    decisionToken,
    privacyGeneration: privacy.generation,
  }, () => isPrivacyPolicyContextCurrent(privacy));
  return { kind: "download", id: job.id };
}

export interface DiskSpaceWarning {
  acknowledgement: string;
  directory: string;
  availableBytes: number;
  remainingBytes: number;
  projectedFreeBytes: number;
  thresholdBytes: number;
  recoveryWarning?: string;
}

export function parseDiskSpaceWarning(error: unknown): DiskSpaceWarning | null {
  const message = error instanceof Error ? error.message : error;
  const prefix = "LOW_DISK_SPACE:";
  if (typeof message !== "string" || !message.startsWith(prefix)) return null;
  try {
    const warning = JSON.parse(message.slice(prefix.length)) as DiskSpaceWarning;
    if (
      !warning ||
      typeof warning.acknowledgement !== "string" ||
      !warning.acknowledgement ||
      typeof warning.directory !== "string" ||
      !warning.directory ||
      !Number.isFinite(warning.availableBytes) ||
      warning.availableBytes < 0 ||
      !Number.isFinite(warning.remainingBytes) ||
      warning.remainingBytes < 0 ||
      !Number.isFinite(warning.projectedFreeBytes) ||
      !Number.isFinite(warning.thresholdBytes) ||
      warning.thresholdBytes <= 0
    )
      return null;
    return warning;
  } catch {
    return null;
  }
}

/**
 * `declined` is only the user's own answer to the prompt. A prompt that could
 * not be shown, or that the lifecycle withdrew, is `unavailable`: the caller
 * must not treat that as a choice the user made.
 */
export type DiskSpaceDecision = "accepted" | "declined" | "unavailable";
type DiskSpaceConfirmationHandler = (warning: DiskSpaceWarning) => Promise<DiskSpaceDecision>;
let diskSpaceConfirmation: DiskSpaceConfirmationHandler | null = null;

/** Only the trusted root mounts the app-owned confirmation surface. */
export function registerDiskSpaceConfirmation(handler: DiskSpaceConfirmationHandler): () => void {
  if (diskSpaceConfirmation) throw new Error("Disk space confirmation is already registered");
  diskSpaceConfirmation = handler;
  return () => { if (diskSpaceConfirmation === handler) diskSpaceConfirmation = null; };
}

async function decideDiskSpaceWarning(warning: DiskSpaceWarning): Promise<DiskSpaceDecision> {
  const context = capturePrivacyContext();
  if (!diskSpaceConfirmation || !context || context.privateMode || context.readOnly) return "unavailable";
  const decision = await diskSpaceConfirmation(warning);
  // Consent given under an earlier privacy generation authorizes nothing.
  return decision === "accepted" && !isPrivacyContextCurrent(context) ? "unavailable" : decision;
}

export async function confirmDiskSpaceWarning(warning: DiskSpaceWarning): Promise<boolean> {
  return (await decideDiskSpaceWarning(warning)) === "accepted";
}

async function request<T>(
  command: string,
  args: Record<string, unknown>,
  localSwarmAdmission?: number,
  admissionCurrent?: () => boolean,
): Promise<T> {
  const checkAdmission = () => {
    checkLocalSwarmAdmission(localSwarmAdmission);
    if (admissionCurrent && !admissionCurrent()) throw new Error(t("Download cancelled."));
  };
  checkAdmission();
  const stillCurrent = () =>
    (localSwarmAdmission === undefined || localSwarmAdmission === localSwarmStopGeneration) &&
    (!admissionCurrent || admissionCurrent());
  if (
    !["start_download", "resume_download", "edition_download", "localswarm_download", "shared_library_download", "map_start", "map_resume"].includes(
      command,
    )
  )
    return await requestOnce<T>(command, args, stillCurrent);
  const context = capturePrivacyContext();
  // The acknowledgement is scoped to this immutable request, never cached or
  // shared with another destination, window, or browsing generation.
  const original = structuredClone(args);
  if (context && !("privacyGeneration" in original))
    original.privacyGeneration = context.generation;
  try {
    return await requestOnce<T>(command, original, stillCurrent);
  } catch (error) {
    checkAdmission();
    const warning = parseDiskSpaceWarning(error);
    if (
      !warning ||
      !context ||
      context.privateMode ||
      context.readOnly ||
      !isPrivacyContextCurrent(context)
    )
      throw error;
    const decision = await decideDiskSpaceWarning(warning);
    checkAdmission();
    // Declining the prompt is the user's own choice, not a failed download:
    // callers neither paint nor record it. Every other refusal stays an error.
    if (decision === "declined")
      throw new UserCancelledError(t("Download cancelled."));
    if (decision !== "accepted" || !isPrivacyContextCurrent(context))
      throw new Error(t("Download cancelled."));
    // Exactly one retry, only after explicit consent. Any second warning or
    // persistence/network/consent error propagates to the caller unchanged.
    return await requestOnce<T>(command, {
      ...original,
      diskSpaceAcknowledgement: warning.acknowledgement,
    }, stillCurrent);
  }
}

async function requestOnce<T>(
  command: string,
  args: Record<string, unknown>,
  admissionCurrent?: () => boolean,
): Promise<T> {
  if (admissionCurrent && !admissionCurrent()) throw new Error(t("Download cancelled."));
  if (command === "open_article" || command === "random_article")
    args = { ...args, parentOrigin: location.origin };
  if (mobileBackgroundCheckpoint && checkpointDurabilityCommands[command] === true &&
      !("mobileBackgroundRequestId" in args))
    args = { ...args, mobileBackgroundRequestId: mobileBackgroundCheckpoint.id };
  if (
    [
      "search",
      "suggest",
      "catalog",
      "catalog_facets",
      "catalog_torrent",
      "find_updates",
      "edition_prepare",
      "edition_download",
      "edition_status",
      "edition_commit",
      "edition_cancel",
      "edition_rollback",
      "mobile_speech_voices",
      "mobile_speech_prepare",
      "mobile_apply_locale",
      "mobile_speech_start",
      "check_integrity",
      "diagnostics_integrity",
      "integrity_status",
      "cancel_integrity",
      "save_settings",
      "save_session",
      "record_visit",
      "record_search",
      "remove_search",
      "remove_book",
      "set_book_zoom",
      "set_book_pinned",
      "set_speech_preference",
      "spelling_status",
      "spelling_build",
      "spelling_cancel",
      "open_attachment",
      "mobile_save_resource",
      "mobile_save_mhtml",
      "mobile_save_pdf",
      "print_pdf",
      "nearby_status",
      "localswarm_discovery",
      "nearby_connect",
      "nearby_discover",
    ].includes(command) &&
    privacyGeneration !== null &&
    !("privacyGeneration" in args)
  )
    args = { ...args, privacyGeneration };
  // A policy change while the app was suspended retires the request; do not
  // replay it against the new generation.
  const generation = privacyGeneration;
  return retrySuspended(() => dispatchOnce<T>(command, args), foregroundGate, {
    // Pairing is explicit foreground intent, never a deferred radio start.
    // Cancellation is sent once as cancellation, not replayed after navigation.
    enabled: isMobile && !command.startsWith("nearby_") && !command.startsWith("private_sync_") &&
      command !== "localswarm_stop" && command !== "localswarm_stop_seed" &&
      !("mobileBackgroundRequestId" in args) &&
      command !== "mobile_validate_background" && command !== "mobile_background_ack",
    stillRelevant: () => privacyGeneration === generation && (!admissionCurrent || admissionCurrent()),
    // Stable receipt for the device log: the command waits for the foreground
    // and is sent exactly once more when the native lifecycle resumes.
    onSuspend: () => { if (!command.startsWith("nearby_")) console.info(`KIWIX_SUSPENDED_RETRY ${command}`); },
  });
}

async function dispatchOnce<T>(
  command: string,
  args: Record<string, unknown>,
): Promise<T> {
  try {
    if (isNative) {
      // Keep both source identities independent of current native routing.
      return await invoke<T>("dispatch", {
        command,
        args,
        ...androidDocumentSource,
      });
    }
    const token = previewToken;
    if (!token)
      throw new Error(
        t(
          "Open the authenticated preview URL printed by the Rust application.",
        ),
      );
    const response = await fetch("/api", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ command, args }),
      credentials: "omit",
      cache: "no-store",
    });
    const body = (await response.json()) as { result?: T; error?: string };
    if (!response.ok || body.error)
      throw new Error(body.error || `HTTP ${response.status}`);
    return body.result as T;
  } catch (raw) {
    const message = raw instanceof Error ? raw.message : String(raw);
    // Translate the lifecycle gate once, at the boundary; no caller matches
    // the sentence or has to know how the Rust side encodes it.
    const suspended = parseSuspendedError(message);
    if (suspended) throw suspended;
    const provider = parseProviderDocumentError(message);
    if (provider) throw provider;
    throw raw instanceof Error ? raw : new Error(message);
  }
}

type DesktopDialogKind = "import" | "save";

async function desktopDialogPath(
  kind: DesktopDialogKind,
  context: PrivacyContext,
  suggestedName?: string,
): Promise<string | null> {
  const path = await api<string>("desktop_dialog_directory", {
    kind,
    suggestedName,
    privacyGeneration: context.generation,
  });
  return isPrivacyContextCurrent(context) ? path : null;
}

async function rememberDesktopDialog(
  kind: DesktopDialogKind,
  path: string,
  context: PrivacyContext,
): Promise<void> {
  if (
    context.privateMode ||
    context.readOnly ||
    !isPrivacyContextCurrent(context)
  )
    return;
  try {
    await api("remember_desktop_dialog_directory", {
      kind,
      path,
      privacyGeneration: context.generation,
    });
  } catch (error) {
    // Remembering a preference must not turn a valid selection or completed
    // export into a failure. Surface the persistence error independently.
    if (isPrivacyContextCurrent(context))
      window.dispatchEvent(
        new CustomEvent("kiwix-desktop-workflow-error", {
          detail: { error, severity: "recoverable" },
        }),
      );
  }
}

async function desktopSave(
  suggestedName: string,
  write: (destination: string, context: PrivacyContext) => Promise<unknown>,
): Promise<void> {
  const context = capturePrivacyContext();
  if (!context) return;
  const defaultPath = await desktopDialogPath("save", context, suggestedName);
  if (!defaultPath) return;
  const destination = await save({ defaultPath });
  if (!destination || !isPrivacyContextCurrent(context)) return;
  await write(destination, context);
  await rememberDesktopDialog("save", destination, context);
}

export async function chooseFiles(): Promise<string[]> {
  if (isMobile) throw new Error(t("Use Add files to read originals, or explicitly import a managed copy."));
  if (!isNative) {
    const value = window.prompt(t("ZIM file paths on the host (one per line)"));
    return value
      ? value
          .split("\n")
          .map((p) => p.trim())
          .filter(Boolean)
      : [];
  }
  const context = capturePrivacyContext();
  if (!context) return [];
  const defaultPath = await desktopDialogPath("import", context);
  if (!defaultPath) return [];
  const paths = await open({
    defaultPath,
    multiple: true,
    directory: false,
    filters: [{ name: "ZIM", extensions: ["zim", "zimaa"] }],
  });
  if (!paths || !isPrivacyContextCurrent(context)) return [];
  const selected = Array.isArray(paths) ? paths : [paths];
  if (selected[0]) await rememberDesktopDialog("import", selected[0], context);
  if (!isPrivacyContextCurrent(context)) return [];
  await requestFolderAccess(selected);
  return isPrivacyContextCurrent(context) ? selected : [];
}

/**
 * The macOS App Sandbox grants a chosen file but not its folder, and Kiwix
 * reads archives through their folder (split parts, change detection, delete).
 * Ask once for each folder the app cannot open yet; the grant is remembered
 * across launches. Does nothing outside the sandbox.
 */
export async function requestFolderAccess(paths: string[]): Promise<void> {
  if (!isNative || isMobile || !paths.length) return;
  const folders = await api<string[]>("sandbox_folders_needing_access", { paths });
  for (const folder of folders) {
    await open({
      directory: true,
      multiple: false,
      defaultPath: folder,
      title: t("Allow Kiwix to read the folder that contains the archive"),
    });
  }
}

export async function chooseDirectory(): Promise<string | null> {
  if (isMobile) throw new Error(t("Choose an app storage location in Settings, or use Add folder to read originals."));
  if (!isNative) return window.prompt(t("Directory path on the host"));
  const path = await open({ directory: true, multiple: false });
  return Array.isArray(path) ? (path[0] ?? null) : path;
}

function browserSave(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export async function saveText(
  suggestedName: string,
  text: string,
  context: PrivacyContext,
): Promise<void> {
  if (context.readOnly || !isPrivacyContextCurrent(context)) return;
  if (isMobile) {
    await api("mobile_save_text", { suggestedName, text, privacyGeneration: context.generation });
    return;
  }
  if (!isNative) {
    browserSave(
      suggestedName,
      new Blob([text], { type: suggestedName.endsWith(".xml")
        ? "application/xml;charset=utf-8" : "application/json;charset=utf-8" }),
    );
    return;
  }
  await desktopSave(suggestedName, (path, context) =>
    api("save_text", { path, text, privacyGeneration: context.generation }),
  );
}

export async function saveBytes(
  suggestedName: string,
  bytes: string,
  mime: string,
  context: PrivacyContext,
): Promise<void> {
  if (context.readOnly || !isPrivacyContextCurrent(context)) return;
  if (isMobile) {
    await api("mobile_save_bytes", { suggestedName, bytes, mime, privacyGeneration: context.generation });
    return;
  }
  if (!isNative) {
    const raw = atob(bytes);
    const buffer = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) buffer[i] = raw.charCodeAt(i);
    browserSave(suggestedName, new Blob([buffer], { type: mime }));
    return;
  }
  await desktopSave(suggestedName, (path, context) =>
    api("save_bytes", { path, bytes, privacyGeneration: context.generation }),
  );
}

export async function readTextFile(context: PrivacyContext): Promise<string | null> {
  if (!isPrivacyContextCurrent(context)) return null;
  if (isMobile) {
    const text = await api<string | null>("mobile_read_text", { privacyGeneration: context.generation });
    if (text === null || !isPrivacyPolicyContextCurrent(context)) return null;
    // The picker can finish before the root has observed foreground reader
    // retirement. Reconcile the native generation before dispatching an import.
    const before = privacyGeneration;
    try {
      await api("snapshot");
    } catch (error) {
      // An overlapping root refresh can supersede our suspended snapshot.
      if (!isSuspendedError(error) || privacyGeneration === before) throw error;
    }
    return isPrivacyPolicyContextCurrent(context) ? text : null;
  }
  if (!isNative) {
    const text = await new Promise<string | null>((resolve, reject) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".json,.xml,.opds";
      input.onchange = () => {
        const file = input.files?.[0];
        if (file && file.size > 16 * 1024 * 1024)
          reject(new Error("Import exceeds the 16 MiB limit. Split the export into smaller files."));
        else if (file) file.text().then(resolve, reject);
        else resolve(null);
      };
      input.oncancel = () => resolve(null);
      input.click();
    });
    return isPrivacyContextCurrent(context) ? text : null;
  }
  const defaultPath = await desktopDialogPath("import", context);
  if (!defaultPath) return null;
  const selected = await open({
    defaultPath,
    multiple: false,
    filters: [
      { name: "JSON / XML / OPDS", extensions: ["json", "xml", "opds"] },
    ],
  });
  if (!selected || !isPrivacyContextCurrent(context)) return null;
  const path = Array.isArray(selected) ? selected[0] : selected;
  const text = await api<string>("read_text", {
    path,
    privacyGeneration: context.generation,
  });
  await rememberDesktopDialog("import", path, context);
  return isPrivacyContextCurrent(context) ? text : null;
}

export async function saveResource(
  bookId: string,
  installationId: string,
  path: string,
  suggestedName: string,
): Promise<void> {
  const context = capturePrivacyContext();
  if (!context || context.readOnly) return;
  if (isMobile) {
    await api("mobile_save_resource", { bookId, installationId, path, suggestedName, privacyGeneration: context.generation });
    return;
  }
  if (!isNative) {
    const resource = await api<{
      fileName: string;
      bytes: string;
      mime: string;
    }>("export_article", { bookId, installationId, path, privacyGeneration: context.generation });
    await saveBytes(resource.fileName, resource.bytes, resource.mime, context);
    return;
  }
  await desktopSave(suggestedName, (destination, context) =>
    api("save_resource", {
      bookId, installationId,
      path,
      destination,
      overwrite: true,
      privacyGeneration: context.generation,
    }),
  );
}

export async function saveArticlePdf(
  bookId: string,
  installationId: string,
  path: string,
  suggestedName: string,
  liveSnapshot: LiveArticleSnapshot,
): Promise<void> {
  if (!isNative)
    throw new Error(t("Article PDF export requires the native application."));
  const context = capturePrivacyContext();
  if (!context) return;
  if (isMobile) {
    await api("mobile_save_pdf", {
      bookId, installationId, path, suggestedName, liveSnapshot, privacyGeneration: context.generation,
    });
    return;
  }
  const defaultPath = await desktopDialogPath("save", context, suggestedName);
  if (!defaultPath || !isPrivacyContextCurrent(context)) return;
  const destination = await save({
    defaultPath,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!destination || !isPrivacyContextCurrent(context)) return;
  await api("save_pdf", {
    bookId, installationId, path, destination, suggestedName, liveSnapshot, overwrite: true,
    privacyGeneration: context.generation,
  });
  await rememberDesktopDialog("save", destination, context);
}

/** Share the current article through the native PDF renderer and share sheet.
 * Uses the same admission and privacy generation as saving the PDF. */
export async function shareArticlePdf(
  bookId: string,
  installationId: string,
  path: string,
  suggestedName: string,
  liveSnapshot: LiveArticleSnapshot,
): Promise<void> {
  // The share action is only rendered where the renderer returns a PDF.
  if (!canShareArticlePdf) return;
  const context = capturePrivacyContext();
  if (!context) return;
  await api("share_pdf", {
    bookId,
    installationId,
    path,
    suggestedName,
    liveSnapshot,
    privacyGeneration: context.generation,
  });
}

/** Print the captured live article through the native desktop print panel.
 * Reuses the same admission, privacy generation and one-use commit authority
 * as saving; the private temp PDF is removed when the panel closes. */
export async function printArticlePdf(
  bookId: string,
  installationId: string,
  path: string,
  suggestedName: string,
  liveSnapshot: LiveArticleSnapshot,
): Promise<void> {
  if (!canPrintArticlePdf)
    throw new Error(t("Article PDF export requires the native application."));
  const context = capturePrivacyContext();
  if (!context) return;
  await api("print_pdf", {
    bookId,
    installationId,
    path,
    suggestedName,
    liveSnapshot,
    privacyGeneration: context.generation,
  });
}

/** Toggle the session-scoped restricted reader policy for this window/book. */
export async function setReaderRestricted(
  bookId: string,
  restricted: boolean,
): Promise<boolean> {
  const result = await api<{ restricted: boolean }>("set_reader_restricted", {
    bookId,
    restricted,
  });
  return result.restricted === true;
}

/** Share a library archive file through the native share sheet. */
export async function shareArchive(bookId: string, installationId: string): Promise<void> {
  if (!canShareSystem) return;
  const context = capturePrivacyContext();
  if (!context) return;
  await api("share_book", {
    bookId,
    installationId,
    privacyGeneration: context.generation,
  });
}

export async function saveMhtml(
  bookId: string,
  installationId: string,
  path: string,
  suggestedName: string,
): Promise<void> {
  const context = capturePrivacyContext();
  if (!context || context.readOnly) return;
  if (isMobile) {
    await api("mobile_save_mhtml", { bookId, installationId, path, suggestedName, privacyGeneration: context.generation });
    return;
  }
  if (isNative) {
    await desktopSave(suggestedName, (destination, context) =>
      api("save_mhtml", {
        bookId, installationId,
        path,
        destination,
        overwrite: true,
        privacyGeneration: context.generation,
      }),
    );
    return;
  }
  const destination = window.prompt(
    t("Export destination path on the host"),
    suggestedName,
  );
  if (destination && isPrivacyContextCurrent(context))
    await api("save_mhtml", {
      bookId, installationId,
      path,
      destination,
      overwrite: isNative,
      privacyGeneration: context.generation,
    });
}
