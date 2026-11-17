import { useCallback, useEffect, useRef, useState } from "react";
import { api, capturePrivacyContext, isNative, isPrivacyContextCurrent } from "./api";
import type { PrivacyContext } from "./api";
import { t } from "./i18n";
import type { InstalledVoice } from "./speech-preferences";

export interface NativeSpeechContext {
  sessionId: string;
  archiveId: string;
  origin: string;
  url: string;
  privacyGeneration: number;
}
export interface NativeSpeechPreferences { lang: string; voice: string; rate: number; title?: string }
interface Request {
  id: string;
  token?: string;
  context: NativeSpeechContext;
  privacy: PrivacyContext;
  preferences: NativeSpeechPreferences;
  text: string | null;
  base: number;
  offset: number;
  starting: boolean;
  started: boolean;
  timer?: number;
}
interface Options {
  context: () => NativeSpeechContext | null;
  identity: () => NativeSpeechContext | null;
  contextKey: string;
  privacyGeneration: number;
  interactive: boolean;
  sourceReady: boolean;
  /** Caller-side identity revision: reported, never a discovery trigger. */
  interactionRevision: number;
  extract: (requestId: string) => void;
  onVoices: (voices: InstalledVoice[], available: boolean, failure: unknown | null) => void;
  onSpeaking: (speaking: boolean) => void;
  /** Absolute UTF-16 offset in the full spoken text, or null when no paint should remain. */
  onProgress: (offset: number | null) => void;
  onError: (error: unknown) => void;
}

/**
 * Only root-owned button requests may turn archive text into native speech.
 * Mobile routes every voice here; desktop routes only Kokoro neural voices
 * (its system voices stay in the archive frame's speechSynthesis), and its
 * backend lists only the neural voices.
 */
