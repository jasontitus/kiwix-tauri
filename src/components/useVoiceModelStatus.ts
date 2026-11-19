import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, isNative } from "../api";

interface KokoroStatus {
  available: boolean;
  downloading: boolean;
  downloaded: number;
  total: number;
  voices: unknown[];
  error?: string | null;
  performance?: "unknown" | "realtime" | "slow";
  probing?: boolean;
  performanceRatio?: number | null;
  performanceError?: string | null;
  bundled?: boolean;
  bundleCheckFailed?: boolean;
  removalPending?: boolean;
  repairAvailable?: boolean;
  legacyRetained?: boolean;
  legacyAudioRetained?: boolean;
}

export type RetainedVoiceFiles = Pick<KokoroStatus, "legacyRetained" | "legacyAudioRetained">;

/** Native events invalidate the view. Only the service can confirm the current installation. */
export function useVoiceModelStatus(enabled: boolean) {
  const [status, setStatus] = useState<KokoroStatus | null>(null);
  const [readError, setReadError] = useState("");
  const sequence = useRef(0);
  const pending = useRef(false);
  const active = useRef(false);

  const refresh = useCallback(async (preserved?: RetainedVoiceFiles) => {
    if (!active.current) return;
    const request = ++sequence.current;
    pending.current = true;
    try {
      const next = await api<KokoroStatus>("kokoro_status");
      if (request !== sequence.current) return;
      setStatus({
        ...next,
        legacyRetained: next.legacyRetained ?? preserved?.legacyRetained,
        legacyAudioRetained: next.legacyAudioRetained ?? preserved?.legacyAudioRetained,
      });
      setReadError("");
    } catch (value) {
      if (request === sequence.current) setReadError(value instanceof Error ? value.message : String(value));
    } finally {
      if (request === sequence.current) pending.current = false;
    }
  }, []);

  useEffect(() => {
    if (!isNative || !enabled) {
      setStatus(null);
      setReadError("");
      return;
    }
    active.current = true;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    let progressTimer: number | undefined;
    void listen<{ state: string }>("kiwix-speech-model", ({ payload }) => {
      if (disposed) return;
      if (payload.state === "downloading") {
        if (progressTimer === undefined && !pending.current) {
          progressTimer = window.setTimeout(() => {
            progressTimer = undefined;
            if (!pending.current) void refresh();
          }, 350);
        }
      } else {
        window.clearTimeout(progressTimer);
        progressTimer = undefined;
        void refresh();
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else {
        unsubscribe = unlisten;
        void refresh();
      }
    }).catch(() => { /* Manual refresh and the initial status remain available. */ });
    void refresh();
    return () => {
      disposed = true;
      active.current = false;
      ++sequence.current;
      pending.current = false;
      window.clearTimeout(progressTimer);
      unsubscribe?.();
    };
  }, [enabled, refresh]);

  return { status: enabled ? status : null, readError: enabled ? readError : "", refresh };
}