export function useNativeSpeech(options: Options) {
  const latest = useRef(options);
  latest.current = options;
  const current = useRef<Request | null>(null);
  const desiredPreferences = useRef<NativeSpeechPreferences | null>(null);
  const serial = useRef(0);
  const alive = useRef(false);
  const ready = useRef(false);
  const listenerReady = useRef<Promise<void> | null>(null);
  const [pending, setPending] = useState(false);
  const [paused, setPaused] = useState(false);
  const report = useCallback((error: unknown) => {
    if (alive.current) latest.current.onError(error);
  }, []);
  const sourceCurrent = useCallback((request: Pick<Request, "context" | "privacy">) => {
    const context = latest.current.identity();
    return alive.current && isPrivacyContextCurrent(request.privacy) &&
      context?.sessionId === request.context.sessionId && context.url === request.context.url &&
      context.archiveId === request.context.archiveId && context.origin === request.context.origin &&
      context.privacyGeneration === request.context.privacyGeneration;
  }, []);
  const valid = useCallback((request: Request) =>
    current.current === request && sourceCurrent(request), [sourceCurrent]);
  const retire = useCallback(async (request: Request, stop: boolean): Promise<{ offset: number; stopped: boolean }> => {
    window.clearTimeout(request.timer);
    if (!request.token) return { offset: request.offset, stopped: false };
    const args = { id: request.id, token: request.token };
    let offset = request.offset;
    let stopped = false;
    try {
      if (stop) {
        const result = await api<{ offset: number; stopped: boolean }>("mobile_speech_stop", args);
        stopped = result.stopped;
        if (Number.isSafeInteger(result.offset) && result.offset >= 0) offset = Math.max(offset, result.offset);
      }
    } finally {
      await api("mobile_speech_retire", args);
    }
    return { offset, stopped };
  }, []);
  const stop = useCallback(async () => {
    serial.current++;
    const request = current.current;
    current.current = null;
    if (alive.current) {
      setPending(false);
      setPaused(false);
      latest.current.onProgress(null);
    }
    if (request) {
      // Stopping is user intent and the local state is already cleared. A
      // concurrent retirement can reject the redundant stop/retire pair; that
      // must not surface as a read-aloud error banner.
      try { await retire(request, true); } catch { /* already retired */ }
    }
    if (alive.current && !current.current) latest.current.onSpeaking(false);
  }, [report, retire]);
  const startText = useCallback(async (request: Request, text: string) => {
    if (!valid(request) || !request.token || request.starting || !latest.current.context()) return;
    window.clearTimeout(request.timer);
    request.starting = true;
    request.text = text;
    try {
      await api("mobile_speech_start", {
        id: request.id, token: request.token,
        sessionId: request.context.sessionId, archiveId: request.context.archiveId,
        origin: request.context.origin, ...request.preferences,
        privacyGeneration: request.context.privacyGeneration,
        text: text.slice(request.base),
      });
      if (valid(request) && current.current === request && !request.started) {
        request.timer = window.setTimeout(() => {
          if (current.current !== request || request.started) return;
          report(new Error(t("Read-aloud did not start. Check the selected voice and device volume, or download the natural voice model.")));
          void stop();
        }, 45_000);
      }
      // Native callbacks, never the admission acknowledgement, report playback.
      if (!valid(request)) await retire(request, true);
    } catch (error) {
      if (current.current === request) {
        current.current = null;
        if (alive.current) {
          setPending(false);
          latest.current.onSpeaking(false);
          latest.current.onProgress(null);
          report(error);
        }
      }
      try { await retire(request, true); } catch (failure) { report(failure); }
    }
  }, [report, retire, valid]);
  const begin = useCallback(async (preferences: NativeSpeechPreferences, selected: string | null, base = 0,
    source?: Pick<Request, "context" | "privacy">) => {
    if (!isNative || !ready.current) return;
    desiredPreferences.current = preferences;
    const revision = ++serial.current;
    const previous = current.current;
    current.current = null;
    if (alive.current) latest.current.onProgress(null);
    if (previous) {
      try { await retire(previous, true); } catch (error) { report(error); return; }
    }
    if (!alive.current || revision !== serial.current) return;
    if (source && !sourceCurrent(source)) return;
    const context = source?.context ?? latest.current.context();
    const privacy = source?.privacy ?? capturePrivacyContext();
    if (!context || !privacy || !isPrivacyContextCurrent(privacy)) return;
    const request: Request = {
      id: crypto.randomUUID(), context, privacy, preferences,
      text: selected, base, offset: 0, starting: false, started: false,
    };
    current.current = request;
    setPending(true);
    setPaused(false);
    latest.current.onSpeaking(false);
    try {
      const prepared = await api<{ id: string; token: string }>("mobile_speech_prepare", {
        id: request.id, sessionId: context.sessionId, archiveId: context.archiveId, origin: context.origin,
        privacyGeneration: context.privacyGeneration,
      });
      request.token = prepared.token;
      if (!valid(request) || revision !== serial.current) {
        await retire(request, true);
        return;
      }
      if (selected !== null) await startText(request, selected);
      else {
        request.timer = window.setTimeout(() => {
          if (current.current !== request) return;
          report(new Error(t("The article did not provide text for read-aloud.")));
          void stop();
        }, 8000);
        latest.current.extract(request.id);
      }
    } catch (error) {
      if (current.current === request) {
        current.current = null;
        if (alive.current) {
          setPending(false);
          latest.current.onSpeaking(false);
          latest.current.onProgress(null);
          report(error);
        }
      }
      try { await retire(request, true); } catch (failure) { report(failure); }
    }
  }, [report, retire, sourceCurrent, startText, stop, valid]);
  const acceptText = useCallback((id: string, text: unknown, error: unknown) => {
    const request = current.current;
    if (!request || request.id !== id || !valid(request) || request.starting) return;
    if (!latest.current.context()) { void stop(); return; }
    if (typeof text !== "string" || !text.trim() || text.length > 1_000_000 || error) {
      report(new Error(error === "SPEECH_TEXT_LIMIT" || (typeof text === "string" && text.length > 1_000_000)
        ? t("This article is too large for read-aloud.")
        : error ? t("The article did not provide text for read-aloud.") : t("This article has no readable text.")));
      void stop();
      return;
    }
    void startText(request, text);
  }, [report, startText, stop, valid]);
  const transport = useCallback(async (command: "mobile_speech_pause" | "mobile_speech_resume" | "mobile_speech_previous" |
    "mobile_speech_next" | "mobile_speech_rewind" | "mobile_speech_forward" | "mobile_speech_seek",
    position: { seconds?: number; offset?: number } = {}) => {
    const request = current.current;
    if (!request?.token || !valid(request)) return;
    // Pause, previous/next and seek may not emit a new native state before the
    // next boundary; clear the old sentence immediately so paint cannot imply
    // that stale text is still being spoken.
    if (command !== "mobile_speech_resume") latest.current.onProgress(null);
    try {
      const result = await api<{ paused?: boolean; offset?: number }>(command, {
        id: request.id, token: request.token,
        ...position,
        sessionId: request.context.sessionId, archiveId: request.context.archiveId,
        origin: request.context.origin,
        privacyGeneration: request.context.privacyGeneration,
      });
      if (!alive.current || current.current !== request) return;
      if (Number.isSafeInteger(result.offset) && (result.offset ?? -1) >= 0)
        request.offset = result.offset!;
      if (command === "mobile_speech_pause") setPaused(true);
      if (command === "mobile_speech_resume" && result.paused === false) setPaused(false);
      // Resume/seek acknowledgements are the only immediate position proof when
      // the engine emits its next range callback later (or not at all).
      if (command !== "mobile_speech_pause" && result.paused !== true &&
          Number.isSafeInteger(result.offset) && (result.offset ?? -1) >= 0)
        latest.current.onProgress(request.base + result.offset!);
    } catch (error) { report(error); }
  }, [report, valid]);
  const pause = useCallback(() => transport("mobile_speech_pause"), [transport]);
  const resume = useCallback(() => transport("mobile_speech_resume"), [transport]);
  const previous = useCallback(() => transport("mobile_speech_previous"), [transport]);
  const next = useCallback(() => transport("mobile_speech_next"), [transport]);
  const rewind = useCallback((seconds = 15) => transport("mobile_speech_rewind", { seconds }), [transport]);
  const forward = useCallback((seconds = 15) => transport("mobile_speech_forward", { seconds }), [transport]);
  const seek = useCallback((offset: number) => transport("mobile_speech_seek", { offset }), [transport]);
  const preferences = useCallback(async (next: NativeSpeechPreferences) => {
    desiredPreferences.current = next;
    const request = current.current;
    if (!request || !valid(request)) return;
    request.preferences = next;
    if (!request.starting || request.text === null) return;
    const revision = ++serial.current;
    current.current = null;
    if (alive.current) latest.current.onProgress(null);
    try {
      const stopped = await retire(request, true);
      if (!alive.current || revision !== serial.current) return;
      if (!sourceCurrent(request) || (request.started && !stopped.stopped)) {
        setPending(false);
        latest.current.onSpeaking(false);
        latest.current.onProgress(null);
        return;
      }
      const base = Math.min(request.text.length, request.base + stopped.offset);
      if (base < request.text.length) await begin(desiredPreferences.current ?? next, request.text, base, request);
      else { setPending(false); latest.current.onSpeaking(false); latest.current.onProgress(null); }
    } catch (error) {
      if (alive.current && revision === serial.current) {
        setPending(false);
        latest.current.onSpeaking(false);
        latest.current.onProgress(null);
        report(error);
      }
    }
  }, [begin, report, retire, sourceCurrent, valid]);
  useEffect(() => {
    // The headless browser preview has no Tauri event bridge or native engine.
    if (!isNative) return;
    alive.current = true;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const listening = (async () => {
      const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      if (cancelled) return;
      const remove = await getCurrentWebviewWindow().listen<{
        id: string; speaking: boolean; offset: number; paused?: boolean; error?: string;
      }>("mobile-speech-state", ({ payload }) => {
        if (cancelled) return;
        const request = current.current;
        if (!request || payload.id !== request.id) return;
        const offset = Number.isSafeInteger(payload.offset) && payload.offset >= 0 ? payload.offset : null;
        if (!valid(request)) {
          // The source identity can be momentarily unavailable while the
          // reader frame settles or the app returns to the foreground. Progress
          // is paint-only and privacy-gated; do not freeze the read-along
          // highlight on the first sentence for that transient.
          if (isPrivacyContextCurrent(request.privacy)) {
            if (payload.speaking === true) {
              if (offset !== null) latest.current.onProgress(request.base + offset);
            } else if (payload.paused !== true) {
              latest.current.onProgress(null);
            }
          }
          // A terminal event ends the session in Rust whatever the reader's
          // state (another window's start, model deletion, retirement), so
          // the controls must not stay on "Pause reading" for a dead session.
          if (payload.speaking === false && payload.paused !== true) {
            window.clearTimeout(request.timer);
            current.current = null;
            setPending(false);
            setPaused(false);
            latest.current.onSpeaking(false);
          }
          return;
        }
        if (offset !== null) request.offset = offset;
        if (typeof payload.speaking !== "boolean") return;
        window.clearTimeout(request.timer);
        const paused = !payload.speaking && payload.paused === true;
        if (payload.speaking) request.started = true;
        setPending(false);
        setPaused(paused);
        latest.current.onSpeaking(payload.speaking);
        if (payload.speaking) {
          if (offset !== null)
            latest.current.onProgress(request.base + offset);
        } else {
          latest.current.onProgress(null);
          if (!paused) {
            current.current = null;
            if (payload.error) report(new Error(payload.error));
            void retire(request, false).catch(report);
          }
        }
      });
      if (cancelled) { remove(); return; }
      unlisten = remove;
    })();
    listenerReady.current = listening;
    void listening.catch((error) => { if (!cancelled) report(error); });
    return () => {
      cancelled = true;
      ready.current = false;
      alive.current = false;
      unlisten?.();
      if (listenerReady.current === listening) listenerReady.current = null;
      void stop();
    };
  }, [report, retire, stop, valid]);
  // Voice discovery is keyed on privacy/source, never on interactionRevision:
  // every foreground reconcile bumps that revision but cannot change voices.
  useEffect(() => {
    if (!isNative || !options.interactive || !options.sourceReady) { ready.current = false; return; }
    const listening = listenerReady.current;
    const context = options.context();
    const privacy = capturePrivacyContext();
    if (!listening || !context || !privacy ||
        context.privacyGeneration !== options.privacyGeneration ||
        privacy.generation !== context.privacyGeneration) {
      ready.current = false;
      return;
    }
    const admitted = { context, privacy };
    let cancelled = false;
    const currentDiscovery = () => !cancelled && latest.current.interactive &&
      latest.current.sourceReady && listenerReady.current === listening &&
      latest.current.privacyGeneration === context.privacyGeneration && sourceCurrent(admitted);
    void (async () => {
      await listening;
      if (!currentDiscovery()) return;
      const voices = await api<{ available: boolean; voices: InstalledVoice[]; error?: string }>(
        "mobile_speech_voices", { privacyGeneration: context.privacyGeneration });
      if (!currentDiscovery()) return;
      ready.current = voices.available;
      latest.current.onVoices(voices.voices, voices.available, voices.error ? new Error(voices.error) : null);
    })().catch((error) => {
      if (currentDiscovery()) {
        ready.current = false;
        latest.current.onVoices([], false, error);
      }
    });
    return () => { cancelled = true; };
  }, [options.privacyGeneration, options.interactive, options.sourceReady,
      options.contextKey, report, sourceCurrent]);
  useEffect(() => {
    if (isNative) void stop();
  }, [options.contextKey, stop]);
  return { pending, paused, begin, stop, pause, resume, previous, next, rewind, forward, seek, acceptText, preferences };
}
